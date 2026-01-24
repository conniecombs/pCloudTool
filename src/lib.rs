//! # pCloud Rust Client
//!
//! A high-performance, async Rust client for the pCloud API with support for
//! parallel file transfers, recursive folder sync, and duplicate detection.
//!
//! ## Features
//!
//! - **Async/await** - Built on tokio for efficient concurrent operations
//! - **Streaming I/O** - Memory-efficient file transfers without loading files into RAM
//! - **Parallel transfers** - Configurable worker count for concurrent uploads/downloads
//! - **Recursive sync** - Upload or download entire directory trees
//! - **Duplicate detection** - Skip, overwrite, or rename duplicate files
//! - **Type-safe errors** - Comprehensive error handling with custom error types
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use pcloud_rust::{PCloudClient, Region, DuplicateMode};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create a client for the US region with 8 parallel workers
//!     let mut client = PCloudClient::new(None, Region::US, 8);
//!
//!     // Authenticate
//!     let token = client.login("user@example.com", "password").await?;
//!
//!     // Configure duplicate handling
//!     client.set_duplicate_mode(DuplicateMode::Skip);
//!
//!     // Upload a file
//!     client.upload_file("local/file.txt", "/remote/folder").await?;
//!
//!     // List folder contents
//!     let items = client.list_folder("/remote/folder").await?;
//!     for item in items {
//!         println!("{}: {}", if item.isfolder { "DIR" } else { "FILE" }, item.name);
//!     }
//!
//!     Ok(())
//! }
//! ```

use reqwest::{Client, multipart};
use serde::Deserialize;
use std::path::Path;
use futures::stream::{self, StreamExt};
use tokio::io::AsyncWriteExt;
use std::collections::{HashSet, HashMap};
use walkdir::WalkDir;
use sha2::{Sha256, Digest};

const API_US: &str = "https://api.pcloud.com";
const API_EU: &str = "https://eapi.pcloud.com";

/// The pCloud API region to use.
///
/// pCloud has separate API endpoints for US and EU users. Choose the region
/// closest to you for best performance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// US region (api.pcloud.com)
    US,
    /// EU region (eapi.pcloud.com)
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
    /// Skip files that already exist on the remote (based on filename and size)
    Skip,
    /// Overwrite existing files with the same name
    Overwrite,
    /// Let pCloud auto-rename the file (e.g., file(1).txt)
    Rename,
}

/// Errors that can occur during pCloud API operations.
#[derive(Debug, thiserror::Error)]
pub enum PCloudError {
    /// An error returned by the pCloud API
    #[error("API error: {0}")]
    ApiError(String),

    /// A network-level error (connection failed, timeout, etc.)
    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),

    /// A local filesystem I/O error
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// Operation requires authentication but no token is set
    #[error("Not authenticated")]
    NotAuthenticated,

    /// The provided path is invalid or malformed
    #[error("Invalid path: {0}")]
    InvalidPath(String),

    /// The specified file was not found locally
    #[error("File not found: {0}")]
    FileNotFound(String),
}

/// Result type alias for pCloud operations.
pub type Result<T> = std::result::Result<T, PCloudError>;

// --- STRUCTS ---

#[derive(Deserialize, Debug)]
struct ApiResponse {
    result: i32,
    #[serde(default)]
    auth: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(flatten)]
    #[serde(default)]
    #[allow(dead_code)]
    extra: std::collections::HashMap<String, serde_json::Value>,
}

/// Represents a file or folder in pCloud storage.
///
/// This struct contains metadata about items returned from folder listings.
#[derive(Deserialize, Debug, Clone)]
pub struct FileItem {
    /// The name of the file or folder
    pub name: String,
    /// Whether this item is a folder (true) or file (false)
    #[serde(default)]
    pub isfolder: bool,
    /// File size in bytes (0 for folders)
    #[serde(default)]
    pub size: u64,
    /// Creation timestamp (ISO 8601 format)
    #[serde(default)]
    pub created: Option<String>,
    /// Last modification timestamp (ISO 8601 format)
    #[serde(default)]
    pub modified: Option<String>,
    #[serde(flatten)]
    #[serde(default)]
    #[allow(dead_code)]
    extra: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Deserialize, Debug)]
