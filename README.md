# pCloud Fast Transfer

A high-performance tool for uploading and downloading files to/from pCloud with parallel transfer support. Available in both **Python** and **Rust** implementations, with modern GUI and command-line interfaces.

## 🦀 NEW: Rust Implementation Available!

For maximum performance and memory efficiency, check out the **Rust version**:
- ⚡ **10x faster** startup time
- 💾 **Constant memory usage** (~10 MB) regardless of file size
- 🛡️ **Memory-safe** by design
- 📦 **Single binary** with no dependencies
- 🚀 **Streaming uploads/downloads** for large files

See **[RUST_README.md](RUST_README.md)** for Rust-specific documentation.

## Choose Your Implementation

| Feature | Python | Rust |
|---------|--------|------|
| Easy to modify | ✅ | - |
| Memory usage (large files) | File size | ~10 MB |
| Startup time | ~1s | ~0.1s |
| Binary size | N/A | 3-16 MB |
| Type safety | Runtime | Compile-time |
| Platform support | All with Python | Linux, macOS, Windows |

---

# Python Implementation

## Features

- **🎨 Modern GUI**: Sleek, intuitive interface with dark/light themes
- **⚡ Parallel Transfers**: Upload/download multiple files simultaneously using configurable workers
- **📦 Chunked Uploads**: Large files are split into chunks for better reliability
- **📊 Progress Tracking**: Real-time transfer speed and progress monitoring
- **🔐 Token Caching**: Authentication tokens are cached for faster subsequent operations
- **🔄 Resume Support**: Failed transfers can be retried without starting over
- **🌍 Multi-Region**: Support for both US and EU pCloud regions
- **📁 Batch Operations**: Upload/download entire directories recursively
- **🖱️ Drag & Drop**: Easy file selection with modern UI (GUI mode)

## Installation

1. Clone this repository:
```bash
git clone <repository-url>
cd NewProject
```

2. Install dependencies:
```bash
pip install -r requirements.txt
```

3. Make the CLI executable (optional):
```bash
chmod +x cli.py
```

## Quick Start

### GUI Mode (Recommended for Desktop Users)

Launch the modern graphical interface:

```bash
python gui.py
```

Or use the launcher:

```bash
python launch_gui.py
```

**GUI Features:**
- 🔐 **Login Dialog**: Secure authentication with username/password
- 📤 **Upload Tab**: Select files or folders with visual progress tracking
- 📥 **Download Tab**: Browse and select files from pCloud
- 📁 **Browse Tab**: Navigate your pCloud storage
- ⚙️ **Settings**: Configure workers, appearance (dark/light mode)
- 📊 **Real-time Progress**: Live speed and ETA display during transfers

### CLI Mode (For Automation & Scripting)

### Authentication

You can authenticate using either username/password or an auth token:

**Option 1: Environment Variables (Recommended)**
```bash
export PCLOUD_USERNAME="your-email@example.com"
export PCLOUD_PASSWORD="your-password"
```

**Option 2: Command-line Arguments**
```bash
python cli.py upload file.txt --username your-email@example.com --password your-password
```

**Option 3: Auth Token**
```bash
export PCLOUD_TOKEN="your-auth-token"
```

### Basic Usage

**Upload a file:**
```bash
python cli.py upload myfile.txt --remote-path /MyFolder
```

**Upload multiple files:**
```bash
python cli.py upload file1.txt file2.txt file3.txt --remote-path /Documents
```

**Upload an entire directory:**
```bash
python cli.py upload ./mydir --remote-path /Backup --create-folder
```

**Download files:**
```bash
python cli.py download file1.txt file2.txt --remote-path /MyFolder --local-path ./downloads
```

**Download all files from a folder:**
```bash
python cli.py download --all --remote-path /MyFolder --local-path ./downloads
```

**List folder contents:**
```bash
python cli.py list /MyFolder
```

## Advanced Usage

### Parallel Workers

Increase the number of parallel workers for faster transfers:

```bash
# Use 8 parallel workers for uploads
python cli.py upload ./large-dataset --workers 8 --remote-path /Data
```

