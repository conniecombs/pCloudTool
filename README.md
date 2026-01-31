<p align="center">
  <img src="https://img.shields.io/badge/pCloud-Fast%20Transfer-0078d4?style=for-the-badge&logo=icloud&logoColor=white" alt="pCloud Fast Transfer"/>
</p>

<h1 align="center">pCloud Fast Transfer</h1>

<p align="center">
  <strong>A high-performance, production-ready file transfer client for pCloud</strong>
</p>

<p align="center">
  <a href="#features">Features</a> •
  <a href="#installation">Installation</a> •
  <a href="#usage">Usage</a> •
  <a href="#api">API</a> •
  <a href="#contributing">Contributing</a>
</p>

<p align="center">
  <a href="https://github.com/conniecombs/pCloudTool/releases"><img src="https://img.shields.io/github/v/release/conniecombs/pCloudTool?style=flat-square&color=00b894" alt="Release"></a>
  <a href="https://opensource.org/licenses/MIT"><img src="https://img.shields.io/badge/License-MIT-yellow.svg?style=flat-square" alt="License: MIT"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-1.70%2B-orange.svg?style=flat-square&logo=rust" alt="Rust"></a>
  <a href="#"><img src="https://img.shields.io/badge/Platform-Linux%20%7C%20macOS%20%7C%20Windows-blue.svg?style=flat-square" alt="Platform"></a>
</p>

---

## Overview

pCloud Fast Transfer is a Rust-based tool for efficiently managing files on pCloud storage. It provides both a modern graphical interface and a powerful command-line tool, featuring parallel transfers, resumable operations, and bidirectional synchronization.

**Why pCloud Fast Transfer?**

| Feature | Benefit |
|---------|---------|
| **Parallel Transfers** | Upload/download multiple files simultaneously with configurable concurrency |
| **Streaming I/O** | Constant ~10 MB memory usage regardless of file size |
| **Resumable Transfers** | Pick up where you left off after interruptions |
| **Bidirectional Sync** | Keep local and remote folders in sync with checksum verification |
| **Zero Unsafe Code** | Memory-safe by design with compile-time guarantees |

## Features

### Core Capabilities

- **Parallel File Transfers** — Configure 1–32 concurrent workers for optimal throughput
- **Adaptive Concurrency** — Automatically determines optimal worker count based on system resources
- **Streaming I/O** — Memory-efficient transfers that don't load files entirely into RAM
- **Resumable Transfers** — Save and restore interrupted transfer sessions
- **Bidirectional Sync** — Synchronize folders with optional SHA-256 checksum verification
- **Chunked Uploads** — Handle files larger than 2 GB using pCloud's chunked upload API
- **Per-File Timeouts** — Size-based timeouts with automatic retry and exponential backoff
- **Duplicate Handling** — Skip, overwrite, or rename files that already exist

### User Interfaces

- **GUI Application** — Modern, Windows Fluent-inspired interface with light/dark themes
- **CLI Tool** — Full-featured command-line interface with progress indicators

## Installation

### Prerequisites

