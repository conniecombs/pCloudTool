//! # pCloud Rust Client
//!
//! A high-performance, production-ready async Rust client for the pCloud API.
//!
//! This crate provides a complete solution for interacting with pCloud storage,
//! featuring parallel file transfers, recursive folder synchronization, resumable
//! transfers, and comprehensive error handling.
//!
//! ## Features
//!
//! - **Parallel Transfers** — Upload and download multiple files concurrently with
//!   configurable worker pools (1–32 workers)
//! - **Adaptive Concurrency** — Automatically determines optimal worker count based
//!   on available CPU cores and system memory
//! - **Resumable Transfers** — Persist and restore interrupted transfer sessions with
//!   integrity validation and automatic repair
//! - **Bidirectional Sync** — Synchronize local and remote directories with optional
//!   SHA-256 checksum verification
//! - **Chunked Uploads** — Handle files larger than 2 GB using pCloud's chunked upload API
//! - **Per-File Timeouts** — Configurable size-based timeouts with automatic retry
//!   and exponential backoff
//! - **Streaming I/O** — Memory-efficient transfers maintaining constant ~10 MB usage
//!   regardless of file size
//! - **Zero Unsafe Code** — Entirely safe Rust with compile-time memory guarantees
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use pcloud_rust::{PCloudClient, Region, DuplicateMode};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create a client with adaptive worker count
//!     let mut client = PCloudClient::new_adaptive(None, Region::US);
//!
//!     // Authenticate with pCloud
//!     let token = client.login("user@example.com", "password").await?;
//!     println!("Authenticated successfully");
//!
//!     // Upload a single file
//!     client.upload_file("local/file.txt", "/remote/folder").await?;
//!
//!     // Upload an entire directory tree
//!     let tasks = client.upload_folder_tree(
//!         "local/folder".into(),
//!         "/remote/backup".into(),
//!     ).await?;
//!     let (uploaded, failed) = client.upload_files(tasks).await;
//!     println!("Uploaded {uploaded} files, {failed} failed");
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Error Handling
//!
//! All operations return [`Result<T, PCloudError>`]. The [`PCloudError`] enum provides
//! detailed, actionable error information:
//!
//! ```rust,no_run
//! use pcloud_rust::{PCloudClient, PCloudError, Region};
//!
//! # async fn example() -> Result<(), PCloudError> {
//! let client = PCloudClient::new_adaptive(None, Region::US);
//!
//! match client.list_folder("/nonexistent").await {
//!     Ok(files) => println!("Found {} files", files.len()),
//!     Err(PCloudError::ApiError(msg)) => eprintln!("API error: {msg}"),
//!     Err(PCloudError::NotAuthenticated) => eprintln!("Please log in first"),
//!     Err(e) => eprintln!("Unexpected error: {e}"),
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Configuration
//!
//! The client supports extensive configuration through builder-style methods:
//!
//! ```rust,no_run
//! use pcloud_rust::{PCloudClient, Region, DuplicateMode, RetryConfig, FileTimeoutConfig};
//!
//! let mut client = PCloudClient::new(None, Region::EU, 16);
//!
//! // Configure duplicate file handling
//! client.set_duplicate_mode(DuplicateMode::Skip);
//!
//! // Configure retry behavior
//! client.set_retry_config(RetryConfig {
//!     max_retries: 5,
//!     initial_delay_ms: 1000,
//!     max_delay_ms: 60_000,
//!     backoff_multiplier: 2.0,
//! });
//!
//! // Configure per-file timeouts
//! client.set_file_timeout_config(FileTimeoutConfig {
//!     base_timeout_secs: 30,
//!     secs_per_mb: 3,
//!     max_timeout_secs: 3600,
//! });
//! ```
//!
//! ## Crate Features
//!
//! This crate is designed with sensible defaults and requires no feature flags for
//! standard usage. All functionality is available out of the box.
//!
//! ## Safety
//!
//! This crate forbids unsafe code and relies entirely on Rust's type system and
//! ownership model for memory safety. All network operations use TLS 1.2+.

// =============================================================================
// Imports
// =============================================================================

use futures::stream::{self, StreamExt};
use reqwest::{multipart, Client};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use sysinfo::System;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, ReadBuf};
use tracing::{debug, info, instrument, warn};
use walkdir::WalkDir;

// =============================================================================
// Constants
// =============================================================================

/// Base URL for the pCloud US API endpoint.
const API_US: &str = "https://api.pcloud.com";

/// Base URL for the pCloud EU API endpoint.
const API_EU: &str = "https://eapi.pcloud.com";

/// Default chunk size for large file uploads (10 MiB).
///
/// This value balances memory usage with upload efficiency. Larger chunks
/// reduce HTTP overhead but increase memory consumption and retry cost.
const DEFAULT_CHUNK_SIZE: u64 = 10 * 1024 * 1024;

/// File size threshold above which chunked uploads are used (2 GiB).
///
/// The pCloud API has limitations on single-request uploads for very large
/// files. Files exceeding this threshold are automatically uploaded in chunks.
const LARGE_FILE_THRESHOLD: u64 = 2 * 1024 * 1024 * 1024;

/// Default maximum timeout for file operations in seconds (10 minutes).
///
/// This serves as the upper bound for size-based timeout calculations.
const DEFAULT_FILE_TIMEOUT_SECS: u64 = 600;

/// Minimum number of concurrent workers allowed.
const MIN_WORKERS: usize = 1;

/// Maximum number of concurrent workers allowed.
///
/// Higher values increase parallelism but may overwhelm the API or exhaust
/// system resources. The adaptive worker calculation respects this limit.
const MAX_WORKERS: usize = 32;

// =============================================================================
// Enums & Types
// =============================================================================

/// The pCloud API region to connect to.
///
/// pCloud operates data centers in both the United States and Europe. Selecting
/// the region closest to you (or where your account is registered) will provide
/// the best performance.
///
/// # Example
///
/// ```rust
/// use pcloud_rust::Region;
///
/// let region = Region::EU;
/// assert_eq!(region.to_string(), "EU");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Region {
    /// United States data center (api.pcloud.com).
    #[default]
    US,
    /// European Union data center (eapi.pcloud.com).
    EU,
}

impl Region {
    /// Returns the API base URL for this region.
    #[inline]
    #[must_use]
    pub const fn endpoint(&self) -> &'static str {
        match self {
            Self::US => API_US,
            Self::EU => API_EU,
        }
    }

    /// Returns a human-readable description of the region.
    #[inline]
    #[must_use]
    pub const fn description(&self) -> &'static str {
        match self {
            Self::US => "United States",
            Self::EU => "European Union",
        }
    }
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::US => write!(f, "US"),
            Self::EU => write!(f, "EU"),
        }
    }
}

/// Strategy for handling duplicate files during transfers.
///
/// When uploading or downloading files, you may encounter files that already
/// exist at the destination. This enum controls how such conflicts are resolved.
///
/// # Example
///
/// ```rust
/// use pcloud_rust::{PCloudClient, Region, DuplicateMode};
///
/// let mut client = PCloudClient::new(None, Region::US, 8);
///
/// // Skip files that already exist
/// client.set_duplicate_mode(DuplicateMode::Skip);
///
/// // Or overwrite existing files
/// client.set_duplicate_mode(DuplicateMode::Overwrite);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DuplicateMode {
    /// Skip files that already exist at the destination.
    ///
    /// This is the safest option and preserves existing data.
    Skip,
    /// Overwrite existing files with the new version.
    ///
    /// Use with caution as this permanently replaces existing files.
    Overwrite,
    /// Allow pCloud to automatically rename the file to avoid conflicts.
    ///
    /// The new file will be given a unique name like `file (1).txt`.
    #[default]
    Rename,
}