struct FolderMetadata {
    #[serde(default)]
    contents: Vec<FileItem>,
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
    #[serde(flatten)]
    #[serde(default)]
    #[allow(dead_code)]
    extra: std::collections::HashMap<String, serde_json::Value>,
}

/// The main pCloud API client.
///
/// This client handles authentication and provides methods for all pCloud operations
/// including file uploads, downloads, folder management, and recursive sync.
///
/// # Example
///
/// ```rust,no_run
/// use pcloud_rust::{PCloudClient, Region};
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let mut client = PCloudClient::new(None, Region::US, 8);
///     client.login("user@example.com", "password").await?;
///
///     let files = client.list_folder("/").await?;
///     println!("Found {} items", files.len());
///     Ok(())
/// }
/// ```
#[derive(Clone)]
pub struct PCloudClient {
    client: Client,
    region: Region,
    auth_token: Option<String>,
    /// Number of parallel workers for batch operations
    pub workers: usize,
    /// How to handle duplicate files during transfers
    pub duplicate_mode: DuplicateMode,
}

impl PCloudClient {
    /// Creates a new pCloud client.
    ///
    /// # Arguments
    ///
    /// * `token` - Optional pre-existing authentication token
    /// * `region` - The pCloud API region to use (US or EU)
    /// * `workers` - Number of parallel workers for batch transfers
    ///
    /// # Example
    ///
    /// ```rust
    /// use pcloud_rust::{PCloudClient, Region};
    ///
    /// // Create client with 8 parallel workers
    /// let client = PCloudClient::new(None, Region::US, 8);
    /// ```
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

    /// Sets the authentication token directly.
    ///
    /// Use this if you have a pre-existing token from a previous session.
    pub fn set_token(&mut self, token: String) {
        self.auth_token = Some(token);
    }

    /// Sets the duplicate handling strategy for file transfers.
    ///
    /// # Arguments
    ///
    /// * `mode` - The duplicate handling mode (Skip, Overwrite, or Rename)
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

    /// Authenticates with pCloud using username and password.
    ///
    /// On success, stores the authentication token internally and returns it.
    /// The token can be saved and reused for future sessions via `set_token()`.
    ///
    /// # Arguments
    ///
    /// * `username` - pCloud account email
    /// * `password` - pCloud account password
    ///
    /// # Returns
    ///
    /// The authentication token on success.
    ///
    /// # Errors
    ///
    /// Returns `PCloudError::ApiError` if credentials are invalid.
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

    /// Creates a folder at the specified path.
    ///
    /// Creates the folder if it doesn't exist. Does not fail if the folder
    /// already exists (idempotent operation).
    ///
    /// # Arguments
    ///
    /// * `path` - The remote folder path (e.g., "/Documents/NewFolder")
    pub async fn create_folder(&self, path: &str) -> Result<()> {
        let url = self.api_url("createfolderifnotexists");
        let auth = self.auth_token.as_deref().ok_or(PCloudError::NotAuthenticated)?;
        let params = [("auth", auth), ("path", path)];

        let response = self.client.get(&url).query(&params).send().await?;
        let api_resp: ApiResponse = response.json().await?;

        if api_resp.result == 0 || api_resp.result == 2004 {
            Ok(())
        } else {
            Self::ensure_success(&api_resp)
        }
    }