- **Rust 1.70+** — Install from [rustup.rs](https://rustup.rs)
- **OpenSSL** (Linux only):
  ```bash
  # Ubuntu/Debian
  sudo apt-get install libssl-dev pkg-config

  # Fedora/RHEL
  sudo dnf install openssl-devel
  ```

### Build from Source

```bash
# Clone the repository
git clone https://github.com/conniecombs/pCloudTool.git
cd pCloudTool

# Build optimized release binaries
cargo build --release

# The binaries are located at:
# ./target/release/pcloud-gui
# ./target/release/pcloud-cli

# Optional: Install to PATH
cargo install --path .
```

## Usage

### GUI Application

```bash
./target/release/pcloud-gui
```

The GUI provides:
- Secure login with region selection (US/EU)
- File browser for pCloud navigation
- Drag-and-drop file uploads
- Real-time transfer progress with speed metrics
- Configurable worker count and duplicate handling
- Light and dark themes

**Keyboard Shortcuts:**

| Shortcut | Action |
|----------|--------|
| `Ctrl+U` | Upload files |
| `Ctrl+Shift+U` | Upload folder |
| `Ctrl+D` | Download selected |
| `Ctrl+N` | New folder |
| `Ctrl+R` / `F5` | Refresh |
| `Enter` | Open folder |
| `Backspace` | Go up |
| `Delete` | Delete selected |
| `Escape` | Cancel / Clear |

### CLI Tool

```bash
# Display help
pcloud-cli --help

# Upload files
pcloud-cli upload file1.txt file2.txt -d /Backups

# Upload a folder recursively
pcloud-cli upload ./my-folder -d /Backups

# Download files
pcloud-cli download document.pdf -d /Documents -o ./downloads

# Download a folder recursively
pcloud-cli download my-folder --recursive -d / -o ./downloads

# List folder contents
pcloud-cli list /Documents

# Create a folder
pcloud-cli create-folder /NewFolder

# Sync folders bidirectionally
pcloud-cli sync ./local-folder -d /remote-folder --direction both

# Sync with checksum verification
pcloud-cli sync ./local-folder -d /remote-folder --checksum --recursive

# Resume an interrupted transfer
pcloud-cli resume .transfer-state.json

# Show account status
pcloud-cli status
```

### Authentication

Credentials can be provided via command-line arguments or environment variables:

```bash
# Via arguments
pcloud-cli --username user@example.com --password secret upload file.txt -d /

# Via environment variables
export PCLOUD_USERNAME="user@example.com"
export PCLOUD_PASSWORD="secret"
pcloud-cli upload file.txt -d /

# Via auth token
export PCLOUD_TOKEN="your-auth-token"
pcloud-cli list /
```

### Configuration

```bash
# Set number of parallel workers (default: 8, max: 32)
pcloud-cli upload ./data -w 16 -d /Backup

# Select API region (us or eu)
pcloud-cli upload file.txt -r eu -d /

# Handle duplicates: skip, overwrite, or rename
pcloud-cli upload file.txt --duplicate-mode skip -d /
```

## API

The library can be used directly in your Rust projects:

```rust
use pcloud_rust::{PCloudClient, Region, DuplicateMode, SyncDirection};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create client with adaptive worker count
    let mut client = PCloudClient::new_adaptive(None, Region::US);

    // Authenticate
    client.login("user@example.com", "password").await?;

    // Configure duplicate handling
    client.set_duplicate_mode(DuplicateMode::Skip);

    // Upload a single file
    client.upload_file("local/file.txt", "/remote/folder").await?;

    // Upload a folder recursively
    let tasks = client.upload_folder_tree(
        "local/folder".into(),
        "/remote/backup".into(),
    ).await?;
    let (uploaded, failed) = client.upload_files(tasks).await;
    println!("Uploaded {uploaded} files, {failed} failed");

    // Sync folders bidirectionally
    let result = client.sync_folder_recursive(
        "./local",
        "/remote",
        SyncDirection::Bidirectional,
        true, // use checksums
    ).await?;
    println!("Synced: {} up, {} down", result.uploaded, result.downloaded);

    Ok(())
}
```

### Resume Interrupted Transfers

```rust
use pcloud_rust::{PCloudClient, TransferState};
use std::sync::{Arc, atomic::AtomicU64};

// Load saved state
let (mut state, validation) = TransferState::load_and_validate("transfer.json")?;

if !validation.is_valid && validation.can_repair {
    state.repair();
}

// Resume the transfer
let bytes = Arc::new(AtomicU64::new(0));
let (completed, failed) = client.resume_upload(&mut state, bytes, None).await;

// Save updated state
state.save_to_file("transfer.json")?;
```

## Performance

Benchmarks on a 100 Mbps connection:

| Operation | Files | Size | Workers | Time |
|-----------|-------|------|---------|------|
| Upload folder | 100 | 500 MB | 8 | ~45s |
| Download folder | 100 | 500 MB | 8 | ~40s |
| Large file upload | 1 | 2 GB | 1 | ~3m 20s |
| Many small files | 1000 | 100 MB | 16 | ~1m 15s |

**Memory Usage:**
- Streaming uploads: ~10 MB constant
- Parallel transfers: ~2 MB per worker
- GUI application: ~25–35 MB total

## Contributing

We welcome contributions! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

**Quick Start:**

```bash
# Clone and build
git clone https://github.com/conniecombs/pCloudTool.git
cd pCloudTool
cargo build

# Run tests
cargo test

# Check code quality
cargo clippy
cargo fmt --check
```

## License

This project is licensed under the MIT License. See [LICENSE](LICENSE) for details.

## Acknowledgments

- Built with the [pCloud API](https://docs.pcloud.com/)
- GUI powered by [Iced](https://github.com/iced-rs/iced)
- CLI powered by [clap](https://github.com/clap-rs/clap)

---

<p align="center">
  <sub>Made with Rust</sub>
</p>
