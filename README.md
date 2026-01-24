# pCloud Fast Transfer

A high-performance tool for uploading and downloading files to/from pCloud with parallel transfer support, recursive folder sync, and modern GUI and CLI interfaces.

## 🦀 Rust Implementation (Recommended)

The **Rust version** is the primary implementation offering superior performance, safety, and efficiency:

- ⚡ **10x faster startup** (0.1s vs 1s)
- 💾 **Constant memory usage** (~10 MB regardless of file size)
- 🛡️ **Memory-safe** by design (zero unsafe code)
- 📦 **Single binary** with no dependencies
- 🚀 **Streaming I/O** for efficient large file handling
- 📁 **Recursive folder sync** preserving directory structure
- ✅ **Type-safe** with compile-time error checking
- 🔄 **Duplicate detection** (skip/overwrite/rename modes)

### Quick Start (Rust)

#### Prerequisites
- Rust 1.70+ ([Install Rust](https://rustup.rs))
- OpenSSL dev libraries:
  ```bash
  # Ubuntu/Debian
  sudo apt-get install libssl-dev pkg-config

  # macOS
  brew install openssl
  ```

#### Build & Install

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

### CLI Usage (Rust)

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

#### Environment Variables

```bash
# Set credentials via environment
export PCLOUD_USERNAME="user@example.com"
export PCLOUD_PASSWORD="your-password"
export PCLOUD_TOKEN="your-auth-token"  # Alternative to username/password

# Now you can omit credentials
pcloud-cli upload file.txt -d /Documents
pcloud-cli list /
```

#### Advanced Options

```bash
# Configure parallel workers (default: 8)
pcloud-cli upload ./data -w 16 -d /Backup

# Choose region (us or eu)
pcloud-cli upload file.txt -r eu -d /MyFolder

# Handle duplicates: skip, overwrite, or rename
pcloud-cli upload file.txt --duplicate-mode skip -d /MyFolder
```

### GUI Usage (Rust)

```bash
./target/release/pcloud-gui
```

**Features:**
- 🔐 Secure login interface
- 📤 Upload files or entire folders
- 📥 Download files or complete directory trees
- 📁 Browse your pCloud storage
- 📊 Real-time transfer status

### Rust API Usage

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

### Performance Benchmarks

Tested on 100 Mbps connection:

| Operation | Files | Size | Workers | Time (Rust) | Time (Python) |
|-----------|-------|------|---------|-------------|---------------|
| Upload folder | 100 | 500 MB | 8 | ~45s | ~65s |
| Download folder | 100 | 500 MB | 8 | ~40s | ~55s |
| Large file upload | 1 | 2 GB | 1 | ~3m 20s | ~3m 45s |
| Many small files | 1000 | 100 MB | 16 | ~1m 15s | ~2m 10s |

**Memory Usage:**
- Rust: Constant ~10 MB (streaming)
- Python: Grows with file size (up to 2+ GB for large files)

---

## 📦 Implementation Comparison

| Feature | Rust (Recommended) | Python (Legacy) |
|---------|-------------------|-----------------|
| **Performance** | | |
| Startup time | 0.1s | ~1s |
| Memory (large files) | ~10 MB | File size |
| Transfer speed | Excellent | Good |
| Binary size | 3-16 MB | N/A |
| | | |
| **Safety & Reliability** | | |
| Memory safety | ✅ Guaranteed | ⚠️ Runtime |
| Type safety | ✅ Compile-time | ⚠️ Runtime |
| Error handling | ✅ Result types | ⚠️ Exceptions |
| Crash resistance | ✅ High | ⚠️ Medium |
| | | |
| **Features** | | |
| Recursive folder sync | ✅ | ✅ |
| Duplicate detection | ✅ | ✅ |
| Parallel transfers | ✅ | ✅ |
| Streaming I/O | ✅ | Partial |
| GUI | ✅ | ✅ |
| CLI | ✅ | ✅ |
| | | |
| **Deployment** | | |
| Dependencies | None | Python + libs |
| Distribution | Single binary | Source + runtime |
| Installation | `cargo install` | `pip install` |
| Platform support | Linux, macOS, Windows | All with Python |
| | | |
| **Development** | | |
| Easy to modify | Good | ✅ Excellent |
| Build time | ~2-4 min | Instant |
| Debugging | Good | ✅ Excellent |

**Recommendation:** Use **Rust** for production deployments, automation, and performance-critical applications. Use **Python** for quick prototyping or if you need to modify the code frequently.

---

## 📚 Complete Documentation

- **[RUST_README.md](RUST_README.md)** - Comprehensive Rust documentation
  - Architecture details and improvements
  - Complete API reference
  - Troubleshooting guide
  - Advanced usage examples

- **[SCREENSHOTS.md](SCREENSHOTS.md)** - GUI screenshots and visual guide

---

## 🐍 Python Implementation (Legacy)

The Python version is still available for users who prefer Python or need to modify the code.

### Python Features

- **🎨 Modern GUI**: CustomTkinter-based interface with dark/light themes
- **⚡ Parallel Transfers**: Multi-threaded file operations
- **📦 Chunked Uploads**: Split large files for reliability
- **📊 Progress Tracking**: Real-time speed and progress
- **🔐 Token Caching**: Cached authentication
- **🌍 Multi-Region**: US and EU support

### Python Installation

```bash
# Install dependencies
pip install -r requirements.txt

# Run GUI
python gui.py

# Run CLI
python cli.py --help
```

### Python CLI Examples

```bash
# Upload file
python cli.py upload file.txt --username user@example.com --remote-path /MyFolder

# Upload directory
python cli.py upload ./mydir --remote-path /Backup --create-folder

# Download files
python cli.py download --all --remote-path /MyFolder --local-path ./downloads

# List contents
python cli.py list /MyFolder
```

### Python API Usage

```python
from pcloud_fast_transfer import PCloudClient

# Initialize client
client = PCloudClient(
    username="user@example.com",
    password="password",
    region="us",
    workers=4
)

# Upload files
files = ["file1.txt", "file2.txt"]
uploaded, skipped, failed = client.upload_files(files, "/MyFolder")

# Download files
remote_files = [("/MyFolder/file1.txt", "./downloads/file1.txt")]
downloaded, skipped, failed = client.download_files(remote_files)

# List folder
contents = client.list_folder("/MyFolder")
```

### Python Advanced Usage

**Parallel Workers:**
```bash
python cli.py upload ./dataset --workers 8 --remote-path /Data
```

**EU Region:**
```bash
python cli.py upload file.txt --region eu --remote-path /MyFolder
```

**Duplicate Handling:**
```bash
python cli.py upload file.txt --duplicate-mode skip --remote-path /MyFolder
```

### Python Troubleshooting

**Authentication Issues:**
```bash
# Clear cached token
rm ~/.pcloud_fast_transfer/auth_token.json
```

**Transfer Failures:**
```bash
# Reduce workers
python cli.py upload file.txt --workers 2 --remote-path /MyFolder
```

---

## 🔧 Development

### Building from Source

**Rust:**
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

**Python:**
```bash
# Install in development mode
pip install -e .

# Run with live reload (for GUI development)
python gui.py
```

---

## 🤝 Contributing

Contributions are welcome for both implementations!

**Before submitting:**
- Rust: Ensure `cargo clippy` passes with no warnings
- Python: Follow PEP 8 style guidelines
- Add tests for new features
- Update documentation

---

## 📄 License

MIT License - See LICENSE file for details

---

## 🙏 Acknowledgments

- Built with the [pCloud API](https://docs.pcloud.com/)
- Rust implementation uses [iced](https://github.com/iced-rs/iced) for GUI
- Python implementation uses [CustomTkinter](https://github.com/TomSchimansky/CustomTkinter)

---

## 📖 Resources

- [pCloud API Documentation](https://docs.pcloud.com/)
- [pCloud Authentication Guide](https://docs.pcloud.com/methods/intro/authentication.html)
- [File Upload API](https://docs.pcloud.com/methods/file/uploadfile.html)
- [File Download API](https://docs.pcloud.com/methods/streaming/getfilelink.html)

---

## ⚡ Quick Command Reference

### Rust CLI

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

### Python CLI

```bash
# Basic operations
python cli.py upload <files...> -u <email> -d <remote-path>
python cli.py download <files...> --all -d <remote-path> -o <local-path>
python cli.py list <path>

# Options are similar to Rust CLI
```

---

**For detailed documentation, see [RUST_README.md](RUST_README.md)** 🦀
