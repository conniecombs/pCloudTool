//! # pCloud Rust Client
//!
//! A high-performance, async Rust client for the pCloud API with support for
//! parallel file transfers, recursive folder sync, and duplicate detection.

use futures::stream::{self, StreamExt};
use reqwest::{multipart, Client};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWriteExt, ReadBuf};
use walkdir::WalkDir;

const API_US: &str = "https://api.pcloud.com";
const API_EU: &str = "https://eapi.pcloud.com";

/// The pCloud API region to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    US,
    EU,
}

impl Region {
    fn endpoint(&self) -> &'static str {
        match self {
            Region::US => API_US,
            Region::EU => API_EU,
        }
    }
}

/// Strategy for handling duplicate files during transfers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuplicateMode {
    Skip,
    Overwrite,
    Rename,
}

/// Errors that can occur during pCloud API operations.
#[derive(Debug, thiserror::Error)]
pub enum PCloudError {
    #[error("API error: {0}")]
    ApiError(String),
    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Not authenticated")]
    NotAuthenticated,
    #[error("Invalid path: {0}")]
    InvalidPath(String),
    #[error("File not found: {0}")]
    FileNotFound(String),
}

pub type Result<T> = std::result::Result<T, PCloudError>;

// --- INTERNAL HELPERS ---

/// A wrapper around an AsyncRead that triggers a callback on every read.
struct ProgressReader<R, F> {
    inner: R,
    callback: F,
}

impl<R, F> ProgressReader<R, F> {
    fn new(inner: R, callback: F) -> Self {
        Self { inner, callback }
    }
}

impl<R: AsyncRead + Unpin, F: FnMut(usize) + Unpin> AsyncRead for ProgressReader<R, F> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let poll = Pin::new(&mut self.inner).poll_read(cx, buf);
        let after = buf.filled().len();

        if let Poll::Ready(Ok(())) = &poll {
            let bytes_read = after - before;
            if bytes_read > 0 {
                (self.callback)(bytes_read);
            }
        }
        poll
    }
}

// --- STRUCTS ---

#[derive(Deserialize, Debug)]
struct ApiResponse {
    result: i32,
    #[serde(default)]
    auth: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct FileItem {
    pub name: String,
    #[serde(default)]
    pub isfolder: bool,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub modified: Option<String>,
}

#[derive(Deserialize, Debug)]
struct FolderMetadata {
    #[serde(default)]
    contents: Vec<FileItem>,
}

#[derive(Deserialize, Debug)]
struct ListFolderResponse {
    result: i32,
    #[serde(default)]
    metadata: Option<FolderMetadata>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AccountInfo {
    pub email: String,
    pub quota: u64,
    pub used_quota: u64,
    pub premium: bool,
}

impl AccountInfo {
    pub fn available(&self) -> u64 {
        self.quota.saturating_sub(self.used_quota)
    }

    pub fn usage_percent(&self) -> f64 {
        if self.quota == 0 {
            0.0
        } else {
            (self.used_quota as f64 / self.quota as f64) * 100.0
        }
    }
}

#[derive(Deserialize, Debug)]
struct AccountInfoResponse {
    result: i32,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    quota: Option<u64>,
    #[serde(default)]
    usedquota: Option<u64>,
    #[serde(default)]
    premium: Option<bool>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
    pub backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay_ms: 500,
            max_delay_ms: 30000,
            backoff_multiplier: 2.0,
        }
    }
}

#[derive(Clone)]
pub struct PCloudClient {
    client: Client,
    region: Region,
    auth_token: Option<String>,
    pub workers: usize,
    pub duplicate_mode: DuplicateMode,
    pub retry_config: RetryConfig,
}

impl PCloudClient {
    pub fn new(token: Option<String>, region: Region, workers: usize) -> Self {
        let client = Client::builder()
            .pool_max_idle_per_host(workers)
            .pool_idle_timeout(Some(std::time::Duration::from_secs(90)))
            .connect_timeout(std::time::Duration::from_secs(30))
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .unwrap_or_default();

        Self {
            client,
            region,
            auth_token: token,
            workers,
            duplicate_mode: DuplicateMode::Rename,
            retry_config: RetryConfig::default(),
        }
    }

    pub fn set_retry_config(&mut self, config: RetryConfig) {
        self.retry_config = config;
    }

    pub fn disable_retries(&mut self) {
        self.retry_config.max_retries = 0;
    }