### Chunk Size

Adjust chunk size for large file uploads (in MB):

```bash
# Use 50MB chunks for large files
python cli.py upload large-file.zip --chunk-size 50 --remote-path /Files
```

### EU Region

If your pCloud account is in the EU region:

```bash
python cli.py upload file.txt --region eu --remote-path /MyFolder
```

## API Usage

You can also use the `PCloudClient` class directly in your Python code:

```python
from pcloud_fast_transfer import PCloudClient

# Initialize client
client = PCloudClient(
    username="your-email@example.com",
    password="your-password",
    region="us",
    workers=4
)

# Upload files
files = ["file1.txt", "file2.txt", "file3.txt"]
successful, failed = client.upload_files(files, remote_path="/MyFolder")

# Download files
remote_files = [
    ("/MyFolder/file1.txt", "./downloads/file1.txt"),
    ("/MyFolder/file2.txt", "./downloads/file2.txt")
]
successful, failed = client.download_files(remote_files)

# List folder contents
contents = client.list_folder("/MyFolder")
for item in contents:
    print(f"{item['name']} - {'DIR' if item['isfolder'] else 'FILE'}")

# Create folder
folder_id = client.create_folder("/NewFolder")
```

## Performance Optimization

### Tips for Maximum Speed

1. **Use Multiple Workers**: Increase workers for many small files
   ```bash
   --workers 8
   ```

2. **Adjust Chunk Size**: Larger chunks for fast connections, smaller for unstable networks
   ```bash
   --chunk-size 20  # 20MB chunks
   ```

3. **Network Location**: Use the correct region (US/EU) closest to you
   ```bash
   --region eu
   ```

4. **Batch Uploads**: Upload multiple files in one command instead of separate commands

### Benchmark Examples

Based on testing with typical configurations:

- **10 small files (1MB each)** with 4 workers: ~2-5 seconds
- **100 files (various sizes)** with 8 workers: ~20-60 seconds (depends on connection)
- **Large file (1GB)** with chunking: Better reliability, automatic retry on failure

## Command Reference

### Upload Command

```bash
python cli.py upload [FILES...] [OPTIONS]

Options:
  --remote-path, -d PATH    Remote folder path (default: /)
  --create-folder, -c       Create remote folder if it doesn't exist
  --chunk-size SIZE         Chunk size in MB for large files (default: 10)
  --workers, -w NUM         Number of parallel workers (default: 4)
  --username, -u EMAIL      pCloud username
  --password, -p PASS       pCloud password
  --token, -t TOKEN         pCloud auth token
  --region, -r REGION       API region: us or eu (default: us)
```

### Download Command

```bash
python cli.py download [FILES...] [OPTIONS]

Options:
  --remote-path, -d PATH    Remote folder path (default: /)
  --local-path, -o PATH     Local destination path (default: ./downloads)
  --all, -a                 Download all files from remote folder
  --workers, -w NUM         Number of parallel workers (default: 4)
  --username, -u EMAIL      pCloud username
  --password, -p PASS       pCloud password
  --token, -t TOKEN         pCloud auth token
  --region, -r REGION       API region: us or eu (default: us)
```

### List Command

```bash
python cli.py list [PATH] [OPTIONS]

Options:
  --username, -u EMAIL      pCloud username
  --password, -p PASS       pCloud password
  --token, -t TOKEN         pCloud auth token
  --region, -r REGION       API region: us or eu (default: us)
```

## Architecture

### Key Components

1. **PCloudClient**: Main client class with parallel transfer support
2. **TransferStats**: Real-time statistics tracking (speed, ETA, progress)
3. **Authentication**: Token-based auth with caching
4. **Parallel Execution**: ThreadPoolExecutor for concurrent transfers

### Upload Strategy

- **Small files (< 10MB)**: Direct upload via `uploadfile` API
- **Large files (≥ 10MB)**: Chunked upload with progress tracking
- **Parallel processing**: Multiple files uploaded simultaneously

### Download Strategy