impl fmt::Display for DuplicateMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Skip => write!(f, "skip"),
            Self::Overwrite => write!(f, "overwrite"),
            Self::Rename => write!(f, "rename"),
        }
    }
}

/// Direction for folder synchronization operations.
///
/// Controls how files are synchronized between local and remote directories.
///
/// # Example
///
/// ```rust,no_run
/// use pcloud_rust::{PCloudClient, Region, SyncDirection};
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let client = PCloudClient::new_adaptive(None, Region::US);
///
/// // Only upload new/changed local files
/// let result = client.sync_folder(
///     "./local",
///     "/remote",
///     SyncDirection::Upload,
///     false,
/// ).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum SyncDirection {
    /// Upload local changes to remote storage.
    ///
    /// Files that exist locally but not remotely (or are newer locally)
    /// will be uploaded. Remote-only files are ignored.
    Upload,
    /// Download remote changes to local storage.
    ///
    /// Files that exist remotely but not locally (or are newer remotely)
    /// will be downloaded. Local-only files are ignored.
    Download,
    /// Synchronize in both directions.
    ///
    /// Both local and remote directories are brought into sync. This is
    /// the most comprehensive option but may overwrite changes on either side.
    #[default]
    Bidirectional,
}

impl fmt::Display for SyncDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Upload => write!(f, "upload"),
            Self::Download => write!(f, "download"),
            Self::Bidirectional => write!(f, "bidirectional"),
        }
    }
}

// =============================================================================
// Progress Tracking
// =============================================================================

/// Information about a single file transfer in progress.
///
/// This struct is passed to progress callbacks during file transfers,
/// providing real-time visibility into transfer status.
///
/// # Example
///
/// ```rust,no_run
/// use pcloud_rust::FileTransferInfo;
/// use std::sync::Arc;
///
/// let callback = Arc::new(|info: FileTransferInfo| {
///     if info.is_complete {
///         println!("✓ Completed: {}", info.filename);
///     } else if info.is_failed {
///         println!("✗ Failed: {} - {:?}", info.filename, info.error_message);
///     } else {
///         let percent = (info.transferred as f64 / info.size as f64) * 100.0;
///         println!("  {} - {:.1}%", info.filename, percent);
///     }
/// });
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTransferInfo {
    /// The filename (without path) being transferred.
    pub filename: String,
    /// The full local file path.
    pub local_path: String,
    /// The full remote file path.
    pub remote_path: String,
    /// Total size of the file in bytes.
    pub size: u64,
    /// Number of bytes transferred so far.
    pub transferred: u64,
    /// Whether the transfer completed successfully.
    pub is_complete: bool,
    /// Whether the transfer failed.
    pub is_failed: bool,
    /// Error message if the transfer failed.
    pub error_message: Option<String>,
}

impl FileTransferInfo {
    /// Returns the transfer progress as a percentage (0.0 to 100.0).
    #[inline]
    #[must_use]
    pub fn progress_percent(&self) -> f64 {
        if self.size == 0 {
            if self.is_complete {
                100.0
            } else {
                0.0
            }
        } else {
            (self.transferred as f64 / self.size as f64) * 100.0
        }
    }

    /// Returns whether the transfer is currently in progress.
    #[inline]
    #[must_use]
    pub const fn is_in_progress(&self) -> bool {
        !self.is_complete && !self.is_failed
    }
}

/// Type alias for a thread-safe progress callback function.
///
/// This callback is invoked during file transfers to report progress.
/// It receives a [`FileTransferInfo`] struct with current transfer state.
pub type FileProgressCallback = Arc<dyn Fn(FileTransferInfo) + Send + Sync>;

// =============================================================================
// Transfer State Management
// =============================================================================

/// Current version of the transfer state file format.
const TRANSFER_STATE_VERSION: u32 = 1;

/// Serializable state of a transfer session for resumption.
///
/// This struct captures all information needed to resume an interrupted
/// transfer operation. It can be serialized to JSON and saved to disk.
///
/// # Integrity
///
/// The state includes a SHA-256 checksum for integrity verification.
/// When loading a state file, use [`TransferState::load_and_validate`]
/// to detect and optionally repair corruption.
///
/// # Example
///
/// ```rust,no_run
/// use pcloud_rust::TransferState;
///
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // Create a new transfer state
/// let files = vec![
///     ("local/a.txt".to_string(), "/remote/a.txt".to_string()),
///     ("local/b.txt".to_string(), "/remote/b.txt".to_string()),
/// ];
/// let state = TransferState::new("upload", files, 1024 * 1024);
///
/// // Save to disk
/// state.save_to_file("transfer.json")?;
///
/// // Later: Load and resume
/// let (mut state, validation) = TransferState::load_and_validate("transfer.json")?;
/// if !validation.is_valid {
///     eprintln!("State file issues: {:?}", validation.issues);
///     if validation.can_repair {
///         state.repair();
///     }
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferState {
    /// Unique identifier for this transfer session (UUID v4).
    pub id: String,
    /// Transfer direction: `"upload"` or `"download"`.
    pub direction: String,
    /// Total number of files in the transfer.
    pub total_files: usize,
    /// Paths of files that completed successfully.
    pub completed_files: Vec<String>,
    /// Paths of files that failed to transfer.
    pub failed_files: Vec<String>,
    /// Files remaining to be transferred: `(source, destination)` pairs.
    pub pending_files: Vec<(String, String)>,
    /// Total bytes across all files.
    pub total_bytes: u64,
    /// Bytes successfully transferred so far.
    pub transferred_bytes: u64,
    /// Unix timestamp when the transfer was created.
    pub created_at: u64,
    /// Unix timestamp of the last update.
    pub updated_at: u64,
    /// State file format version for compatibility checking.
    #[serde(default = "default_state_version")]
    pub version: u32,
    /// SHA-256 checksum for integrity validation.
    #[serde(default)]
    pub checksum: Option<String>,
}

/// Returns the current state file version.
#[inline]
const fn default_state_version() -> u32 {
    TRANSFER_STATE_VERSION
}

/// Result of validating a transfer state file.
///
/// Returned by [`TransferState::validate`] and [`TransferState::load_and_validate`].
///
/// # Example
///
/// ```rust,no_run
/// use pcloud_rust::TransferState;
///
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let (mut state, validation) = TransferState::load_and_validate("transfer.json")?;
///
/// if !validation.is_valid {
///     for issue in &validation.issues {
///         eprintln!("Warning: {issue}");
///     }
///
///     if validation.can_repair {
///         let repairs = state.repair();
///         println!("Applied {} repairs", repairs.len());
///     }
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateValidation {
    /// Whether the state file passed all validation checks.
    pub is_valid: bool,
    /// List of issues found during validation.
    pub issues: Vec<String>,
    /// Whether automatic repair is possible.
    pub can_repair: bool,
}

