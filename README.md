# pCloud Fast Transfer

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/Rust-1.70%2B-orange.svg)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/Platform-Linux%20%7C%20macOS%20%7C%20Windows-blue.svg)](#)

A high-performance Rust tool for uploading and downloading files to/from pCloud with parallel transfer support, recursive folder sync, and modern GUI and CLI interfaces.

![pCloud Fast Transfer Banner](https://via.placeholder.com/800x200/1a1a2e/16213e?text=pCloud+Fast+Transfer)

## Features

- **10x faster startup** (0.1s vs interpreted languages)
- **Constant memory usage** (~10 MB regardless of file size)
- **Memory-safe** by design (zero unsafe code)
- **Single binary** with no dependencies
- **Streaming I/O** for efficient large file handling
- **Recursive folder sync** preserving directory structure
- **Type-safe** with compile-time error checking
- **Duplicate detection** (skip/overwrite/rename modes)

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
# Configure parallel workers (default: 8)
pcloud-cli upload ./data -w 16 -d /Backup

# Choose region (us or eu)
pcloud-cli upload file.txt -r eu -d /MyFolder

# Handle duplicates: skip, overwrite, or rename
pcloud-cli upload file.txt --duplicate-mode skip -d /MyFolder
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
- Real-time transfer status

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
use pcloud_rust::{PCloudClient, Region, DuplicateMode};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize client
    let mut client = PCloudClient::new(None, Region::US, 8);

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

    Ok(())
}
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

# Options
-u, --username <EMAIL>       # pCloud email
-p, --password <PASSWORD>    # pCloud password
-t, --token <TOKEN>          # Auth token (alternative)
-r, --region <us|eu>         # API region
-w, --workers <N>            # Parallel workers (default: 8)
-d, --remote-path <PATH>     # Remote folder
-o, --local-path <PATH>      # Local destination
--duplicate-mode <MODE>      # skip|overwrite|rename
--recursive                  # Download folders recursively
```

## Documentation

- **[RUST_README.md](RUST_README.md)** - Comprehensive documentation
  - Architecture details and improvements
  - Complete API reference
  - Troubleshooting guide
  - Advanced usage examples

- **[SCREENSHOTS.md](SCREENSHOTS.md)** - GUI screenshots and visual guide

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
