//! # pCloud Rust Client
//!
//! A high-performance, async Rust client for the pCloud API with support for
//! parallel file transfers, recursive folder sync, resume capability, and duplicate detection.

use futures::stream::{self, StreamExt};
use reqwest::{multipart, Client};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, ReadBuf};
use tracing::warn;
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DuplicateMode {
    Skip,
    Overwrite,
    Rename,
}

/// Direction for sync operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncDirection {
    /// Upload local changes to remote
    Upload,
    /// Download remote changes to local
    Download,
    /// Sync both directions (bidirectional)
    Bidirectional,
}

/// Information about a single file transfer.
#[derive(Debug, Clone)]
pub struct FileTransferInfo {
    pub filename: String,
    pub local_path: String,
    pub remote_path: String,
    pub size: u64,
    pub transferred: u64,
    pub is_complete: bool,
    pub is_failed: bool,
    pub error_message: Option<String>,
}

/// Progress callback for per-file tracking.
pub type FileProgressCallback = Arc<dyn Fn(FileTransferInfo) + Send + Sync>;

/// State of a transfer session that can be saved and resumed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferState {
    pub id: String,
    pub direction: String, // "upload" or "download"
    pub total_files: usize,
    pub completed_files: Vec<String>,
    pub failed_files: Vec<String>,
    pub pending_files: Vec<(String, String)>, // (local, remote) or (remote, local)
    pub total_bytes: u64,
    pub transferred_bytes: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

impl TransferState {
    pub fn new(direction: &str, files: Vec<(String, String)>, total_bytes: u64) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            direction: direction.to_string(),
            total_files: files.len(),
            completed_files: Vec::new(),
            failed_files: Vec::new(),
            pending_files: files,
            total_bytes,
            transferred_bytes: 0,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn mark_completed(&mut self, file_path: &str, bytes: u64) {
        self.pending_files.retain(|(l, _)| l != file_path);
        if !self.completed_files.contains(&file_path.to_string()) {
            self.completed_files.push(file_path.to_string());
            self.transferred_bytes += bytes;
        }
        self.update_timestamp();
    }

    pub fn mark_failed(&mut self, file_path: &str) {
        self.pending_files.retain(|(l, _)| l != file_path);
        if !self.failed_files.contains(&file_path.to_string()) {
            self.failed_files.push(file_path.to_string());
        }
        self.update_timestamp();
    }

    fn update_timestamp(&mut self) {
        self.updated_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
    }

    pub fn is_complete(&self) -> bool {
        self.pending_files.is_empty()
    }

    pub fn save_to_file(&self, path: &str) -> Result<()> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| PCloudError::IoError(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load_from_file(path: &str) -> Result<Self> {
        let json = std::fs::read_to_string(path)?;
        serde_json::from_str(&json)
            .map_err(|e| PCloudError::IoError(std::io::Error::new(std::io::ErrorKind::Other, e)))
    }
}

/// Result of a sync operation.
#[derive(Debug, Clone)]
pub struct SyncResult {
    pub uploaded: u32,
    pub downloaded: u32,
    pub skipped: u32,
    pub failed: u32,
    pub files_to_upload: Vec<String>,
    pub files_to_download: Vec<String>,
}

/// Information about a file for sync comparison.
#[derive(Debug, Clone)]
pub struct SyncFileInfo {
    pub path: String,
    pub size: u64,
    pub checksum: Option<String>,
    pub modified: Option<String>,
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

                    // Check if error is retryable (network errors or 5xx HTTP errors)
                    let is_retryable = match &e {
                        PCloudError::NetworkError(_) => true,
                        PCloudError::ApiError(s) => s.starts_with("HTTP error: 5"),
                        _ => false,
                    };

                    // Return immediately if error is not retryable or max retries exceeded
                    if !is_retryable || attempt > self.retry_config.max_retries {
                        return Err(e);
                    }

