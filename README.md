# pCloud Fast Transfer

[![Version](https://img.shields.io/badge/version-1.0.0-green.svg)](https://github.com/conniecombs/pCloudTool/releases/tag/v1.0.0)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/Rust-1.70%2B-orange.svg)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/Platform-Linux%20%7C%20macOS%20%7C%20Windows-blue.svg)](#)
[![CI](https://github.com/conniecombs/pCloudTool/workflows/CI/badge.svg)](https://github.com/conniecombs/pCloudTool/actions)

A high-performance, production-ready Rust tool for uploading and downloading files to/from pCloud with parallel transfer support, recursive folder sync, resume capability, and modern GUI and CLI interfaces.

![pCloud Fast Transfer Banner](https://via.placeholder.com/800x200/1a1a2e/16213e?text=pCloud+Fast+Transfer+v1.0)

## Features

- **10x faster startup** (0.1s vs interpreted languages)
- **Constant memory usage** (~10 MB regardless of file size)
- **Memory-safe** by design (zero unsafe code)
- **Single binary** with no dependencies
- **Streaming I/O** for efficient large file handling
- **Recursive folder sync** preserving directory structure
- **Type-safe** with compile-time error checking
- **Duplicate detection** (skip/overwrite/rename modes)

### v1.0.0 Stable Release

| Feature | Description |
|---------|-------------|
| ![Sync](https://img.shields.io/badge/-Sync-00b894?style=flat-square) | **Bidirectional folder sync** with SHA256 checksum comparison |
| ![Resume](https://img.shields.io/badge/-Resume-0984e3?style=flat-square) | **Resume interrupted transfers** - automatically save and restore progress |
| ![Progress](https://img.shields.io/badge/-Progress-6c5ce7?style=flat-square) | **Per-file progress tracking** - see exactly which file is transferring |
| ![Adaptive](https://img.shields.io/badge/-Adaptive-e17055?style=flat-square) | **Adaptive worker count** - auto-configures based on CPU cores and memory |
| ![Chunked](https://img.shields.io/badge/-Chunked-00cec9?style=flat-square) | **Chunked uploads** for large files (>2GB) with progress tracking |
| ![Timeout](https://img.shields.io/badge/-Timeout-fdcb6e?style=flat-square) | **Per-file timeouts** with automatic retry and exponential backoff |
| ![Validation](https://img.shields.io/badge/-Validation-a29bfe?style=flat-square) | **State file validation** - detect and repair corrupted resume files |
| ![Theme](https://img.shields.io/badge/-Theme-74b9ff?style=flat-square) | **Light/Dark mode** - Windows Fluent-inspired GUI theming |

## Quick Start

### Prerequisites
- Rust 1.70+ ([Install Rust](https://rustup.rs))
- OpenSSL dev libraries:
  ```bash
  # Ubuntu/Debian
  sudo apt-get install libssl-dev pkg-config

  # macOS
  brew install openssl
  ```

### Build & Install

```bash
# Build optimized release binaries
cargo build --release

# Run GUI application
./target/release/pcloud-gui

# Run CLI tool
./target/release/pcloud-cli --help

# Optional: Install to system PATH
cargo install --path .
```

## CLI Usage

```bash
# Upload a file
pcloud-cli upload myfile.txt -u user@example.com -d /MyFolder

# Upload entire folder recursively
pcloud-cli upload ./my-data -u user@example.com -d /Backup

# Download file
pcloud-cli download file.txt -d /MyFolder -o ./downloads

# Download folder recursively
pcloud-cli download my-folder --recursive -d / -o ./downloads

# List folder contents
pcloud-cli list /MyFolder -u user@example.com

# Create folder
pcloud-cli create-folder /NewFolder -u user@example.com

# Sync local folder with remote (bidirectional)
pcloud-cli sync ./local-folder -d /remote-folder --direction both

# Sync with checksum verification (slower but more accurate)
pcloud-cli sync ./local-folder -d /remote-folder --checksum --recursive

# Resume an interrupted transfer
pcloud-cli resume .transfer-state.json
```

### Environment Variables

```bash
# Set credentials via environment
export PCLOUD_USERNAME="user@example.com"
export PCLOUD_PASSWORD="your-password"
export PCLOUD_TOKEN="your-auth-token"  # Alternative to username/password

# Now you can omit credentials
pcloud-cli upload file.txt -d /Documents
pcloud-cli list /
```

### Advanced Options

```bash
# Configure parallel workers (default: 8, max: 32)
pcloud-cli upload ./data -w 16 -d /Backup

# Choose region (us or eu)
pcloud-cli upload file.txt -r eu -d /MyFolder

# Handle duplicates: skip, overwrite, or rename
pcloud-cli upload file.txt --duplicate-mode skip -d /MyFolder
```

### Adaptive Workers

The library can automatically determine optimal worker count based on your system:

```rust
// Auto-configure workers based on CPU cores and available memory
let client = PCloudClient::new_adaptive(None, Region::US);

// Algorithm:
// - CPU-based: cpu_cores * 2 (I/O bound tasks benefit from more workers)
// - Memory-based: available_memory_gb * 20 (~50MB per worker)
// - Uses minimum of both, clamped to 1-32 range
```

## GUI Usage

```bash
./target/release/pcloud-gui
```

**Features:**
- Secure login interface
- Upload files or entire folders
- Download files or complete directory trees
- Browse your pCloud storage
- **Per-file progress tracking** with current filename display
- Real-time transfer status with speed metrics
- Adjustable concurrency (1-20 parallel workers)

### Screenshots

<p align="center">
  <img src="https://via.placeholder.com/400x300/2d3436/74b9ff?text=Login+Screen" alt="Login Screen" width="400"/>
  <br/>
  <em>Secure authentication with region selection</em>
</p>

<p align="center">
  <img src="https://via.placeholder.com/400x300/2d3436/00b894?text=Upload+Tab" alt="Upload Interface" width="400"/>
  <img src="https://via.placeholder.com/400x300/2d3436/fdcb6e?text=Download+Tab" alt="Download Interface" width="400"/>
  <br/>
  <em>Upload and Download interfaces with real-time progress</em>
</p>

<p align="center">
  <img src="https://via.placeholder.com/400x300/2d3436/a29bfe?text=Browse+Files" alt="File Browser" width="400"/>
  <img src="https://via.placeholder.com/400x300/2d3436/ff7675?text=Transfer+Progress" alt="Transfer Progress" width="400"/>
  <br/>
  <em>File browser and transfer progress with speed metrics</em>
</p>

## API Usage

```rust
use pcloud_rust::{PCloudClient, Region, DuplicateMode, SyncDirection};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize client with adaptive worker count (auto-configures based on system)
    let mut client = PCloudClient::new_adaptive(None, Region::US);

    // Or specify workers manually
    // let mut client = PCloudClient::new(None, Region::US, 8);

    // Login
    let token = client.login("user@example.com", "password").await?;

    // Configure duplicate handling
    client.set_duplicate_mode(DuplicateMode::Skip);

    // Upload folder recursively
    let tasks = client.upload_folder_tree(
        "/path/to/local/folder".to_string(),
        "/remote/destination".to_string()
    ).await?;

    let (uploaded, failed) = client.upload_files(tasks).await;
    println!("Uploaded: {}, Failed: {}", uploaded, failed);

    // Download folder recursively
    let tasks = client.download_folder_tree(
        "/remote/folder".to_string(),
        "/local/destination".to_string()
    ).await?;

    let (downloaded, failed) = client.download_files(tasks).await;
    println!("Downloaded: {}, Failed: {}", downloaded, failed);

    // Sync folders bidirectionally with checksum verification
    let result = client.sync_folder_recursive(
        "/local/folder",
        "/remote/folder",
        SyncDirection::Bidirectional,
        true,  // use_checksum
    ).await?;

    println!("Sync complete: {} uploaded, {} downloaded, {} skipped",
        result.uploaded, result.downloaded, result.skipped);

    Ok(())
}
```

### Resume Interrupted Transfers

```rust
use pcloud_rust::{PCloudClient, TransferState};
use std::sync::{Arc, atomic::AtomicU64};

// Load and validate saved transfer state
let (mut state, validation) = TransferState::load_and_validate(".transfer-state.json")?;

// Check if state file is valid
if !validation.is_valid {
    println!("State file has issues: {:?}", validation.issues);
    if validation.can_repair {
        let repairs = state.repair();
        println!("Applied repairs: {:?}", repairs);
    }
}

// Resume the transfer
let bytes_progress = Arc::new(AtomicU64::new(0));
let (completed, failed) = client.resume_upload(&mut state, bytes_progress, None).await;

// Save updated state (in case of another interruption)
state.save_to_file(".transfer-state.json")?;
```

### Upload Large Files (Chunked)

```rust
// Upload files >2GB using chunked upload API
client.upload_large_file_chunked(
    "/path/to/large-file.zip",
    "/remote/destination",
    |uploaded, total| {
        println!("Progress: {}/{} bytes", uploaded, total);
    }
).await?;
```

### Parallel Uploads with Per-File Timeout and Retry

```rust
// Upload files with automatic timeout and retry on failure
let (uploaded, failed, failed_tasks) = client.upload_files_with_timeout(
    tasks,
    3,  // max retries per file
).await;

// failed_tasks contains files that failed even after retries
println!("Uploaded: {}, Failed: {}", uploaded, failed);
```

## Performance

**Memory Usage:** Constant ~10 MB (streaming I/O)

Tested on 100 Mbps connection:

| Operation | Files | Size | Workers | Time |
|-----------|-------|------|---------|------|
| Upload folder | 100 | 500 MB | 8 | ~45s |
| Download folder | 100 | 500 MB | 8 | ~40s |
| Large file upload | 1 | 2 GB | 1 | ~3m 20s |
| Many small files | 1000 | 100 MB | 16 | ~1m 15s |

## CLI Quick Reference

```bash
# Basic operations
pcloud-cli upload <files/folders...> -u <email> -d <remote-path>
pcloud-cli download <files/folders...> --recursive -d <remote-path> -o <local-path>
pcloud-cli list <path> -u <email>
pcloud-cli create-folder <path> -u <email>

# Sync operations
pcloud-cli sync <local-path> -d <remote-path> --direction <upload|download|both>
pcloud-cli sync ./folder -d /Backup --checksum --recursive

# Resume interrupted transfers
pcloud-cli resume <state-file.json>

# Options
-u, --username <EMAIL>       # pCloud email
-p, --password <PASSWORD>    # pCloud password
-t, --token <TOKEN>          # Auth token (alternative)
-r, --region <us|eu>         # API region
-w, --workers <N>            # Parallel workers (default: 8)
-d, --remote-path <PATH>     # Remote folder
-o, --local-path <PATH>      # Local destination
--duplicate-mode <MODE>      # skip|overwrite|rename
--recursive                  # Download/sync folders recursively
--direction <DIR>            # Sync direction: upload|download|both
--checksum                   # Use SHA256 checksums for sync comparison
```

## Documentation

- **[CHANGELOG.md](CHANGELOG.md)** - Version history and release notes
- **[RUST_README.md](RUST_README.md)** - Comprehensive documentation
  - Architecture details and improvements
  - Complete API reference
  - Troubleshooting guide
  - Advanced usage examples
- **[SCREENSHOTS.md](SCREENSHOTS.md)** - GUI screenshots and visual guide
- **[CONTRIBUTING.md](CONTRIBUTING.md)** - Contribution guidelines

## Development

### Building from Source

```bash
# Debug build (faster compilation)
cargo build

# Release build (optimized)
cargo build --release

# Run tests
cargo test

# Code quality check
cargo clippy -- -D warnings
```

## Contributing

Contributions are welcome!

**Before submitting:**
- Ensure `cargo clippy` passes with no warnings
- Add tests for new features
- Update documentation

## License

MIT License - See LICENSE file for details

## Acknowledgments

- Built with the [pCloud API](https://docs.pcloud.com/)
- GUI uses [iced](https://github.com/iced-rs/iced)

## Resources

- [pCloud API Documentation](https://docs.pcloud.com/)
- [pCloud Authentication Guide](https://docs.pcloud.com/methods/intro/authentication.html)
- [File Upload API](https://docs.pcloud.com/methods/file/uploadfile.html)
- [File Download API](https://docs.pcloud.com/methods/streaming/getfilelink.html)