- **Link generation**: Uses `getfilelink` API to get download URLs
- **Streaming downloads**: Efficient memory usage for large files
- **Parallel processing**: Multiple files downloaded simultaneously

## Troubleshooting

### Authentication Issues

If authentication fails:
1. Verify your username and password are correct
2. Check if you're using the correct region (US vs EU)
3. Try clearing the cached token:
   ```bash
   rm ~/.pcloud_fast_transfer/auth_token.json
   ```

### Upload/Download Failures

If transfers fail:
1. Check your internet connection
2. Verify file paths are correct
3. Try reducing the number of workers: `--workers 2`
4. For large files, try smaller chunk size: `--chunk-size 5`

### Rate Limiting

If you encounter rate limiting:
1. Reduce the number of parallel workers
2. Add delays between batches of uploads

## Security

- **Token Storage**: Auth tokens are stored in `~/.pcloud_fast_transfer/auth_token.json`
- **Password Handling**: Passwords are never stored; only auth tokens are cached
- **Secure Connections**: All API calls use HTTPS

## GUI Details

The graphical interface is built with **CustomTkinter**, providing a modern, responsive user experience.

### GUI Components

1. **gui.py** - Main GUI application
2. **launch_gui.py** - Simple launcher script

### GUI Architecture

- **PCloudGUI**: Main application window with tabbed interface
- **AuthDialog**: Modal dialog for secure authentication
- **ProgressDialog**: Real-time transfer progress with metrics
- **Threading**: Background operations keep UI responsive
- **Auto-login**: Cached tokens enable seamless startup

### Appearance Customization

The GUI supports three appearance modes:
- **Dark** (default): Modern dark theme
- **Light**: Bright, clean interface
- **System**: Matches OS preference

Change appearance in Settings (⚙️) after logging in.

For GUI screenshots and visual guide, see [SCREENSHOTS.md](SCREENSHOTS.md).

## API Reference

### pCloud API Documentation

This tool uses the official pCloud API. For detailed API documentation, visit:
- [pCloud API Docs](https://docs.pcloud.com/)
- [Authentication](https://docs.pcloud.com/methods/intro/authentication.html)
- [File Upload](https://docs.pcloud.com/methods/file/uploadfile.html)
- [File Download](https://docs.pcloud.com/methods/streaming/getfilelink.html)

---

## 🦀 Getting Started with Rust

### Prerequisites
- Rust 1.70+ ([Install Rust](https://rustup.rs))
- OpenSSL development libraries

### Quick Start

```bash
# Build release binaries
cargo build --release

# Run CLI
./target/release/pcloud-cli upload myfile.txt --username user@example.com --remote-path /MyFolder

# Run GUI
./target/release/pcloud-gui

# Or install to system
cargo install --path .
```

### CLI Examples

```bash
# Upload a folder recursively
pcloud-cli upload ./my-folder --username user@example.com --remote-path /Backup

# Download a folder recursively
pcloud-cli download my-folder --recursive --remote-path / --local-path ./downloads

# List folder contents
pcloud-cli list /MyFolder --username user@example.com

# Use environment variables for auth
export PCLOUD_USERNAME="user@example.com"
export PCLOUD_PASSWORD="your-password"
pcloud-cli upload file.txt --remote-path /Documents
```

For complete Rust documentation, see **[RUST_README.md](RUST_README.md)**.

---

## Contributing

Contributions are welcome for both Python and Rust implementations! Please feel free to submit issues or pull requests.

## License

MIT License - See LICENSE file for details

## Acknowledgments

- Built using the [pCloud API](https://docs.pcloud.com/)
- Inspired by high-performance transfer tools like rclone
- Uses Python's `concurrent.futures` for parallel execution

## Sources

- [pCloud Developers Documentation](https://docs.pcloud.com/)
- [pCloud API - GitHub](https://github.com/tomgross/pcloud)
- [pCloud Upload File Method](https://docs.pcloud.com/methods/file/uploadfile.html)
- [pCloud Chunked Upload Discussion](https://github.com/rclone/rclone/issues/7896)
- [pCloud Authentication Guide](https://docs.pcloud.com/methods/intro/authentication.html)