    /// Lists the contents of a folder.
    ///
    /// Returns a list of files and subfolders in the specified folder.
    ///
    /// # Arguments
    ///
    /// * `path` - The remote folder path (e.g., "/" for root)
    ///
    /// # Returns
    ///
    /// A vector of `FileItem` structs representing folder contents.
    pub async fn list_folder(&self, path: &str) -> Result<Vec<FileItem>> {
        let url = self.api_url("listfolder");
        let auth = self.auth_token.as_deref().ok_or(PCloudError::NotAuthenticated)?;

        let params = vec![
            ("auth", auth),
            ("path", path),
            ("recursive", "0"), 
            ("showdeleted", "0"),
        ];

        let response = self.client.get(&url).query(&params).send().await?;
        let api_resp: ListFolderResponse = response.json().await?;

        if api_resp.result != 0 {
            let error_msg = api_resp.error.unwrap_or_else(|| format!("Unknown API error (code: {})", api_resp.result));
            return Err(PCloudError::ApiError(error_msg));
        }

        Ok(api_resp.metadata.map(|m| m.contents).unwrap_or_default())
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
            return Ok((false, None)); 
        }

        let filename = local_file.file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| PCloudError::InvalidPath("Invalid filename".to_string()))?;

        let existing_file = match self.check_file_exists(remote_folder, filename).await {
            Ok(Some(file)) => file,
            Ok(None) => return Ok((false, None)), 
            Err(_) => return Ok((false, None)), 
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
            return Ok((true, Some(format!("likely identical (same size: {} bytes)", local_size))));
        }
        Ok((false, Some("will overwrite".to_string())))
    }

    /// Uploads a single file to pCloud.
    ///
    /// Uses streaming I/O for memory-efficient transfers of large files.
    /// Respects the current `duplicate_mode` setting.
    ///
    /// # Arguments
    ///
    /// * `local_path` - Path to the local file to upload
    /// * `remote_path` - Remote folder to upload to (e.g., "/Documents")
    pub async fn upload_file(&self, local_path: &str, remote_path: &str) -> Result<()> {
        let path = Path::new(local_path);
        if !path.exists() {
            return Err(PCloudError::FileNotFound(local_path.to_string()));
        }
        
        let (should_skip, _) = self.should_skip_upload(path, remote_path).await?;
        if should_skip {
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

        let file = tokio::fs::File::open(local_file).await?;
        let file_size = file.metadata().await?.len();

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
            DuplicateMode::Skip => {},
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

    /// Downloads a single file from pCloud.
    ///
    /// Uses streaming I/O for memory-efficient transfers. Creates the
    /// local directory if it doesn't exist.
    ///
    /// # Arguments
    ///
    /// * `remote_path` - Full path to the remote file (e.g., "/Documents/file.txt")
    /// * `local_folder` - Local directory to save the file to
    ///
    /// # Returns
    ///
    /// The full local path of the downloaded file.
    pub async fn download_file(&self, remote_path: &str, local_folder: &str) -> Result<String> {
        let download_url = self.get_download_link(remote_path).await?;
        let filename = remote_path.split('/').next_back()
            .ok_or_else(|| PCloudError::InvalidPath("Invalid remote path".to_string()))?;
        let local_path = Path::new(local_folder).join(filename);

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

    /// Scans a local folder and prepares it for upload.
    ///
    /// Recursively scans the local directory, creates all necessary remote
    /// folders, and returns a list of file upload tasks.
    ///
    /// # Arguments
    ///
    /// * `local_root` - Path to the local folder to upload
    /// * `remote_base` - Remote folder to upload to (e.g., "/Backup")
    ///
    /// # Returns
    ///
    /// A vector of (local_path, remote_folder) tuples for use with `upload_files()`.
    pub async fn upload_folder_tree(&self, local_root: String, remote_base: String) -> Result<Vec<(String, String)>> {
        let mut files_to_upload = Vec::new();
        let mut folders_to_create = HashSet::new();

        let local_root_path = Path::new(&local_root);
        if !local_root_path.exists() {
            return Err(PCloudError::FileNotFound(local_root.clone()));
        }

        let folder_name = local_root_path.file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| PCloudError::InvalidPath("Invalid folder name".to_string()))?;

        // 1. Scan Local Files
        let walker = WalkDir::new(&local_root);
        for entry in walker.into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            let relative_path = path.strip_prefix(&local_root)
                .map_err(|e| PCloudError::InvalidPath(e.to_string()))?;

            if relative_path.as_os_str().is_empty() {
                continue;
            }

            let relative_str = relative_path.to_string_lossy().replace("\\", "/");
            let remote_full_path = if remote_base == "/" {
                format!("/{}/{}", folder_name, relative_str)
            } else {
                format!("{}/{}/{}", remote_base.trim_end_matches('/'), folder_name, relative_str)
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

        // 2. Robust Parent Generation (Fill Gaps)
        let collected_paths: Vec<String> = folders_to_create.iter().cloned().collect();
        for path_str in collected_paths {
            let mut current = Path::new(&path_str);
            while let Some(parent) = current.parent() {
                let parent_str = parent.to_string_lossy().replace("\\", "/");
                if parent_str == "/" || parent_str.is_empty() { break; }
                folders_to_create.insert(parent_str);
                current = parent;
            }
        }

        // 3. Optimized Parallel Folder Creation (Layer by Layer)
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
                // FIXED: Use .iter().cloned() to pass owned Strings to the async block
                let creations = stream::iter(folders.iter().cloned())
                    .map(|folder| {
                        let client = self.clone();
                        async move {
                            if folder != "/" {
                                if let Err(e) = client.create_folder(&folder).await {
                                    tracing::warn!("Failed to create folder {}: {}", folder, e);
                                }
                            }
                        }
                    })
                    .buffer_unordered(self.workers); 
                
                creations.collect::<Vec<_>>().await;
            }
        }

        Ok(files_to_upload)
    }

    /// Scans a remote folder and prepares it for download.
    ///
    /// Recursively scans the remote directory and returns a list of file
    /// download tasks. Creates all necessary local directories.
    ///
    /// # Arguments
    ///
    /// * `remote_root` - Remote folder to download (e.g., "/Documents")
    /// * `local_base` - Local directory to save files to
    ///
    /// # Returns
    ///
    /// A vector of (remote_path, local_folder) tuples for use with `download_files()`.
    pub async fn download_folder_tree(&self, remote_root: String, local_base: String) -> Result<Vec<(String, String)>> {
        let mut files_to_download = Vec::new();
        let mut queue = vec![remote_root.clone()];

        let folder_name = Path::new(&remote_root).file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("download");
        let local_dest_base = Path::new(&local_base).join(folder_name);

        while let Some(current_remote_path) = queue.pop() {
            match self.list_folder(&current_remote_path).await {
                Ok(items) => {
                    let suffix = current_remote_path.strip_prefix(&remote_root)
                        .unwrap_or(&current_remote_path);
                    let local_dest_dir = local_dest_base.join(suffix.trim_start_matches('/'));

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
                Err(e) => tracing::error!("Error listing {}: {}", current_remote_path, e),
            }
        }
        Ok(files_to_download)
    }

    /// Uploads multiple files in parallel.
    ///
    /// Processes the provided upload tasks using the configured number of workers.
    ///
    /// # Arguments
    ///
    /// * `tasks` - Vector of (local_path, remote_folder) tuples
    ///
    /// # Returns
    ///
    /// A tuple of (successful_count, failed_count).
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

        for (path, remote, res) in results {
            match res {
                Ok(_) => {
                    uploaded += 1;
                    let filename = Path::new(&path)
                        .file_name()
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_else(|| path.as_str().into());
                    tracing::info!("Uploaded {} -> {}", filename, remote);
                }
                Err(e) => {
                    tracing::error!("Failed {}: {}", path, e);
                    failed += 1;
                }
            }
        }
        (uploaded, failed)
    }

    /// Downloads multiple files in parallel.
    ///
    /// Processes the provided download tasks using the configured number of workers.
    ///
    /// # Arguments
    ///
    /// * `tasks` - Vector of (remote_path, local_folder) tuples
    ///
    /// # Returns
    ///
    /// A tuple of (successful_count, failed_count).
    /// Downloads multiple files in parallel.
    ///
    /// Processes the provided download tasks using the configured number of workers.
    ///
    /// # Arguments
    ///
    /// * `tasks` - Vector of (remote_path, local_folder) tuples
    ///
    /// # Returns
    ///
    /// A tuple of (successful_count, failed_count).
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
                    let filename = Path::new(&path)
                        .file_name()
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_else(|| path.as_str().into());
                    tracing::info!("Downloaded {}", filename);
                }
                Err(e) => {
                    tracing::error!("Failed {}: {}", path, e);
                    failed += 1;
                }
            }
        }
        (downloaded, failed)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::uninlined_format_args)]