                    // Wait before retrying with exponential backoff
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
        let mut failed_folders: Vec<(String, String)> = Vec::new();

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
                Err(e) => {
                    // Log the error instead of silently ignoring it
                    warn!(
                        folder = %current_remote_path,
                        error = %e,
                        "Failed to list remote folder during download tree traversal"
                    );
                    failed_folders.push((current_remote_path, e.to_string()));
                }
            }
        }

        // Log summary if there were failures
        if !failed_folders.is_empty() {
            warn!(
                failed_count = failed_folders.len(),
                "Some folders could not be listed during download tree traversal"
            );
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

    // --- Per-File Progress Tracking ---

    /// Upload files with per-file progress tracking.
    /// The callback receives FileTransferInfo for each file as it progresses.
    pub async fn upload_files_with_progress(
        &self,
        tasks: Vec<(String, String)>,
        bytes_progress: Arc<AtomicU64>,
        file_callback: Option<FileProgressCallback>,
    ) -> (u32, u32, TransferState) {
        let total_bytes: u64 = tasks
            .iter()
            .map(|(p, _)| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
            .sum();

        let mut state = TransferState::new("upload", tasks.clone(), total_bytes);
        let mut uploaded = 0u32;
        let mut failed = 0u32;

        let uploads = stream::iter(tasks)
            .map(|(local_path, remote_folder)| {
                let client = self.clone();
                let bp = bytes_progress.clone();
                let fc = file_callback.clone();
                async move {
                    let size = std::fs::metadata(&local_path).map(|m| m.len()).unwrap_or(0);
                    let filename = Path::new(&local_path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown")
                        .to_string();

                    // Notify file start
                    if let Some(ref cb) = fc {
                        cb(FileTransferInfo {
                            filename: filename.clone(),
                            local_path: local_path.clone(),
                            remote_path: remote_folder.clone(),
                            size,
                            transferred: 0,
                            is_complete: false,
                            is_failed: false,
                            error_message: None,
                        });
                    }

                    let file_progress = Arc::new(AtomicU64::new(0));
                    let fp_clone = file_progress.clone();
                    let bp_clone = bp.clone();

                    let result = client
                        .upload_file_with_progress(&local_path, &remote_folder, move |bytes| {
                            fp_clone.fetch_add(bytes as u64, Ordering::Relaxed);
                            bp_clone.fetch_add(bytes as u64, Ordering::Relaxed);
                        })
                        .await;

                    let is_ok = result.is_ok();
                    let error_msg = result.err().map(|e| e.to_string());

                    // Notify file complete
                    if let Some(ref cb) = fc {
                        cb(FileTransferInfo {
                            filename,
                            local_path: local_path.clone(),
                            remote_path: remote_folder,
                            size,
                            transferred: if is_ok { size } else { 0 },
                            is_complete: is_ok,
                            is_failed: !is_ok,
                            error_message: error_msg,
                        });
                    }

                    (local_path, size, is_ok)
                }
            })
            .buffer_unordered(self.workers);

        let results: Vec<_> = uploads.collect().await;

        for (path, size, ok) in results {
            if ok {
                uploaded += 1;
                state.mark_completed(&path, size);
            } else {
                failed += 1;
                state.mark_failed(&path);
            }
        }

        (uploaded, failed, state)
    }

    /// Download files with per-file progress tracking.
    pub async fn download_files_with_progress(
        &self,
        tasks: Vec<(String, String)>,
        bytes_progress: Arc<AtomicU64>,
        file_callback: Option<FileProgressCallback>,
    ) -> (u32, u32, TransferState) {
        let mut state = TransferState::new("download", tasks.clone(), 0);
        let mut downloaded = 0u32;
        let mut failed = 0u32;

        let downloads = stream::iter(tasks)
            .map(|(remote_path, local_folder)| {
                let client = self.clone();
                let bp = bytes_progress.clone();
                let fc = file_callback.clone();
                async move {
                    let filename = remote_path
                        .split('/')
                        .next_back()
                        .unwrap_or("unknown")
                        .to_string();

                    // Notify file start
                    if let Some(ref cb) = fc {
                        cb(FileTransferInfo {
                            filename: filename.clone(),
                            local_path: local_folder.clone(),
                            remote_path: remote_path.clone(),
                            size: 0,
                            transferred: 0,
                            is_complete: false,
                            is_failed: false,
                            error_message: None,
                        });
                    }

                    let result = client.download_file(&remote_path, &local_folder).await;
                    let (is_ok, size, error_msg) = match &result {
                        Ok(path) => {
                            let s = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                            bp.fetch_add(s, Ordering::Relaxed);
                            (true, s, None)
                        }
                        Err(e) => (false, 0, Some(e.to_string())),
                    };

                    // Notify file complete
                    if let Some(ref cb) = fc {
                        cb(FileTransferInfo {
                            filename,
                            local_path: local_folder,
                            remote_path: remote_path.clone(),
                            size,
                            transferred: size,
                            is_complete: is_ok,
                            is_failed: !is_ok,
                            error_message: error_msg,
                        });
                    }

                    (remote_path, size, is_ok)
                }
            })
            .buffer_unordered(self.workers);

        let results: Vec<_> = downloads.collect().await;

        for (path, size, ok) in results {
            if ok {
                downloaded += 1;
                state.mark_completed(&path, size);
            } else {
                failed += 1;
                state.mark_failed(&path);
            }
        }

        (downloaded, failed, state)
    }

    // --- Resume Transfers ---

    /// Resume an upload from a saved transfer state.
    pub async fn resume_upload(
        &self,
        state: &mut TransferState,
        bytes_progress: Arc<AtomicU64>,
        file_callback: Option<FileProgressCallback>,
    ) -> (u32, u32) {
        let tasks = state.pending_files.clone();
        if tasks.is_empty() {
            return (0, 0);
        }

        let (uploaded, failed, new_state) = self
            .upload_files_with_progress(tasks, bytes_progress, file_callback)
            .await;

        // Merge results into existing state
        for completed in new_state.completed_files {
            if !state.completed_files.contains(&completed) {
                state.completed_files.push(completed);
            }
        }
        for failed_file in new_state.failed_files {
            if !state.failed_files.contains(&failed_file) {
                state.failed_files.push(failed_file);
            }
        }
        state.pending_files = new_state.pending_files;
        state.transferred_bytes += new_state.transferred_bytes;

        (uploaded, failed)
    }

    /// Resume a download from a saved transfer state.
    pub async fn resume_download(
        &self,
        state: &mut TransferState,
        bytes_progress: Arc<AtomicU64>,
        file_callback: Option<FileProgressCallback>,
    ) -> (u32, u32) {
        let tasks = state.pending_files.clone();
        if tasks.is_empty() {
            return (0, 0);
        }

        let (downloaded, failed, new_state) = self
            .download_files_with_progress(tasks, bytes_progress, file_callback)
            .await;

        // Merge results into existing state
        for completed in new_state.completed_files {
            if !state.completed_files.contains(&completed) {
                state.completed_files.push(completed);
            }
        }
        for failed_file in new_state.failed_files {
            if !state.failed_files.contains(&failed_file) {
                state.failed_files.push(failed_file);
            }
        }
        state.pending_files = new_state.pending_files;
        state.transferred_bytes += new_state.transferred_bytes;

        (downloaded, failed)
    }

    // --- Sync Mode ---

    /// Calculate SHA256 checksum of a local file.
    pub async fn compute_local_checksum(path: &str) -> Result<String> {
        let mut file = tokio::fs::File::open(path).await?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 65536]; // 64KB buffer

        loop {
            let bytes_read = file.read(&mut buffer).await?;
            if bytes_read == 0 {
                break;
            }
            hasher.update(&buffer[..bytes_read]);
        }

        Ok(hex::encode(hasher.finalize()))
    }

    /// Get file checksum from pCloud API.
    pub async fn get_remote_checksum(&self, path: &str) -> Result<String> {
        let url = self.api_url("checksumfile");
        let auth = self
            .auth_token
            .as_deref()
            .ok_or(PCloudError::NotAuthenticated)?;
        let params = [("auth", auth), ("path", path)];

        #[derive(Deserialize)]
        struct ChecksumResponse {
            result: i32,
            #[serde(default)]
            sha256: Option<String>,
            #[serde(default)]
            error: Option<String>,
        }

        let api_resp: ChecksumResponse = self.with_retry(|| self.api_get(&url, &params)).await?;

        if api_resp.result == 0 {
            api_resp
                .sha256
                .ok_or_else(|| PCloudError::ApiError("No checksum in response".to_string()))
        } else {
            Err(PCloudError::ApiError(api_resp.error.unwrap_or_else(|| {
                format!("Error code: {}", api_resp.result)
            })))
        }
    }

    /// Compare local and remote folders and determine what needs to be synced.
    pub async fn compare_folders(
        &self,
        local_path: &str,
        remote_path: &str,
        use_checksum: bool,
    ) -> Result<(Vec<(String, String)>, Vec<(String, String)>)> {
        let mut to_upload: Vec<(String, String)> = Vec::new();
        let mut to_download: Vec<(String, String)> = Vec::new();

        // Get remote files
        let remote_items = self.list_folder(remote_path).await.unwrap_or_default();
        let remote_files: HashMap<String, &FileItem> = remote_items
            .iter()
            .filter(|i| !i.isfolder)
            .map(|i| (i.name.clone(), i))
            .collect();

        // Scan local files
        let local_root = Path::new(local_path);
        if !local_root.exists() {
            return Err(PCloudError::FileNotFound(local_path.to_string()));
        }

        let mut local_files: HashMap<String, (String, u64)> = HashMap::new();

        if local_root.is_dir() {
            for entry in WalkDir::new(local_path)
                .follow_links(false)
                .max_depth(1)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                if path.is_file() {
                    if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                        local_files.insert(
                            filename.to_string(),
                            (path.to_string_lossy().to_string(), size),
                        );
                    }
                }
            }
        }

        // Compare and determine what to sync
        for (filename, (local_file_path, local_size)) in &local_files {
            let needs_upload = if let Some(remote_item) = remote_files.get(filename) {
                if use_checksum {
                    // Compare checksums
                    let local_hash = Self::compute_local_checksum(local_file_path).await.ok();
                    let remote_hash = self
                        .get_remote_checksum(&format!(
                            "{}/{}",
                            remote_path.trim_end_matches('/'),
                            filename
                        ))
                        .await
                        .ok();
                    local_hash != remote_hash
                } else {
                    // Compare sizes
                    *local_size != remote_item.size
                }
            } else {
                true // File doesn't exist remotely
            };

            if needs_upload {
                to_upload.push((local_file_path.clone(), remote_path.to_string()));
            }
        }

        // Find files that exist remotely but not locally
        for filename in remote_files.keys() {
            if !local_files.contains_key(filename) {
                let remote_file_path =
                    format!("{}/{}", remote_path.trim_end_matches('/'), filename);
                to_download.push((remote_file_path, local_path.to_string()));
            }
        }

        Ok((to_upload, to_download))
    }

    /// Sync a local folder with a remote folder.
    pub async fn sync_folder(
        &self,
        local_path: &str,
        remote_path: &str,
        direction: SyncDirection,
        use_checksum: bool,
    ) -> Result<SyncResult> {
        // Ensure remote folder exists
        self.create_folder(remote_path).await?;

        // Compare folders
        let (to_upload, to_download) = self
            .compare_folders(local_path, remote_path, use_checksum)
            .await?;

        let mut result = SyncResult {
            uploaded: 0,
            downloaded: 0,
            skipped: 0,
            failed: 0,
            files_to_upload: to_upload.iter().map(|(l, _)| l.clone()).collect(),
            files_to_download: to_download.iter().map(|(r, _)| r.clone()).collect(),
        };

        // Perform sync based on direction
        match direction {
            SyncDirection::Upload => {
                if !to_upload.is_empty() {
                    let (uploaded, failed) = self.upload_files(to_upload).await;
                    result.uploaded = uploaded;
                    result.failed = failed;
                }
                result.skipped = to_download.len() as u32;
            }
            SyncDirection::Download => {
                if !to_download.is_empty() {
                    let (downloaded, failed) = self.download_files(to_download).await;
                    result.downloaded = downloaded;
                    result.failed += failed;
                }
                result.skipped = to_upload.len() as u32;
            }
            SyncDirection::Bidirectional => {
                if !to_upload.is_empty() {
                    let (uploaded, failed) = self.upload_files(to_upload).await;
                    result.uploaded = uploaded;
                    result.failed = failed;
                }
                if !to_download.is_empty() {
                    let (downloaded, failed) = self.download_files(to_download).await;
                    result.downloaded = downloaded;
                    result.failed += failed;
                }
            }
        }

        Ok(result)
    }

    /// Recursively sync folder trees.
    pub async fn sync_folder_recursive(
        &self,
        local_root: &str,
        remote_root: &str,
        direction: SyncDirection,
        use_checksum: bool,
    ) -> Result<SyncResult> {
        let mut total_result = SyncResult {
            uploaded: 0,
            downloaded: 0,
            skipped: 0,
            failed: 0,
            files_to_upload: Vec::new(),
            files_to_download: Vec::new(),
        };

        // Sync root folder first
        let root_result = self
            .sync_folder(local_root, remote_root, direction, use_checksum)
            .await?;

        total_result.uploaded += root_result.uploaded;
        total_result.downloaded += root_result.downloaded;
        total_result.skipped += root_result.skipped;
        total_result.failed += root_result.failed;
        total_result
            .files_to_upload
            .extend(root_result.files_to_upload);
        total_result
            .files_to_download
            .extend(root_result.files_to_download);

        // Find and sync subfolders
        let local_root_path = Path::new(local_root);
        if local_root_path.is_dir() {
            for entry in std::fs::read_dir(local_root)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    if let Some(folder_name) = path.file_name().and_then(|n| n.to_str()) {
                        let local_subfolder = path.to_string_lossy().to_string();
                        let remote_subfolder =
                            format!("{}/{}", remote_root.trim_end_matches('/'), folder_name);

                        // Create remote folder if needed
                        let _ = self.create_folder(&remote_subfolder).await;

                        // Recursively sync subfolder
                        let sub_result = Box::pin(self.sync_folder_recursive(
                            &local_subfolder,
                            &remote_subfolder,
                            direction,
                            use_checksum,
                        ))
                        .await?;

                        total_result.uploaded += sub_result.uploaded;
                        total_result.downloaded += sub_result.downloaded;
                        total_result.skipped += sub_result.skipped;
                        total_result.failed += sub_result.failed;
                        total_result
                            .files_to_upload
                            .extend(sub_result.files_to_upload);
                        total_result
                            .files_to_download
                            .extend(sub_result.files_to_download);
                    }
                }
            }
        }

        Ok(total_result)
    }
}
