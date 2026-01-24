use reqwest::{Client, multipart};
use serde::Deserialize;
use std::path::Path;
use futures::stream::{self, StreamExt};
use tokio::io::AsyncWriteExt;
use std::collections::HashSet;
use walkdir::WalkDir;
use sha2::{Sha256, Digest};

// Constants
const API_US: &str = "https://api.pcloud.com";
const API_EU: &str = "https://eapi.pcloud.com";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuplicateMode {
    Skip,
    Overwrite,
    Rename,
}

// --- ERROR HANDLING ---

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

// --- STRUCTS ---

#[derive(Deserialize, Debug)]
struct ApiResponse {
    result: i32,
    #[serde(default)]
    auth: Option<String>,
    #[serde(default)]
    error: Option<String>,
    // Allow unknown fields from pCloud API
    #[serde(flatten)]
    #[serde(default)]
    #[allow(dead_code)]
    extra: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct FileItem {
    pub name: String,
    #[serde(default)]
    pub isfolder: bool,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub hash: Option<u64>,
    #[serde(default)]
    pub modified: Option<u64>,
    // Allow unknown fields from pCloud API
    #[serde(flatten)]
    #[serde(default)]
    #[allow(dead_code)]
    extra: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Deserialize, Debug)]
struct FolderMetadata {
    #[serde(default)]
    contents: Vec<FileItem>,
    // Allow unknown fields from pCloud API
    #[serde(flatten)]
    #[serde(default)]
    #[allow(dead_code)]
    extra: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Deserialize, Debug)]
struct ListFolderResponse {
    result: i32,
    #[serde(default)]
    metadata: Option<FolderMetadata>,
    #[serde(default)]
    error: Option<String>,
    // Allow unknown fields from pCloud API
    #[serde(flatten)]
    #[serde(default)]
    #[allow(dead_code)]
    extra: std::collections::HashMap<String, serde_json::Value>,
}

// --- THE CLIENT ---

#[derive(Clone)]
pub struct PCloudClient {
    client: Client,
    region: Region,
    auth_token: Option<String>,
    pub workers: usize,
    pub duplicate_mode: DuplicateMode,
}