impl StateValidation {
    /// Creates a new validation result indicating success.
    #[inline]
    #[must_use]
    pub const fn valid() -> Self {
        Self {
            is_valid: true,
            issues: Vec::new(),
            can_repair: true,
        }
    }
}

/// Returns the current Unix timestamp in seconds.
#[inline]
fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Formats a byte count as a human-readable string.
///
/// # Examples
///
/// ```rust
/// use pcloud_rust::format_bytes;
///
/// assert_eq!(format_bytes(0), "0 B");
/// assert_eq!(format_bytes(1024), "1.0 KB");
/// assert_eq!(format_bytes(1536), "1.5 KB");
/// ```
#[must_use]
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB", "PB"];

    if bytes == 0 {
        return "0 B".to_string();
    }

    let exp = (bytes as f64).log2() / 10.0;
    let exp = (exp.floor() as usize).min(UNITS.len() - 1);

    if exp == 0 {
        format!("{bytes} B")
    } else {
        let value = bytes as f64 / (1u64 << (exp * 10)) as f64;
        format!("{value:.1} {}", UNITS[exp])
    }
}

impl TransferState {
    /// Creates a new transfer state with the given files.
    ///
    /// # Arguments
    ///
    /// * `direction` - Either `"upload"` or `"download"`
    /// * `files` - Vector of `(source, destination)` path pairs
    /// * `total_bytes` - Total size of all files in bytes
    ///
    /// # Example
    ///
    /// ```rust
    /// use pcloud_rust::TransferState;
    ///
    /// let files = vec![
    ///     ("local/file.txt".into(), "/remote/file.txt".into()),
    /// ];
    /// let state = TransferState::new("upload", files, 1024);
    /// assert!(!state.is_complete());
    /// ```
    #[must_use]
    pub fn new(direction: &str, files: Vec<(String, String)>, total_bytes: u64) -> Self {
        let now = current_timestamp();
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
            version: TRANSFER_STATE_VERSION,
            checksum: None,
        }
    }

    /// Marks a file as successfully completed.
    ///
    /// Moves the file from pending to completed and updates the transferred bytes count.
    pub fn mark_completed(&mut self, file_path: &str, bytes: u64) {
        self.pending_files.retain(|(l, _)| l != file_path);
        if !self.completed_files.contains(&file_path.to_string()) {
            self.completed_files.push(file_path.to_string());
            self.transferred_bytes = self.transferred_bytes.saturating_add(bytes);
        }
        self.touch();
    }

    /// Marks a file as failed.
    ///
    /// Moves the file from pending to failed.
    pub fn mark_failed(&mut self, file_path: &str) {
        self.pending_files.retain(|(l, _)| l != file_path);
        if !self.failed_files.contains(&file_path.to_string()) {
            self.failed_files.push(file_path.to_string());
        }
        self.touch();
    }

    /// Updates the `updated_at` timestamp to the current time.
    #[inline]
    fn touch(&mut self) {
        self.updated_at = current_timestamp();
    }

    /// Returns `true` if all files have been processed (completed or failed).
    #[inline]
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.pending_files.is_empty()
    }

    /// Returns the number of files still pending transfer.
    #[inline]
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.pending_files.len()
    }

    /// Returns the transfer progress as a percentage (0.0 to 100.0).
    #[inline]
    #[must_use]
    pub fn progress_percent(&self) -> f64 {
        if self.total_bytes == 0 {
            if self.is_complete() {
                100.0
            } else {
                0.0
            }
        } else {
            (self.transferred_bytes as f64 / self.total_bytes as f64) * 100.0
        }
    }

    /// Compute checksum of the state data (excluding the checksum field itself)
    fn compute_checksum(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.id.as_bytes());
        hasher.update(self.direction.as_bytes());
        hasher.update(self.total_files.to_le_bytes());
        for f in &self.completed_files {
            hasher.update(f.as_bytes());
        }
        for f in &self.failed_files {
            hasher.update(f.as_bytes());
        }
        for (a, b) in &self.pending_files {
            hasher.update(a.as_bytes());
            hasher.update(b.as_bytes());
        }
        hasher.update(self.total_bytes.to_le_bytes());
        hasher.update(self.transferred_bytes.to_le_bytes());
        hex::encode(hasher.finalize())
    }

    /// Save state to file with checksum for integrity validation
    pub fn save_to_file(&self, path: &str) -> Result<()> {
        let mut state_with_checksum = self.clone();
        state_with_checksum.checksum = Some(self.compute_checksum());
        let json = serde_json::to_string_pretty(&state_with_checksum)
            .map_err(|e| PCloudError::IoError(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load state from file with optional validation
    pub fn load_from_file(path: &str) -> Result<Self> {
        let json = std::fs::read_to_string(path)?;
        serde_json::from_str(&json)
            .map_err(|e| PCloudError::IoError(std::io::Error::new(std::io::ErrorKind::Other, e)))
    }

    /// Load and validate state file, attempting repair if corrupted
    pub fn load_and_validate(path: &str) -> Result<(Self, StateValidation)> {
        let json = std::fs::read_to_string(path)?;

        // Try to parse the JSON
        let state: Self = match serde_json::from_str(&json) {
            Ok(s) => s,
            Err(e) => {
                // Try to repair by extracting valid portions
                return Err(PCloudError::ApiError(format!(
                    "State file is corrupted and cannot be parsed: {e}"
                )));
            }
        };

        let validation = state.validate();
        Ok((state, validation))
    }

    /// Validate the state file integrity
    pub fn validate(&self) -> StateValidation {
        let mut issues = Vec::new();
        let mut can_repair = true;

        // Check checksum if present
        if let Some(ref stored_checksum) = self.checksum {
            let computed = self.compute_checksum();
            if &computed != stored_checksum {
                issues.push("Checksum mismatch - state file may be corrupted".to_string());
            }
        }

        // Check for logical consistency
        let total_accounted =
            self.completed_files.len() + self.failed_files.len() + self.pending_files.len();
        if total_accounted != self.total_files {
            issues.push(format!(
                "File count mismatch: total={}, accounted={}",
                self.total_files, total_accounted
            ));
        }

        // Check for duplicate entries
        let mut seen_completed: HashSet<&String> = HashSet::new();
        for f in &self.completed_files {
            if !seen_completed.insert(f) {
                issues.push(format!("Duplicate in completed_files: {f}"));
            }
        }

        let mut seen_failed: HashSet<&String> = HashSet::new();
        for f in &self.failed_files {
            if !seen_failed.insert(f) {
                issues.push(format!("Duplicate in failed_files: {f}"));
            }
        }

        // Check transferred bytes consistency
        if self.transferred_bytes > self.total_bytes && self.total_bytes > 0 {
            issues.push(format!(
                "Transferred bytes ({}) exceeds total bytes ({})",
                self.transferred_bytes, self.total_bytes
            ));
        }

        // Check direction is valid
        if self.direction != "upload" && self.direction != "download" {
            issues.push(format!("Invalid direction: {}", self.direction));
            can_repair = false;
        }

        // Check for valid UUID
        if uuid::Uuid::parse_str(&self.id).is_err() {
            issues.push("Invalid state ID (not a valid UUID)".to_string());
        }

        StateValidation {
            is_valid: issues.is_empty(),
            issues,
            can_repair,
        }
    }

    /// Attempt to repair a corrupted state file
    pub fn repair(&mut self) -> Vec<String> {
        let mut repairs = Vec::new();

        // Remove duplicates from completed_files
        let original_completed = self.completed_files.len();
        let mut seen: HashSet<String> = HashSet::new();
        self.completed_files.retain(|f| seen.insert(f.clone()));
        if self.completed_files.len() != original_completed {
            repairs.push(format!(
                "Removed {} duplicate entries from completed_files",
                original_completed - self.completed_files.len()
            ));
        }

        // Remove duplicates from failed_files
        let original_failed = self.failed_files.len();
        seen.clear();
        self.failed_files.retain(|f| seen.insert(f.clone()));
        if self.failed_files.len() != original_failed {
            repairs.push(format!(
                "Removed {} duplicate entries from failed_files",
                original_failed - self.failed_files.len()
            ));
        }

        // Fix total_files count
        let actual_total =
            self.completed_files.len() + self.failed_files.len() + self.pending_files.len();
        if actual_total != self.total_files {
            repairs.push(format!(
                "Corrected total_files from {} to {}",
                self.total_files, actual_total
            ));
            self.total_files = actual_total;
        }

        // Cap transferred_bytes at total_bytes
        if self.transferred_bytes > self.total_bytes && self.total_bytes > 0 {
            repairs.push(format!(
                "Capped transferred_bytes from {} to {}",
                self.transferred_bytes, self.total_bytes
            ));
            self.transferred_bytes = self.total_bytes;
        }

        // Regenerate UUID if invalid
        if uuid::Uuid::parse_str(&self.id).is_err() {
            let new_id = uuid::Uuid::new_v4().to_string();
            repairs.push(format!(
                "Regenerated invalid state ID: {} -> {}",
                self.id, new_id
            ));
            self.id = new_id;
        }

        // Update checksum
        self.checksum = Some(self.compute_checksum());
        repairs.push("Updated checksum".to_string());

        self.touch();

        repairs
    }

    /// Retry failed files by moving them back to pending
    pub fn retry_failed(&mut self) {
        // We need the original file pairs, so this only works if we track them
        // For now, we'll just clear the failed list - the caller should rebuild pending
        warn!(
            failed_count = self.failed_files.len(),
            "Clearing failed files for retry - caller must rebuild pending list"
        );
        self.failed_files.clear();
        self.touch();
    }
}

// =============================================================================
// Sync Operations
// =============================================================================

/// Result of a folder synchronization operation.
///
/// Contains statistics about what was transferred and lists of affected files.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncResult {
    /// Number of files uploaded to remote.
    pub uploaded: u32,
    /// Number of files downloaded from remote.
    pub downloaded: u32,
    /// Number of files skipped (already in sync).
    pub skipped: u32,
    /// Number of files that failed to transfer.
    pub failed: u32,
    /// List of local file paths that were uploaded.
    pub files_to_upload: Vec<String>,
    /// List of remote file paths that were downloaded.
    pub files_to_download: Vec<String>,
}

impl SyncResult {
    /// Returns the total number of files processed.
    #[inline]
    #[must_use]
    pub const fn total_processed(&self) -> u32 {
        self.uploaded + self.downloaded + self.skipped + self.failed
    }

    /// Returns `true` if all transfers succeeded.
    #[inline]
    #[must_use]
    pub const fn is_success(&self) -> bool {
        self.failed == 0
    }
}

/// Information about a file for synchronization comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncFileInfo {
    /// The file path (relative or absolute).
    pub path: String,
    /// File size in bytes.
    pub size: u64,
    /// Optional SHA-256 checksum for content comparison.
    pub checksum: Option<String>,
    /// Optional modification timestamp.
    pub modified: Option<String>,
}