    pub fn set_token(&mut self, token: String) {
        self.auth_token = Some(token);
    }

    pub fn set_duplicate_mode(&mut self, mode: DuplicateMode) {
        self.duplicate_mode = mode;
    }

    fn api_url(&self, method: &str) -> String {
        format!("{}/{}", self.region.endpoint(), method)
    }

    fn ensure_success(response: &ApiResponse) -> Result<()> {
        if response.result == 0 {
            Ok(())
        } else {
            Err(PCloudError::ApiError(
                response
                    .error
                    .clone()
                    .unwrap_or_else(|| format!("Error code: {}", response.result)),
            ))
        }
    }

    fn check_http_status(response: &reqwest::Response) -> Result<()> {
        let status = response.status();
        if status.is_success() {
            Ok(())
        } else {
            Err(PCloudError::ApiError(format!(
                "HTTP error: {} {}",
                status.as_u16(),
                status.canonical_reason().unwrap_or("Unknown")
            )))
        }
    }

    async fn api_get<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        params: &[(&str, &str)],
    ) -> Result<T> {
        let response = self.client.get(url).query(params).send().await?;
        Self::check_http_status(&response)?;
        let body = response.text().await?;
        serde_json::from_str(&body).map_err(|e| {
            PCloudError::ApiError(format!(
                "Failed to parse response: {} (body: {})",
                e,
                &body[..body.len().min(200)]
            ))
        })
    }

    async fn with_retry<F, Fut, T>(&self, operation: F) -> Result<T>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut attempt = 0;
        let mut delay = self.retry_config.initial_delay_ms;

        loop {
            match operation().await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    attempt += 1;
                    if attempt > self.retry_config.max_retries {
                        let retry = match &e {
                            PCloudError::NetworkError(_) => true,
                            PCloudError::ApiError(s) => s.starts_with("HTTP error: 5"),
                            _ => false,
                        };
                        if !retry {
                            return Err(e);
                        }
                    }

                    if attempt > self.retry_config.max_retries {
                        return Err(e);
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    delay = ((delay as f64) * self.retry_config.backoff_multiplier) as u64;
                    delay = delay.min(self.retry_config.max_delay_ms);
                }
            }
        }
    }

    pub async fn login(&mut self, username: &str, password: &str) -> Result<String> {
        let url = self.api_url("userinfo");
        let params = [
            ("username", username),
            ("password", password),
            ("getauth", "1"),
            ("logout", "1"),
        ];

        let api_resp: ApiResponse = self.api_get(&url, &params).await?;
        Self::ensure_success(&api_resp)?;

        let token = api_resp
            .auth
            .ok_or_else(|| PCloudError::ApiError("No auth token in response".to_string()))?;
        self.auth_token = Some(token.clone());
        Ok(token)
    }

    pub async fn create_folder(&self, path: &str) -> Result<()> {
        let url = self.api_url("createfolderifnotexists");
        let auth = self
            .auth_token
            .as_deref()
            .ok_or(PCloudError::NotAuthenticated)?;
        let params = [("auth", auth), ("path", path)];

        let api_resp: ApiResponse = self.with_retry(|| self.api_get(&url, &params)).await?;
        if api_resp.result == 0 || api_resp.result == 2004 {
            Ok(())
        } else {
            Self::ensure_success(&api_resp)
        }
    }

    pub async fn list_folder(&self, path: &str) -> Result<Vec<FileItem>> {
        let url = self.api_url("listfolder");
        let auth = self
            .auth_token
            .as_deref()
            .ok_or(PCloudError::NotAuthenticated)?;

        let params = vec![
            ("auth", auth),
            ("path", path),
            ("recursive", "0"),
            ("showdeleted", "0"),
        ];

        let api_resp: ListFolderResponse = self.with_retry(|| self.api_get(&url, &params)).await?;
        Self::ensure_success(&ApiResponse {
            result: api_resp.result,
            auth: None,
            error: api_resp.error,
        })?;

        Ok(api_resp.metadata.map(|m| m.contents).unwrap_or_default())
    }

    pub async fn delete_file(&self, path: &str) -> Result<()> {
        let url = self.api_url("deletefile");
        let auth = self
            .auth_token
            .as_deref()
            .ok_or(PCloudError::NotAuthenticated)?;
        let params = [("auth", auth), ("path", path)];
        let api_resp: ApiResponse = self.with_retry(|| self.api_get(&url, &params)).await?;
        Self::ensure_success(&api_resp)
    }

    pub async fn delete_folder(&self, path: &str) -> Result<()> {
        let url = self.api_url("deletefolderrecursive");
        let auth = self
            .auth_token
            .as_deref()
            .ok_or(PCloudError::NotAuthenticated)?;
        let params = [("auth", auth), ("path", path)];
        let api_resp: ApiResponse = self.with_retry(|| self.api_get(&url, &params)).await?;
        Self::ensure_success(&api_resp)
    }

    pub async fn rename_file(&self, from_path: &str, to_path: &str) -> Result<()> {
        let url = self.api_url("renamefile");
        let auth = self
            .auth_token
            .as_deref()
            .ok_or(PCloudError::NotAuthenticated)?;
        let params = [("auth", auth), ("path", from_path), ("topath", to_path)];
        let api_resp: ApiResponse = self.with_retry(|| self.api_get(&url, &params)).await?;
        Self::ensure_success(&api_resp)
    }

    pub async fn rename_folder(&self, from_path: &str, to_path: &str) -> Result<()> {
        let url = self.api_url("renamefolder");
        let auth = self
            .auth_token
            .as_deref()
            .ok_or(PCloudError::NotAuthenticated)?;
        let params = [("auth", auth), ("path", from_path), ("topath", to_path)];
        let api_resp: ApiResponse = self.with_retry(|| self.api_get(&url, &params)).await?;
        Self::ensure_success(&api_resp)
    }

    pub async fn get_account_info(&self) -> Result<AccountInfo> {
        let url = self.api_url("userinfo");
        let auth = self
            .auth_token
            .as_deref()
            .ok_or(PCloudError::NotAuthenticated)?;
        let params = [("auth", auth)];
        let api_resp: AccountInfoResponse = self.with_retry(|| self.api_get(&url, &params)).await?;

        if api_resp.result != 0 {
            return Err(PCloudError::ApiError(api_resp.error.unwrap_or_default()));
        }

        Ok(AccountInfo {
            email: api_resp.email.unwrap_or_default(),
            quota: api_resp.quota.unwrap_or(0),
            used_quota: api_resp.usedquota.unwrap_or(0),
            premium: api_resp.premium.unwrap_or(false),
        })
    }

    pub async fn get_download_link(&self, path: &str) -> Result<String> {
        let url = self.api_url("getfilelink");
        let auth = self
            .auth_token
            .as_deref()
            .ok_or(PCloudError::NotAuthenticated)?;
        let params = [("auth", auth), ("path", path)];

        #[derive(Deserialize)]
        struct LinkResponse {
            result: i32,
            hosts: Option<Vec<String>>,
            path: Option<String>,
            #[serde(default)]
            error: Option<String>,
        }

        let api_resp: LinkResponse = self.with_retry(|| self.api_get(&url, &params)).await?;

        if api_resp.result == 0 {
            if let Some(host) = api_resp.hosts.as_ref().and_then(|h| h.first()) {
                if let Some(p) = &api_resp.path {
                    return Ok(format!("https://{}{}", host, p));
                }
            }
        }
        Err(PCloudError::ApiError(
            api_resp
                .error
                .unwrap_or_else(|| "Unknown link error".into()),
        ))
    }

    // --- Duplicate Detection ---

    pub async fn check_file_exists(
        &self,
        remote_folder: &str,
        filename: &str,
    ) -> Result<Option<FileItem>> {
        let contents = self.list_folder(remote_folder).await?;
        Ok(contents
            .into_iter()
            .find(|item| !item.isfolder && item.name == filename))
    }

    // --- Uploads ---

    pub async fn upload_file(&self, local_path: &str, remote_path: &str) -> Result<()> {
        self.upload_file_with_progress(local_path, remote_path, |_| {})
            .await
    }

    pub async fn upload_file_with_progress<F>(
        &self,
        local_path: &str,
        remote_path: &str,
        progress_callback: F,
    ) -> Result<()>
    where
        F: FnMut(usize) + Send + Sync + 'static + Unpin,
    {
        let path = Path::new(local_path);
        if !path.exists() {
            return Err(PCloudError::FileNotFound(local_path.to_string()));
        }

        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| PCloudError::InvalidPath("Invalid filename".to_string()))?;

        if self.duplicate_mode != DuplicateMode::Rename {
            if let Ok(Some(_)) = self.check_file_exists(remote_path, filename).await {
                match self.duplicate_mode {
                    DuplicateMode::Skip => return Ok(()),
                    DuplicateMode::Overwrite => {
                        let temp_filename = format!("{}.tmp.{}", filename, uuid::Uuid::new_v4());
                        self.upload_internal(path, remote_path, &temp_filename, progress_callback)
                            .await?;

                        let full_remote = if remote_path == "/" {
                            format!("/{}", filename)
                        } else {
                            format!("{}/{}", remote_path.trim_end_matches('/'), filename)
                        };
                        let temp_remote = if remote_path == "/" {
                            format!("/{}", temp_filename)
                        } else {
                            format!("{}/{}", remote_path.trim_end_matches('/'), temp_filename)
                        };

                        let _ = self.delete_file(&full_remote).await;
                        self.rename_file(&temp_remote, &full_remote).await?;
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }

        self.upload_internal(path, remote_path, filename, progress_callback)
            .await
    }

    async fn upload_internal<F>(
        &self,
        local_file: &Path,
        remote_path: &str,
        filename: &str,
        progress_callback: F,
    ) -> Result<()>
    where
        F: FnMut(usize) + Send + Sync + 'static + Unpin,
    {
        let url = self.api_url("uploadfile");
        let auth = self
            .auth_token
            .as_deref()
            .ok_or(PCloudError::NotAuthenticated)?;

        let file = tokio::fs::File::open(local_file).await?;
        let file_size = file.metadata().await?.len();

        let reader = ProgressReader::new(file, progress_callback);
        let stream = tokio_util::io::ReaderStream::new(reader);
        let body = reqwest::Body::wrap_stream(stream);

        let part = multipart::Part::stream_with_length(body, file_size)
            .file_name(filename.to_string())
            .mime_str("application/octet-stream")
            .map_err(|e| PCloudError::ApiError(e.to_string()))?;

        let form = multipart::Form::new().part("file", part);

        let params = vec![
            ("auth", auth.to_string()),
            ("path", remote_path.to_string()),
            ("renameifexists", "1".to_string()),
        ];

        let response = self
            .client
            .post(&url)
            .query(&params)
            .multipart(form)
            .send()
            .await?;
        let api_resp: ApiResponse = response.json().await?;
        Self::ensure_success(&api_resp)?;
        Ok(())
    }

    // --- Downloads ---

    pub async fn download_file(&self, remote_path: &str, local_folder: &str) -> Result<String> {
        let download_url = self.get_download_link(remote_path).await?;
        let filename = remote_path
            .split('/')
            .next_back()
            .ok_or_else(|| PCloudError::InvalidPath("Invalid remote path".into()))?;
        let local_path = Path::new(local_folder).join(filename);

        if let Some(parent) = local_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let response = self.client.get(&download_url).send().await?;
        Self::check_http_status(&response)?;

        let mut file = tokio::fs::File::create(&local_path).await?;
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let data = chunk?;
            file.write_all(&data).await?;
        }
        file.flush().await?;
        Ok(local_path.to_string_lossy().to_string())
    }

    // --- Batch Helpers (Restored) ---

    pub async fn upload_folder_tree(
        &self,
        local_root: String,
        remote_base: String,
    ) -> Result<Vec<(String, String)>> {
        let mut files_to_upload = Vec::new();
        let mut folders_to_create = HashSet::new();

        let local_root_path = Path::new(&local_root);
        if !local_root_path.exists() {
            return Err(PCloudError::FileNotFound(local_root.clone()));
        }

        let folder_name = local_root_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| PCloudError::InvalidPath("Invalid folder name".to_string()))?;

        // follow_links(false) prevents infinite loops
        let walker = WalkDir::new(&local_root).follow_links(false);
        for entry in walker.into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            let relative_path = path
                .strip_prefix(&local_root)
                .map_err(|e| PCloudError::InvalidPath(e.to_string()))?;

            if relative_path.as_os_str().is_empty() {
                continue;
            }

            let relative_str = relative_path.to_string_lossy().replace("\\", "/");
            let remote_full_path = if remote_base == "/" {
                format!("/{}/{}", folder_name, relative_str)
            } else {
                format!(
                    "{}/{}/{}",
                    remote_base.trim_end_matches('/'),
                    folder_name,
                    relative_str
                )
            };

            if entry.file_type().is_dir() {
                folders_to_create.insert(remote_full_path);
            } else if entry.file_type().is_file() {
                let parent_remote = if let Some(parent) = Path::new(&remote_full_path).parent() {
                    parent.to_string_lossy().replace("\\", "/")
                } else {
                    remote_base.clone()
                };
                folders_to_create.insert(parent_remote.clone());
                files_to_upload.push((path.to_string_lossy().to_string(), parent_remote));
            }
        }

        // Parent robustness
        let collected_paths: Vec<String> = folders_to_create.iter().cloned().collect();
        for path_str in collected_paths {
            let mut current = Path::new(&path_str);
            while let Some(parent) = current.parent() {
                let parent_str = parent.to_string_lossy().replace("\\", "/");
                if parent_str == "/" || parent_str.is_empty() {
                    break;
                }
                folders_to_create.insert(parent_str);
                current = parent;
            }
        }

        // Create folders
        let mut sorted_folders: Vec<String> = folders_to_create.into_iter().collect();
        sorted_folders.sort_by_key(|a| a.matches('/').count());

        let mut folders_by_depth: HashMap<usize, Vec<String>> = HashMap::new();
        for f in sorted_folders {
            let depth = f.matches('/').count();
            folders_by_depth.entry(depth).or_default().push(f);
        }

        let mut depths: Vec<usize> = folders_by_depth.keys().cloned().collect();
        depths.sort();

        for depth in depths {
            if let Some(folders) = folders_by_depth.get(&depth) {
                let creations = stream::iter(folders.iter().cloned())
                    .map(|folder| {
                        let client = self.clone();
                        async move {
                            if folder != "/" {
                                let _ = client.create_folder(&folder).await;
                            }
                        }
                    })
                    .buffer_unordered(self.workers);

                creations.collect::<Vec<_>>().await;
            }
        }

        Ok(files_to_upload)
    }

    pub async fn download_folder_tree(
        &self,
        remote_root: String,
        local_base: String,
    ) -> Result<Vec<(String, String)>> {
        let mut files_to_download = Vec::new();
        let mut queue = vec![remote_root.clone()];

        let folder_name = Path::new(&remote_root)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("download");
        let local_dest_base = Path::new(&local_base).join(folder_name);

        while let Some(current_remote_path) = queue.pop() {
            match self.list_folder(&current_remote_path).await {
                Ok(items) => {
                    let suffix = current_remote_path
                        .strip_prefix(&remote_root)
                        .unwrap_or(&current_remote_path);
                    let local_dest_dir = local_dest_base.join(suffix.trim_start_matches('/'));

                    tokio::fs::create_dir_all(&local_dest_dir).await?;

                    for item in items {
                        if item.isfolder {
                            let next_path = format!(
                                "{}/{}",
                                current_remote_path.trim_end_matches('/'),
                                item.name
                            );
                            queue.push(next_path);
                        } else {
                            let remote_file_path = format!(
                                "{}/{}",
                                current_remote_path.trim_end_matches('/'),
                                item.name
                            );
                            files_to_download.push((
                                remote_file_path,
                                local_dest_dir.to_string_lossy().to_string(),
                            ));
                        }
                    }
                }
                Err(_e) => {}
            }
        }
        Ok(files_to_download)
    }

    pub async fn upload_files(&self, tasks: Vec<(String, String)>) -> (u32, u32) {
        let mut uploaded = 0;
        let mut failed = 0;

        let uploads = stream::iter(tasks)
            .map(|(local_path, remote_folder)| {
                let client = self.clone();
                async move {
                    let result = client.upload_file(&local_path, &remote_folder).await;
                    (local_path, remote_folder, result)
                }
            })
            .buffer_unordered(self.workers);

        let results: Vec<_> = uploads.collect().await;

        for (_path, _remote, res) in results {
            match res {
                Ok(_) => uploaded += 1,
                Err(_) => failed += 1,
            }
        }
        (uploaded, failed)
    }

    pub async fn download_files(&self, tasks: Vec<(String, String)>) -> (u32, u32) {
        let mut downloaded = 0;
        let mut failed = 0;

        let downloads = stream::iter(tasks)
            .map(|(remote_path, local_folder)| {
                let client = self.clone();
                async move {
                    let result = client.download_file(&remote_path, &local_folder).await;
                    (remote_path, result)
                }
            })
            .buffer_unordered(self.workers);

        let results: Vec<_> = downloads.collect().await;

        for (_path, res) in results {
            match res {
                Ok(_) => downloaded += 1,
                Err(_) => failed += 1,
            }
        }
        (downloaded, failed)
    }
}