impl PCloudClient {
    pub fn new(token: Option<String>, region: Region, workers: usize) -> Self {
        let client = Client::builder()
            .pool_max_idle_per_host(workers)
            .pool_idle_timeout(Some(std::time::Duration::from_secs(90)))
            .build()
            .unwrap_or_default();

        Self {
            client,
            region,
            auth_token: token,
            workers,
            duplicate_mode: DuplicateMode::Rename,
        }
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
                response.error.clone().unwrap_or_else(|| format!("Error code: {}", response.result))
            ))
        }
    }

    // --- Authentication ---
    pub async fn login(&mut self, username: &str, password: &str) -> Result<String> {
        let url = self.api_url("userinfo");
        let params = [
            ("username", username),
            ("password", password),
            ("getauth", "1"),
            ("logout", "1"),
        ];

        let response = self.client.get(&url).query(&params).send().await?;
        let api_resp: ApiResponse = response.json().await?;
        Self::ensure_success(&api_resp)?;

        let token = api_resp.auth.ok_or_else(||
            PCloudError::ApiError("No auth token in response".to_string())
        )?;
        self.auth_token = Some(token.clone());
        Ok(token)
    }

    // --- Core Operations ---

    pub async fn create_folder(&self, path: &str) -> Result<()> {
        let url = self.api_url("createfolderifnotexists");
        let auth = self.auth_token.as_deref().ok_or(PCloudError::NotAuthenticated)?;
        let params = [("auth", auth), ("path", path)];

        let response = self.client.get(&url).query(&params).send().await?;
        let api_resp: ApiResponse = response.json().await?;

        // Ignore "already exists" errors gracefully
        if api_resp.result == 0 || api_resp.result == 2004 {
            Ok(())
        } else {
            Self::ensure_success(&api_resp)
        }
    }

    pub async fn list_folder(&self, path: &str) -> Result<Vec<FileItem>> {
        let url = self.api_url("listfolder");
        let auth = self.auth_token.as_deref().ok_or(PCloudError::NotAuthenticated)?;

        // Try different parameter combinations for compatibility
        // Some pCloud APIs expect different params
        let params = vec![
            ("auth", auth),
            ("path", path),
            ("recursive", "0"),  // Don't list recursively
            ("showdeleted", "0"), // Don't show deleted files
        ];

        eprintln!("DEBUG: Listing folder: '{}'", path);
        eprintln!("DEBUG: API URL: {}", url);
        eprintln!("DEBUG: Token present: {}", auth.len() > 0);

        let response = match self.client.get(&url).query(&params).send().await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("ERROR: Network request failed: {}", e);
                return Err(e.into());
            }
        };

        eprintln!("DEBUG: Got HTTP response, status: {}", response.status());

        // Get response text first for better error reporting
        let response_text = match response.text().await {
            Ok(t) => t,
            Err(e) => {
                eprintln!("ERROR: Failed to read response body: {}", e);
                return Err(e.into());
            }
        };

        eprintln!("DEBUG: Response body length: {} bytes", response_text.len());
        if response_text.len() < 500 {
            eprintln!("DEBUG: Response body: {}", response_text);
        } else {
            eprintln!("DEBUG: Response body (first 500 chars): {}...", &response_text[..500]);
        }

        let api_resp: ListFolderResponse = match serde_json::from_str(&response_text) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("ERROR: Failed to parse JSON response: {}", e);
                eprintln!("ERROR: Parse error at line {} column {}", e.line(), e.column());
                return Err(PCloudError::ApiError(format!("JSON parse error: {}", e)));
            }
        };

        eprintln!("DEBUG: API result code: {}", api_resp.result);
        if let Some(ref error) = api_resp.error {
            eprintln!("DEBUG: API error message: {}", error);
        }

        if api_resp.result != 0 {
            let error_msg = api_resp.error.unwrap_or_else(|| format!("Unknown API error (code: {})", api_resp.result));
            eprintln!("ERROR: list_folder failed: {}", error_msg);
            return Err(PCloudError::ApiError(error_msg));
        }

        if api_resp.metadata.is_none() {
            eprintln!("WARNING: API returned success but no metadata");
        }

        let contents = api_resp.metadata.map(|m| m.contents).unwrap_or_default();
        eprintln!("DEBUG: Successfully retrieved {} items", contents.len());
        Ok(contents)
    }

    pub async fn get_download_link(&self, path: &str) -> Result<String> {
        let url = self.api_url("getfilelink");
        let auth = self.auth_token.as_deref().ok_or(PCloudError::NotAuthenticated)?;
        let params = [("auth", auth), ("path", path)];

        #[derive(Deserialize)]
        struct LinkResponse {
            result: i32,
            hosts: Option<Vec<String>>,
            path: Option<String>,
            #[serde(default)]
            error: Option<String>,
        }

        let response = self.client.get(&url).query(&params).send().await?;
        let api_resp: LinkResponse = response.json().await?;

        if api_resp.result == 0 {
            let hosts = api_resp.hosts.ok_or_else(||
                PCloudError::ApiError("No hosts in response".to_string())
            )?;
            let path = api_resp.path.ok_or_else(||
                PCloudError::ApiError("No path in response".to_string())
            )?;
            if let Some(host) = hosts.first() {
                return Ok(format!("https://{}{}", host, path));
            }
        }
        Err(PCloudError::ApiError(
            api_resp.error.unwrap_or_else(|| "Unknown error getting link".to_string())
        ))
    }

    // --- Duplicate Detection ---

    pub async fn check_file_exists(&self, remote_folder: &str, filename: &str) -> Result<Option<FileItem>> {
        let contents = self.list_folder(remote_folder).await?;
        Ok(contents.into_iter()
            .find(|item| !item.isfolder && item.name == filename))
    }

    #[allow(dead_code)]
    async fn calculate_file_hash(&self, file_path: &Path) -> Result<String> {
        let mut file = tokio::fs::File::open(file_path).await?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 8192];

        use tokio::io::AsyncReadExt;
        loop {
            let n = file.read(&mut buffer).await?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }

        Ok(hex::encode(hasher.finalize()))
    }

    async fn should_skip_upload(&self, local_file: &Path, remote_folder: &str) -> Result<(bool, Option<String>)> {
        if self.duplicate_mode == DuplicateMode::Rename {
            return Ok((false, None)); // Never skip, let pCloud rename
        }

        let filename = local_file.file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| PCloudError::InvalidPath("Invalid filename".to_string()))?;

        let existing_file = match self.check_file_exists(remote_folder, filename).await {
            Ok(Some(file)) => file,
            Ok(None) => return Ok((false, None)), // File doesn't exist
            Err(_) => return Ok((false, None)), // Error checking, don't skip
        };

        if self.duplicate_mode == DuplicateMode::Skip {
            let local_size = tokio::fs::metadata(local_file).await?.len();
            let remote_size = existing_file.size;

            if local_size != remote_size {
                return Ok((true, Some(format!(
                    "exists but different size (local: {}, remote: {})",
                    local_size, remote_size
                ))));
            }

            // Sizes match - consider them identical
            return Ok((true, Some(format!("likely identical (same size: {} bytes)", local_size))));
        }

        // Overwrite mode
        Ok((false, Some("will overwrite".to_string())))
    }

    // --- Transfer Logic ---

    pub async fn upload_file(&self, local_path: &str, remote_path: &str) -> Result<()> {
        let path = Path::new(local_path);

        if !path.exists() {
            return Err(PCloudError::FileNotFound(local_path.to_string()));
        }

        // Check for duplicates
        let (should_skip, skip_reason) = self.should_skip_upload(path, remote_path).await?;
        if should_skip {
            if let Some(reason) = skip_reason {
                eprintln!("⊘ Skipped {}: {}", path.file_name().unwrap().to_string_lossy(), reason);
            }
            return Ok(());
        }

        self.upload_file_streaming(path, remote_path).await
    }

    async fn upload_file_streaming(&self, local_file: &Path, remote_path: &str) -> Result<()> {
        let url = self.api_url("uploadfile");
        let auth = self.auth_token.as_deref().ok_or(PCloudError::NotAuthenticated)?;

        let filename = local_file.file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| PCloudError::InvalidPath("Invalid filename".to_string()))?
            .to_string();

        // Use streaming to avoid loading entire file into memory
        let file = tokio::fs::File::open(local_file).await?;
        let file_size = file.metadata().await?.len();

        // Convert tokio::fs::File to a stream
        let stream = tokio_util::io::ReaderStream::new(file);
        let body = reqwest::Body::wrap_stream(stream);

        let part = multipart::Part::stream_with_length(body, file_size)
            .file_name(filename)
            .mime_str("application/octet-stream")
            .map_err(|e| PCloudError::ApiError(e.to_string()))?;

        let form = multipart::Form::new().part("file", part);

        let mut params = vec![("auth", auth.to_string()), ("path", remote_path.to_string())];

        match self.duplicate_mode {
            DuplicateMode::Rename => params.push(("renameifexists", "1".to_string())),
            DuplicateMode::Overwrite => params.push(("nopartial", "1".to_string())),
            DuplicateMode::Skip => {}, // Already handled above
        }

        let response = self.client.post(&url)
            .query(&params)
            .multipart(form)
            .send()
            .await?;

        let api_resp: ApiResponse = response.json().await?;
        Self::ensure_success(&api_resp)?;
        Ok(())
    }

    pub async fn download_file(&self, remote_path: &str, local_folder: &str) -> Result<String> {
        let download_url = self.get_download_link(remote_path).await?;
        let filename = remote_path.split('/').next_back()
            .ok_or_else(|| PCloudError::InvalidPath("Invalid remote path".to_string()))?;
        let local_path = Path::new(local_folder).join(filename);

        // Ensure local folder exists
        if let Some(parent) = local_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let response = self.client.get(&download_url).send().await?;
        let mut file = tokio::fs::File::create(&local_path).await?;

        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let data = chunk?;
            file.write_all(&data).await?;
        }

        Ok(local_path.to_string_lossy().to_string())
    }

    // --- Batch / Recursive Logic ---

    /// Prepare a local folder for upload: walks the tree, creates remote folders, returns list of file tasks
    pub async fn upload_folder_tree(&self, local_root: String, remote_base: String) -> Result<Vec<(String, String)>> {
        let mut files_to_upload = Vec::new();
        let mut folders_to_create = HashSet::new();

        let local_root_path = Path::new(&local_root);
        if !local_root_path.exists() {
            return Err(PCloudError::FileNotFound(local_root.clone()));
        }

        // Get the folder name to preserve in remote structure
        let folder_name = local_root_path.file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| PCloudError::InvalidPath("Invalid folder name".to_string()))?;

        // 1. Walk local directory
        let walker = WalkDir::new(&local_root);
        for entry in walker.into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();

            // Calculate relative path from local_root
            let relative_path = path.strip_prefix(&local_root)
                .map_err(|e| PCloudError::InvalidPath(e.to_string()))?;

            // Skip the root itself
            if relative_path.as_os_str().is_empty() {
                continue;
            }

            let relative_str = relative_path.to_string_lossy().replace("\\", "/");

            // Construct full remote path, preserving folder name
            let remote_full_path = if remote_base == "/" {
                format!("/{}/{}", folder_name, relative_str)
            } else {
                format!("{}/{}/{}", remote_base.trim_end_matches('/'), folder_name, relative_str)
            };

            if entry.file_type().is_dir() {
                folders_to_create.insert(remote_full_path);
            } else if entry.file_type().is_file() {
                // For a file, we need its PARENT folder on the remote side
                let parent_remote = if let Some(parent) = Path::new(&remote_full_path).parent() {
                    parent.to_string_lossy().replace("\\", "/")
                } else {
                    remote_base.clone()
                };
                folders_to_create.insert(parent_remote.clone());
                files_to_upload.push((path.to_string_lossy().to_string(), parent_remote));
            }
        }

        // 2. Create folders (sorted by length so we create parents first)
        let mut sorted_folders: Vec<String> = folders_to_create.into_iter().collect();
        sorted_folders.sort_by_key(|a| a.matches('/').count());

        // Create folders in parallel batches to avoid overwhelming the API
        let folder_chunks: Vec<_> = sorted_folders.chunks(10).collect();
        for chunk in folder_chunks {
            let futures: Vec<_> = chunk.iter()
                .filter(|folder| *folder != "/")
                .map(|folder| self.create_folder(folder))
                .collect();

            // Wait for this batch to complete
            let results = futures::future::join_all(futures).await;

            // Log errors but don't fail the entire operation
            for (i, result) in results.into_iter().enumerate() {
                if let Err(e) = result {
                    eprintln!("Warning: Failed to create folder {}: {}", chunk[i], e);
                }
            }
        }

        Ok(files_to_upload)
    }

    /// Prepare a remote folder for download: scans tree, creates local folders, returns file list
    pub async fn download_folder_tree(&self, remote_root: String, local_base: String) -> Result<Vec<(String, String)>> {
        let mut files_to_download = Vec::new();
        let mut queue = vec![remote_root.clone()];

        // Get folder name to preserve structure
        let folder_name = Path::new(&remote_root).file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("download");
        let local_dest_base = Path::new(&local_base).join(folder_name);

        // Iterative BFS to scan remote folder
        while let Some(current_remote_path) = queue.pop() {
            match self.list_folder(&current_remote_path).await {
                Ok(items) => {
                    // Calculate local destination for this folder
                    let suffix = current_remote_path.strip_prefix(&remote_root)
                        .unwrap_or(&current_remote_path);
                    let local_dest_dir = local_dest_base.join(suffix.trim_start_matches('/'));

                    // Create this local directory
                    tokio::fs::create_dir_all(&local_dest_dir).await?;

                    for item in items {
                        if item.isfolder {
                            let next_path = format!("{}/{}", current_remote_path.trim_end_matches('/'), item.name);
                            queue.push(next_path);
                        } else {
                            let remote_file_path = format!("{}/{}", current_remote_path.trim_end_matches('/'), item.name);
                            files_to_download.push((remote_file_path, local_dest_dir.to_string_lossy().to_string()));
                        }
                    }
                }
                Err(e) => eprintln!("Error listing {}: {}", current_remote_path, e),
            }
        }

        Ok(files_to_download)
    }

    // --- Unified Batch Processor ---

    /// Upload multiple files in parallel
    /// Accepts list of (LocalFile, RemoteDestFolder)
    pub async fn upload_files(&self, tasks: Vec<(String, String)>) -> (u32, u32) {
        let mut uploaded = 0;
        let mut failed = 0;

        let uploads = stream::iter(tasks)
            .map(|(local_path, remote_folder)| {
                let client = self.clone();
                async move {
                    let result = client.upload_file(&local_path, &remote_folder).await;
                    (local_path, result)
                }
            })
            .buffer_unordered(self.workers);

        let results: Vec<_> = uploads.collect().await;

        for (path, res) in results {
            match res {
                Ok(_) => {
                    uploaded += 1;
                    println!("✓ Uploaded {}", Path::new(&path).file_name().unwrap().to_string_lossy());
                }
                Err(e) => {
                    eprintln!("✗ Failed {}: {}", path, e);
                    failed += 1;
                }
            }
        }
        (uploaded, failed)
    }

    /// Download multiple files in parallel
    /// Accepts list of (RemoteFile, LocalDestFolder)
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

        for (path, res) in results {
            match res {
                Ok(_) => {
                    downloaded += 1;
                    println!("✓ Downloaded {}", Path::new(&path).file_name().unwrap().to_string_lossy());
                }
                Err(e) => {
                    eprintln!("✗ Failed {}: {}", path, e);
                    failed += 1;
                }
            }
        }
        (downloaded, failed)
    }
}