// =============================================================================
// Error Handling
// =============================================================================

/// Errors that can occur during pCloud API operations.
///
/// This enum provides detailed, actionable error information for all
/// possible failure modes when interacting with the pCloud API.
///
/// # Example
///
/// ```rust,no_run
/// use pcloud_rust::{PCloudClient, PCloudError, Region};
///
/// # async fn example() {
/// let client = PCloudClient::new_adaptive(None, Region::US);
///
/// match client.list_folder("/test").await {
///     Ok(files) => println!("Found {} files", files.len()),
///     Err(PCloudError::NotAuthenticated) => {
///         eprintln!("Error: Please authenticate first");
///     }
///     Err(PCloudError::FileNotFound(path)) => {
///         eprintln!("Error: Path not found: {path}");
///     }
///     Err(e) => eprintln!("Error: {e}"),
/// }
/// # }
/// ```
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PCloudError {
    /// The pCloud API returned an error response.
    #[error("pCloud API error: {0}")]
    ApiError(String),

    /// A network error occurred during the HTTP request.
    #[error("network error: {0}")]
    NetworkError(#[from] reqwest::Error),

    /// An I/O error occurred reading or writing files.
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    /// The operation requires authentication, but no token is set.
    #[error("not authenticated: please call login() first")]
    NotAuthenticated,

    /// The provided path is invalid or malformed.
    #[error("invalid path: {0}")]
    InvalidPath(String),

    /// The requested file or folder does not exist.
    #[error("file not found: {0}")]
    FileNotFound(String),

    /// The operation timed out.
    #[error("operation timed out after {0:?}")]
    Timeout(Duration),

    /// The transfer was interrupted and can be resumed.
    #[error("transfer interrupted: {0} files remaining")]
    Interrupted(usize),
}

impl PCloudError {
    /// Returns `true` if this error is likely transient and retrying may succeed.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::NetworkError(_) | Self::Timeout(_) | Self::Interrupted(_)
        ) || matches!(self, Self::ApiError(s) if s.starts_with("HTTP error: 5"))
    }
}

/// A specialized [`Result`] type for pCloud operations.
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

/// Information about the authenticated pCloud account.
///
/// Retrieved via [`PCloudClient::get_account_info`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountInfo {
    /// The email address associated with the account.
    pub email: String,
    /// Total storage quota in bytes.
    pub quota: u64,
    /// Storage currently used in bytes.
    pub used_quota: u64,
    /// Whether the account has a premium subscription.
    pub premium: bool,
}

impl AccountInfo {
    /// Returns the available storage space in bytes.
    #[inline]
    #[must_use]
    pub const fn available(&self) -> u64 {
        self.quota.saturating_sub(self.used_quota)
    }

    /// Returns the storage usage as a percentage (0.0 to 100.0).
    #[inline]
    #[must_use]
    pub fn usage_percent(&self) -> f64 {
        if self.quota == 0 {
            0.0
        } else {
            (self.used_quota as f64 / self.quota as f64) * 100.0
        }
    }

    /// Returns `true` if the account is running low on storage (>90% used).
    #[inline]
    #[must_use]
    pub fn is_storage_low(&self) -> bool {
        self.usage_percent() > 90.0
    }