mod tests {
    use super::*;

    #[test]
    fn test_region_endpoint() {
        assert_eq!(Region::US.endpoint(), "https://api.pcloud.com");
        assert_eq!(Region::EU.endpoint(), "https://eapi.pcloud.com");
    }

    #[test]
    fn test_duplicate_mode_default() {
        let client = PCloudClient::new(None, Region::US, 8);
        assert_eq!(client.duplicate_mode, DuplicateMode::Rename);
    }

    #[test]
    fn test_client_new_with_token() {
        let token = "test_token".to_string();
        let client = PCloudClient::new(Some(token.clone()), Region::EU, 4);
        assert_eq!(client.auth_token, Some(token));
        assert_eq!(client.workers, 4);
    }

    #[test]
    fn test_set_duplicate_mode() {
        let mut client = PCloudClient::new(None, Region::US, 8);
        client.set_duplicate_mode(DuplicateMode::Skip);
        assert_eq!(client.duplicate_mode, DuplicateMode::Skip);

        client.set_duplicate_mode(DuplicateMode::Overwrite);
        assert_eq!(client.duplicate_mode, DuplicateMode::Overwrite);
    }

    #[test]
    fn test_set_token() {
        let mut client = PCloudClient::new(None, Region::US, 8);
        assert!(client.auth_token.is_none());

        client.set_token("new_token".to_string());
        assert_eq!(client.auth_token, Some("new_token".to_string()));
    }