    /// Formats the quota as a human-readable string.
    #[must_use]
    pub fn format_quota(&self) -> String {
        format!(
            "{} / {} ({:.1}% used)",
            format_bytes(self.used_quota),
            format_bytes(self.quota),
            self.usage_percent()
        )
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

// =============================================================================
// Configuration Types
// =============================================================================

/// Configuration for automatic retry behavior on transient failures.
///
/// When network errors or server issues occur, the client can automatically
/// retry operations with exponential backoff.
///
/// # Example
///
/// ```rust
/// use pcloud_rust::RetryConfig;
///
/// let config = RetryConfig {
///     max_retries: 5,
///     initial_delay_ms: 1000,
///     max_delay_ms: 60_000,
///     backoff_multiplier: 2.0,
/// };
///
/// // Retry delays: 1s, 2s, 4s, 8s, 16s (capped at 60s)
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (0 = no retries).
    pub max_retries: u32,
    /// Initial delay before the first retry in milliseconds.
    pub initial_delay_ms: u64,
    /// Maximum delay between retries in milliseconds.
    pub max_delay_ms: u64,
    /// Multiplier applied to delay after each retry.
    pub backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay_ms: 500,
            max_delay_ms: 30_000,
            backoff_multiplier: 2.0,
        }
    }
}

impl RetryConfig {
    /// Creates a configuration with no retries.
    #[inline]
    #[must_use]
    pub const fn no_retries() -> Self {
        Self {
            max_retries: 0,
            initial_delay_ms: 0,
            max_delay_ms: 0,
            backoff_multiplier: 1.0,
        }
    }

    /// Creates an aggressive retry configuration suitable for unreliable networks.
    #[inline]
    #[must_use]
    pub const fn aggressive() -> Self {
        Self {
            max_retries: 10,
            initial_delay_ms: 100,
            max_delay_ms: 60_000,
            backoff_multiplier: 2.0,
        }
    }

    /// Calculates the delay for a given retry attempt.
    #[must_use]
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        if attempt == 0 {
            return Duration::ZERO;
        }
        let delay = self.initial_delay_ms as f64 * self.backoff_multiplier.powi(attempt as i32 - 1);
        Duration::from_millis((delay as u64).min(self.max_delay_ms))
    }
}

/// Configuration for per-file transfer timeouts.
///
/// Timeouts are calculated based on file size to accommodate larger files
/// that naturally take longer to transfer.
///
/// # Formula
///
/// ```text
/// timeout = min(base_timeout + (file_size_mb * secs_per_mb), max_timeout)
/// ```
///
/// # Example
///
/// ```rust
/// use pcloud_rust::FileTimeoutConfig;
///
/// let config = FileTimeoutConfig::default();
///
/// // 100 MB file: 60 + (100 * 2) = 260 seconds
/// let timeout = config.timeout_for_size(100 * 1024 * 1024);
/// assert_eq!(timeout.as_secs(), 260);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileTimeoutConfig {
    /// Base timeout in seconds for any file operation.
    pub base_timeout_secs: u64,
    /// Additional seconds per megabyte of file size.
    pub secs_per_mb: u64,
    /// Maximum timeout in seconds regardless of file size.
    pub max_timeout_secs: u64,
}

impl Default for FileTimeoutConfig {
    fn default() -> Self {
        Self {
            base_timeout_secs: 60,
            secs_per_mb: 2,
            max_timeout_secs: DEFAULT_FILE_TIMEOUT_SECS,
        }
    }
}

impl FileTimeoutConfig {
    /// Calculates the appropriate timeout for a file of the given size.
    #[must_use]
    pub fn timeout_for_size(&self, size_bytes: u64) -> Duration {
        let size_mb = size_bytes / (1024 * 1024);
        let timeout_secs = self
            .base_timeout_secs
            .saturating_add(size_mb.saturating_mul(self.secs_per_mb))
            .min(self.max_timeout_secs);
        Duration::from_secs(timeout_secs)
    }

    /// Creates a configuration optimized for fast networks.
    #[inline]
    #[must_use]
    pub const fn fast_network() -> Self {
        Self {
            base_timeout_secs: 30,
            secs_per_mb: 1,
            max_timeout_secs: 300,
        }
    }

    /// Creates a configuration optimized for slow or unreliable networks.
    #[inline]
    #[must_use]
    pub const fn slow_network() -> Self {
        Self {
            base_timeout_secs: 120,
            secs_per_mb: 5,
            max_timeout_secs: 3600,
        }
    }
}

/// Configuration for chunked uploads of large files.
///
/// Files exceeding the threshold are automatically uploaded in chunks,
/// which is more reliable for large files and allows progress tracking.
///
/// # Example
///
/// ```rust
/// use pcloud_rust::ChunkedUploadConfig;
///
/// let config = ChunkedUploadConfig {
///     threshold_bytes: 500 * 1024 * 1024,  // 500 MB
///     chunk_size: 5 * 1024 * 1024,          // 5 MB chunks
///     enabled: true,
/// };
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkedUploadConfig {
    /// File size threshold in bytes above which chunked uploads are used.
    pub threshold_bytes: u64,
    /// Size of each chunk in bytes.
    pub chunk_size: u64,
    /// Whether chunked uploads are enabled.
    pub enabled: bool,
}

impl Default for ChunkedUploadConfig {
    fn default() -> Self {
        Self {
            threshold_bytes: LARGE_FILE_THRESHOLD,
            chunk_size: DEFAULT_CHUNK_SIZE,
            enabled: true,
        }
    }
}

impl ChunkedUploadConfig {
    /// Creates a disabled chunked upload configuration.
    #[inline]
    #[must_use]
    pub const fn disabled() -> Self {
        Self {
            threshold_bytes: u64::MAX,
            chunk_size: DEFAULT_CHUNK_SIZE,
            enabled: false,
        }
    }

    /// Returns the number of chunks needed for a file of the given size.
    #[must_use]
    pub const fn chunks_for_size(&self, size_bytes: u64) -> u64 {
        if self.chunk_size == 0 {
            return 1;
        }
        (size_bytes + self.chunk_size - 1) / self.chunk_size
    }
}

// =============================================================================
// Client
// =============================================================================

/// The main pCloud API client.
///
/// `PCloudClient` is the primary interface for interacting with pCloud storage.
/// It handles authentication, file transfers, folder operations, and synchronization.
///
/// # Thread Safety
///
/// `PCloudClient` implements `Clone` and can be safely shared across threads.
/// Each clone shares the underlying HTTP connection pool.
///
/// # Example
///
/// ```rust,no_run
/// use pcloud_rust::{PCloudClient, Region, DuplicateMode};
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     // Create a client with adaptive worker count
///     let mut client = PCloudClient::new_adaptive(None, Region::US);
///
///     // Configure duplicate handling
///     client.set_duplicate_mode(DuplicateMode::Skip);
///
///     // Authenticate
///     client.login("user@example.com", "password").await?;
///
///     // Use the client...
///     let files = client.list_folder("/").await?;
///     println!("Root contains {} items", files.len());
///
///     Ok(())
/// }
/// ```
#[derive(Clone)]
pub struct PCloudClient {
    /// The underlying HTTP client.
    client: Client,
    /// The API region to connect to.
    region: Region,
    /// The authentication token (set after login).
    auth_token: Option<String>,
    /// Number of concurrent workers for parallel operations.
    pub workers: usize,
    /// Strategy for handling duplicate files.
    pub duplicate_mode: DuplicateMode,
    /// Configuration for automatic retries.
    pub retry_config: RetryConfig,
    /// Configuration for per-file timeouts.
    pub file_timeout_config: FileTimeoutConfig,
    /// Configuration for chunked uploads.
    pub chunked_upload_config: ChunkedUploadConfig,
}