    #[test]
    fn test_api_url() {
        let client = PCloudClient::new(None, Region::US, 8);
        assert_eq!(client.api_url("listfolder"), "https://api.pcloud.com/listfolder");

        let client_eu = PCloudClient::new(None, Region::EU, 8);
        assert_eq!(client_eu.api_url("listfolder"), "https://eapi.pcloud.com/listfolder");
    }

    #[test]
    fn test_ensure_success_ok() {
        let response = ApiResponse {
            result: 0,
            auth: None,
            error: None,
            extra: std::collections::HashMap::new(),
        };
        assert!(PCloudClient::ensure_success(&response).is_ok());
    }

    #[test]
    fn test_ensure_success_error() {
        let response = ApiResponse {
            result: 2000,
            auth: None,
            error: Some("Invalid login".to_string()),
            extra: std::collections::HashMap::new(),
        };
        let result = PCloudClient::ensure_success(&response);
        assert!(result.is_err());
        if let Err(PCloudError::ApiError(msg)) = result {
            assert!(msg.contains("Invalid login"));
        }
    }

    #[test]
    fn test_file_item_deserialization() {
        let json = r#"{"name": "test.txt", "isfolder": false, "size": 1024}"#;
        let item: FileItem = serde_json::from_str(json).unwrap();
        assert_eq!(item.name, "test.txt");
        assert!(!item.isfolder);
        assert_eq!(item.size, 1024);
    }

    #[test]
    fn test_file_item_folder_deserialization() {
        let json = r#"{"name": "Documents", "isfolder": true}"#;
        let item: FileItem = serde_json::from_str(json).unwrap();
        assert_eq!(item.name, "Documents");
        assert!(item.isfolder);
        assert_eq!(item.size, 0); // default
    }

    #[test]
    fn test_pcloud_error_display() {
        let api_err = PCloudError::ApiError("Test error".to_string());
        assert_eq!(format!("{}", api_err), "API error: Test error");

        let auth_err = PCloudError::NotAuthenticated;
        assert_eq!(format!("{}", auth_err), "Not authenticated");

        let path_err = PCloudError::InvalidPath("/bad/path".to_string());
        assert_eq!(format!("{}", path_err), "Invalid path: /bad/path");
    }
}