impl fmt::Debug for PCloudClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PCloudClient")
            .field("region", &self.region)
            .field("authenticated", &self.auth_token.is_some())
            .field("workers", &self.workers)
            .field("duplicate_mode", &self.duplicate_mode)
            .finish_non_exhaustive()
    }
}

impl PCloudClient {
    /// Creates a new pCloud client with the specified configuration.
    ///
    /// # Arguments
    ///
    /// * `token` - Optional authentication token from a previous session
    /// * `region` - The pCloud API region to connect to
    /// * `workers` - Number of concurrent workers for parallel operations (clamped to 1–32)
    ///
    /// # Example
    ///
    /// ```rust
    /// use pcloud_rust::{PCloudClient, Region};
    ///
    /// // Create a client for the EU region with 16 workers
    /// let client = PCloudClient::new(None, Region::EU, 16);
    /// ```
    #[must_use]
    pub fn new(token: Option<String>, region: Region, workers: usize) -> Self {
        let workers = workers.clamp(MIN_WORKERS, MAX_WORKERS);

        let client = Client::builder()
            .pool_max_idle_per_host(workers)
            .pool_idle_timeout(Some(Duration::from_secs(90)))
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(300))
            .build()
            .unwrap_or_default();

        Self {
            client,
            region,
            auth_token: token,
            workers,
            duplicate_mode: DuplicateMode::default(),
            retry_config: RetryConfig::default(),
            file_timeout_config: FileTimeoutConfig::default(),
            chunked_upload_config: ChunkedUploadConfig::default(),
        }
    }

    /// Creates a new client with adaptive worker count based on system resources.
    ///
    /// The optimal worker count is calculated based on available CPU cores
    /// and system memory. This is the recommended constructor for most use cases.
    ///
    /// # Algorithm
    ///
    /// ```text
    /// cpu_based    = cpu_cores × 2 (I/O-bound tasks benefit from more workers)
    /// memory_based = available_memory_gb × 20 (~50 MB per worker)
    /// workers      = min(cpu_based, memory_based), clamped to 1–32
    /// ```
    ///
    /// # Example
    ///
    /// ```rust
    /// use pcloud_rust::{PCloudClient, Region};
    ///
    /// let client = PCloudClient::new_adaptive(None, Region::US);
    /// println!("Using {} workers", client.workers);
    /// ```
    #[must_use]
    pub fn new_adaptive(token: Option<String>, region: Region) -> Self {
        let workers = Self::calculate_adaptive_workers();
        debug!(workers, "Created client with adaptive worker count");
        Self::new(token, region, workers)
    }

    /// Calculates the optimal worker count for the current system.
    ///
    /// This can be called independently to preview the worker count that
    /// [`new_adaptive`](Self::new_adaptive) would use.
    ///
    /// # Returns
    ///
    /// The recommended number of workers, between 1 and 32.
    #[must_use]
    pub fn calculate_adaptive_workers() -> usize {
        let mut sys = System::new();
        sys.refresh_cpu_all();
        sys.refresh_memory();

        let cpu_cores = sys.cpus().len().max(1);
        let available_memory_gb = sys.available_memory() as f64 / (1024.0 * 1024.0 * 1024.0);

        // I/O-bound tasks benefit from 2× CPU cores
        let cpu_based = cpu_cores.saturating_mul(2);

        // ~50 MB per worker to stay within memory limits
        let memory_based = (available_memory_gb * 20.0) as usize;

        let workers = cpu_based.min(memory_based).clamp(MIN_WORKERS, MAX_WORKERS);

        debug!(
            cpu_cores,
            available_memory_gb = format!("{available_memory_gb:.2}"),
            workers,
            "Calculated adaptive worker count"
        );

        workers
    }

    /// Sets the retry configuration for transient failures.
    #[inline]
    pub fn set_retry_config(&mut self, config: RetryConfig) {
        self.retry_config = config;
    }

    /// Disables automatic retries for all operations.
    #[inline]
    pub fn disable_retries(&mut self) {
        self.retry_config = RetryConfig::no_retries();
    }

    /// Sets the authentication token.
    ///
    /// This is typically called automatically by [`login`](Self::login),
    /// but can be used to restore a token from a previous session.
    #[inline]
    pub fn set_token(&mut self, token: impl Into<String>) {
        self.auth_token = Some(token.into());
    }

    /// Returns the current authentication token, if set.
    #[inline]
    #[must_use]
    pub fn token(&self) -> Option<&str> {
        self.auth_token.as_deref()
    }

    /// Returns `true` if the client has an authentication token.
    #[inline]
    #[must_use]
    pub fn is_authenticated(&self) -> bool {
        self.auth_token.is_some()
    }

    /// Sets the duplicate file handling strategy.
    #[inline]
    pub fn set_duplicate_mode(&mut self, mode: DuplicateMode) {
        self.duplicate_mode = mode;
    }

    /// Sets the per-file timeout configuration.
    #[inline]
    pub fn set_file_timeout_config(&mut self, config: FileTimeoutConfig) {
        self.file_timeout_config = config;
    }

    /// Sets the chunked upload configuration.
    #[inline]
    pub fn set_chunked_upload_config(&mut self, config: ChunkedUploadConfig) {
        self.chunked_upload_config = config;
    }

    /// Returns the API region this client is configured for.
    #[inline]
    #[must_use]
    pub const fn region(&self) -> Region {
        self.region
    }

    /// Constructs the full URL for an API method.
    #[inline]
    fn api_url(&self, method: &str) -> String {
        format!("{}/{}", self.region.endpoint(), method)
    }

    /// Returns the authentication token or an error if not authenticated.
    #[inline]
    fn require_auth(&self) -> Result<&str> {
        self.auth_token
            .as_deref()
            .ok_or(PCloudError::NotAuthenticated)
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

    /// Authenticates with pCloud using username and password.
    ///
    /// On success, the authentication token is stored in the client and
    /// returned for optional persistence across sessions.
    ///
    /// # Arguments
    ///
    /// * `username` - The pCloud account email address
    /// * `password` - The account password
    ///
    /// # Returns
    ///
    /// The authentication token on success, which can be saved and passed
    /// to [`new`](Self::new) or [`set_token`](Self::set_token) later.
    ///
    /// # Errors
    ///
    /// Returns an error if authentication fails or a network error occurs.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use pcloud_rust::{PCloudClient, Region};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut client = PCloudClient::new_adaptive(None, Region::US);
    /// let token = client.login("user@example.com", "password").await?;
    ///
    /// // Save token for later use
    /// std::fs::write(".pcloud_token", &token)?;
    /// # Ok(())
    /// # }
    /// ```
    #[instrument(skip(self, password), fields(username = %username))]
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
            .ok_or_else(|| PCloudError::ApiError("no auth token in response".into()))?;

        self.auth_token = Some(token.clone());
        info!("Successfully authenticated");
        Ok(token)
    }

    // =========================================================================
    // Folder Operations
    // =========================================================================

    /// Creates a folder at the specified path.
    ///
    /// If the folder already exists, this operation succeeds silently.
    /// Parent directories are created automatically if needed.
    ///
    /// # Errors
    ///
    /// Returns an error if not authenticated or the path is invalid.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use pcloud_rust::{PCloudClient, Region};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = PCloudClient::new_adaptive(None, Region::US);
    /// client.create_folder("/Backups/2024/January").await?;
    /// # Ok(())
    /// # }
    /// ```
    #[instrument(skip(self), fields(path = %path))]
    pub async fn create_folder(&self, path: &str) -> Result<()> {
        let url = self.api_url("createfolderifnotexists");
        let auth = self.require_auth()?;
        let params = [("auth", auth), ("path", path)];

        let api_resp: ApiResponse = self.with_retry(|| self.api_get(&url, &params)).await?;

        // 2004 = folder already exists, which is fine
        if api_resp.result == 0 || api_resp.result == 2004 {
            Ok(())
        } else {
            Self::ensure_success(&api_resp)
        }
    }

    /// Lists the contents of a folder.
    ///
    /// Returns a list of files and subfolders in the specified directory.
    /// The list is not recursive—only immediate children are returned.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use pcloud_rust::{PCloudClient, Region};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = PCloudClient::new_adaptive(None, Region::US);
    ///
    /// let items = client.list_folder("/Documents").await?;
    /// for item in items {
    ///     if item.isfolder {
    ///         println!("📁 {}", item.name);
    ///     } else {
    ///         println!("📄 {} ({} bytes)", item.name, item.size);
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[instrument(skip(self), fields(path = %path))]
    pub async fn list_folder(&self, path: &str) -> Result<Vec<FileItem>> {
        let url = self.api_url("listfolder");
        let auth = self.require_auth()?;

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

    /// Deletes a file.
    ///
    /// # Warning
    ///
    /// This operation is irreversible unless the file is in the trash.
    ///
    /// # Errors
    ///
    /// Returns an error if the file doesn't exist or you don't have permission.
    #[instrument(skip(self), fields(path = %path))]
    pub async fn delete_file(&self, path: &str) -> Result<()> {
        let url = self.api_url("deletefile");
        let auth = self.require_auth()?;
        let params = [("auth", auth), ("path", path)];
        let api_resp: ApiResponse = self.with_retry(|| self.api_get(&url, &params)).await?;
        Self::ensure_success(&api_resp)
    }

    /// Deletes a folder and all its contents recursively.
    ///
    /// # Warning
    ///
    /// This operation is irreversible and will delete all files and subfolders.
    ///
    /// # Errors
    ///
    /// Returns an error if the folder doesn't exist or you don't have permission.
    #[instrument(skip(self), fields(path = %path))]
    pub async fn delete_folder(&self, path: &str) -> Result<()> {
        let url = self.api_url("deletefolderrecursive");
        let auth = self.require_auth()?;
        let params = [("auth", auth), ("path", path)];
        let api_resp: ApiResponse = self.with_retry(|| self.api_get(&url, &params)).await?;
        Self::ensure_success(&api_resp)
    }

    /// Renames or moves a file.
    ///
    /// # Arguments
    ///
    /// * `from_path` - Current path of the file
    /// * `to_path` - New path for the file
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use pcloud_rust::{PCloudClient, Region};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = PCloudClient::new_adaptive(None, Region::US);
    ///
    /// // Rename a file
    /// client.rename_file("/old-name.txt", "/new-name.txt").await?;
    ///
    /// // Move a file to another folder
    /// client.rename_file("/file.txt", "/Archive/file.txt").await?;
    /// # Ok(())
    /// # }
    /// ```
    #[instrument(skip(self), fields(from = %from_path, to = %to_path))]
    pub async fn rename_file(&self, from_path: &str, to_path: &str) -> Result<()> {
        let url = self.api_url("renamefile");
        let auth = self.require_auth()?;
        let params = [("auth", auth), ("path", from_path), ("topath", to_path)];
        let api_resp: ApiResponse = self.with_retry(|| self.api_get(&url, &params)).await?;
        Self::ensure_success(&api_resp)
    }

    /// Renames or moves a folder.
    ///
    /// All contents of the folder are moved with it.
    #[instrument(skip(self), fields(from = %from_path, to = %to_path))]
    pub async fn rename_folder(&self, from_path: &str, to_path: &str) -> Result<()> {
        let url = self.api_url("renamefolder");
        let auth = self.require_auth()?;
        let params = [("auth", auth), ("path", from_path), ("topath", to_path)];
        let api_resp: ApiResponse = self.with_retry(|| self.api_get(&url, &params)).await?;
        Self::ensure_success(&api_resp)
    }

    /// Retrieves account information including storage quota.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use pcloud_rust::{PCloudClient, Region};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = PCloudClient::new_adaptive(None, Region::US);
    ///
    /// let info = client.get_account_info().await?;
    /// println!("Account: {}", info.email);
    /// println!("Used: {} / {} bytes", info.used_quota, info.quota);
    /// println!("Plan: {}", if info.premium { "Premium" } else { "Free" });
    /// # Ok(())
    /// # }
    /// ```
    #[instrument(skip(self))]
    pub async fn get_account_info(&self) -> Result<AccountInfo> {
        let url = self.api_url("userinfo");
        let auth = self.require_auth()?;
        let params = [("auth", auth)];

        let api_resp: AccountInfoResponse = self.with_retry(|| self.api_get(&url, &params)).await?;

        if api_resp.result != 0 {
            return Err(PCloudError::ApiError(
                api_resp.error.unwrap_or_else(|| "unknown error".into()),
            ));
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
                    return Ok(format!("https://{host}{p}"));
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
                            format!("/{filename}")
                        } else {
                            format!("{}/{}", remote_path.trim_end_matches('/'), filename)
                        };
                        let temp_remote = if remote_path == "/" {
                            format!("/{temp_filename}")
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

    /// Upload a file with per-file timeout based on file size
    pub async fn upload_file_with_timeout(
        &self,
        local_path: &str,
        remote_path: &str,
    ) -> Result<()> {
        let file_size = std::fs::metadata(local_path).map(|m| m.len()).unwrap_or(0);
        let timeout = self.file_timeout_config.timeout_for_size(file_size);

        match tokio::time::timeout(timeout, self.upload_file(local_path, remote_path)).await {
            Ok(result) => result,
            Err(_) => Err(PCloudError::ApiError(format!(
                "Upload timed out after {timeout:?} for file: {local_path}"
            ))),
        }
    }

    /// Upload a large file in chunks (for files > 2GB)
    /// Uses pCloud's upload_save API for chunked uploads
    pub async fn upload_large_file_chunked<F>(
        &self,
        local_path: &str,
        remote_path: &str,
        mut progress_callback: F,
    ) -> Result<()>
    where
        F: FnMut(u64, u64) + Send + Sync + 'static,
    {
        let path = Path::new(local_path);
        if !path.exists() {
            return Err(PCloudError::FileNotFound(local_path.to_string()));
        }

        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| PCloudError::InvalidPath("Invalid filename".to_string()))?;

        let file_size = std::fs::metadata(local_path)
            .map(|m| m.len())
            .map_err(PCloudError::IoError)?;

        // For files under the threshold, use regular upload
        if file_size < self.chunked_upload_config.threshold_bytes
            || !self.chunked_upload_config.enabled
        {
            return self.upload_file(local_path, remote_path).await;
        }

        let auth = self
            .auth_token
            .as_deref()
            .ok_or(PCloudError::NotAuthenticated)?;

        // Create upload session
        let create_url = self.api_url("upload_create");
        let create_params = [("auth", auth)];

        #[derive(Deserialize)]
        struct CreateResponse {
            result: i32,
            uploadid: Option<u64>,
            #[serde(default)]
            error: Option<String>,
        }

        let create_resp: CreateResponse = self.api_get(&create_url, &create_params).await?;
        if create_resp.result != 0 {
            return Err(PCloudError::ApiError(
                create_resp
                    .error
                    .unwrap_or_else(|| "Failed to create upload session".into()),
            ));
        }

        let upload_id = create_resp
            .uploadid
            .ok_or_else(|| PCloudError::ApiError("No upload ID returned".into()))?;

        // Upload chunks
        let mut file = tokio::fs::File::open(local_path).await?;
        let chunk_size = self.chunked_upload_config.chunk_size;
        let mut offset: u64 = 0;
        let mut buffer = vec![0u8; chunk_size as usize];

        while offset < file_size {
            let bytes_to_read = ((file_size - offset) as usize).min(chunk_size as usize);
            let bytes_read = file.read(&mut buffer[..bytes_to_read]).await?;

            if bytes_read == 0 {
                break;
            }

            // Upload this chunk
            let write_url = self.api_url("upload_write");
            let upload_id_str = upload_id.to_string();
            let offset_str = offset.to_string();

            let chunk_data = buffer[..bytes_read].to_vec();
            let part = multipart::Part::bytes(chunk_data)
                .file_name("chunk")
                .mime_str("application/octet-stream")
                .map_err(|e| PCloudError::ApiError(e.to_string()))?;

            let form = multipart::Form::new().part("file", part);

            let response = self
                .client
                .post(&write_url)
                .query(&[
                    ("auth", auth),
                    ("uploadid", &upload_id_str),
                    ("uploadoffset", &offset_str),
                ])
                .multipart(form)
                .send()
                .await?;

            let write_resp: ApiResponse = response.json().await?;
            if write_resp.result != 0 {
                // Abort the upload on error
                let _ = self
                    .client
                    .get(self.api_url("upload_cancel"))
                    .query(&[("auth", auth), ("uploadid", &upload_id_str)])
                    .send()
                    .await;
                return Err(PCloudError::ApiError(
                    write_resp
                        .error
                        .unwrap_or_else(|| "Chunk upload failed".into()),
                ));
            }

            offset += bytes_read as u64;
            progress_callback(offset, file_size);
        }

        // Save/finalize the upload
        let save_url = self.api_url("upload_save");
        let upload_id_str = upload_id.to_string();

        let save_params = [
            ("auth", auth),
            ("uploadid", &upload_id_str),
            ("path", remote_path),
            ("name", filename),
        ];

        let save_resp: ApiResponse = self.api_get(&save_url, &save_params).await?;
        Self::ensure_success(&save_resp)?;

        info!(
            file = local_path,
            size = file_size,
            chunks = (file_size + chunk_size - 1) / chunk_size,
            "Large file upload completed"
        );

        Ok(())
    }

    /// Upload files with per-file timeout and automatic retry
    pub async fn upload_files_with_timeout(
        &self,
        tasks: Vec<(String, String)>,
        max_retries_per_file: u32,
    ) -> (u32, u32, Vec<(String, String)>) {
        let mut uploaded = 0u32;
        let mut failed = 0u32;
        let mut failed_tasks: Vec<(String, String)> = Vec::new();

        let uploads = stream::iter(tasks)
            .map(|(local_path, remote_folder)| {
                let client = self.clone();
                let max_retries = max_retries_per_file;
                async move {
                    let mut attempts = 0;
                    let mut last_error = None;

                    while attempts <= max_retries {
                        match client
                            .upload_file_with_timeout(&local_path, &remote_folder)
                            .await
                        {
                            Ok(()) => return (local_path, remote_folder, true, None),
                            Err(e) => {
                                attempts += 1;
                                last_error = Some(e.to_string());

                                if attempts <= max_retries {
                                    // Exponential backoff between retries
                                    let delay = std::time::Duration::from_millis(
                                        500 * (2u64.pow(attempts - 1)),
                                    );
                                    tokio::time::sleep(delay).await;
                                }
                            }
                        }
                    }

                    (local_path, remote_folder, false, last_error)
                }
            })
            .buffer_unordered(self.workers);

        let results: Vec<_> = uploads.collect().await;

        for (local_path, remote_folder, success, error) in results {
            if success {
                uploaded += 1;
            } else {
                failed += 1;
                if let Some(err) = error {
                    warn!(
                        file = %local_path,
                        error = %err,
                        "File upload failed after retries"
                    );
                }
                failed_tasks.push((local_path, remote_folder));
            }
        }

        (uploaded, failed, failed_tasks)
    }

    /// Download a file with per-file timeout based on expected size
    pub async fn download_file_with_timeout(
        &self,
        remote_path: &str,
        local_folder: &str,
        expected_size: Option<u64>,
    ) -> Result<String> {
        let size = expected_size.unwrap_or(100 * 1024 * 1024); // Default 100MB estimate
        let timeout = self.file_timeout_config.timeout_for_size(size);

        match tokio::time::timeout(timeout, self.download_file(remote_path, local_folder)).await {
            Ok(result) => result,
            Err(_) => Err(PCloudError::ApiError(format!(
                "Download timed out after {timeout:?} for file: {remote_path}"
            ))),
        }
    }

    /// Download files with per-file timeout and automatic retry
    pub async fn download_files_with_timeout(
        &self,
        tasks: Vec<(String, String)>,
        max_retries_per_file: u32,
    ) -> (u32, u32, Vec<(String, String)>) {
        let mut downloaded = 0u32;
        let mut failed = 0u32;
        let mut failed_tasks: Vec<(String, String)> = Vec::new();

        let downloads = stream::iter(tasks)
            .map(|(remote_path, local_folder)| {
                let client = self.clone();
                let max_retries = max_retries_per_file;
                async move {
                    let mut attempts = 0;
                    let mut last_error = None;

                    while attempts <= max_retries {
                        match client
                            .download_file_with_timeout(&remote_path, &local_folder, None)
                            .await
                        {
                            Ok(_) => return (remote_path, local_folder, true, None),
                            Err(e) => {
                                attempts += 1;
                                last_error = Some(e.to_string());

                                if attempts <= max_retries {
                                    let delay = std::time::Duration::from_millis(
                                        500 * (2u64.pow(attempts - 1)),
                                    );
                                    tokio::time::sleep(delay).await;
                                }
                            }
                        }
                    }

                    (remote_path, local_folder, false, last_error)
                }
            })
            .buffer_unordered(self.workers);

        let results: Vec<_> = downloads.collect().await;

        for (remote_path, local_folder, success, error) in results {
            if success {
                downloaded += 1;
            } else {
                failed += 1;
                if let Some(err) = error {
                    warn!(
                        file = %remote_path,
                        error = %err,
                        "File download failed after retries"
                    );
                }
                failed_tasks.push((remote_path, local_folder));
            }
        }

        (downloaded, failed, failed_tasks)
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
                format!("/{folder_name}/{relative_str}")
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

        let mut depths: Vec<usize> = folders_by_depth.keys().copied().collect();
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
