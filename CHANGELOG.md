# Changelog

All notable changes to pCloud Fast Transfer will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.0] - 2025-01-30

### Highlights

This is the first stable production release of pCloud Fast Transfer, a high-performance
file transfer tool for pCloud with both CLI and GUI interfaces.

### Features

#### Core Library
- **Parallel file transfers** - Configurable 1-32 concurrent workers for maximum throughput
- **Adaptive worker count** - Automatically configures optimal workers based on CPU cores and available memory
- **Streaming I/O** - Constant ~10 MB memory usage regardless of file size
- **Resume capability** - Save and restore interrupted transfers with state file validation and repair
- **Bidirectional sync** - Sync local and remote folders with SHA256 checksum comparison
- **Chunked uploads** - Support for files >2GB using pCloud's chunked upload API
- **Per-file timeouts** - Configurable size-based timeouts with automatic retry and exponential backoff
- **Duplicate handling** - Skip, overwrite, or rename modes for existing files
- **Zero unsafe code** - Memory-safe by design with compile-time guarantees

#### CLI Application (`pcloud-cli`)
- Upload files and folders recursively
- Download files and folders with progress display
- List, create, delete, and rename remote files/folders
- Sync folders with multiple direction modes
- Resume interrupted transfers
- Environment variable support for credentials
- Verbose logging with RUST_LOG support

#### GUI Application (`pcloud-gui`)
- Windows Fluent-inspired theming with light/dark mode toggle
- Tabbed interface for login, upload, download, browse, and settings
- Real-time transfer progress with speed metrics
- Per-file progress tracking with current filename display
- Adjustable concurrency slider (1-20 workers)
- File browser for pCloud navigation
- Keyboard shortcuts support

### Technical Details
- Built with Rust 1.70+ for performance and safety
- Uses `tokio` async runtime for efficient I/O
- `iced` GUI framework for cross-platform native appearance
- `clap` for CLI argument parsing with shell completion support
- Comprehensive error handling with `thiserror`

### Platform Support
- Linux (x86_64)
- macOS (x86_64, arm64)
- Windows (x86_64)

---

## [0.3.0] - 2025-01-15

### Added
- Adaptive worker count based on system resources
- Chunked uploads for large files (>2GB)
- Per-file timeouts with automatic retry
- State file validation and repair mechanism
- Transfer state checksum for integrity

### Changed
- Improved memory efficiency for large transfers
- Enhanced error messages with more context

---

## [0.2.0] - 2025-01-01

### Added
- Bidirectional folder sync with SHA256 checksum comparison
- Resume interrupted transfers functionality
- Per-file progress tracking in GUI and CLI
- Transfer state persistence

### Changed
- Improved progress display in CLI
- Enhanced GUI with transfer statistics

---

## [0.1.0] - 2024-12-15

### Added
- Initial release
- Basic upload and download functionality
- Parallel file transfers
- CLI interface with clap
- GUI interface with iced
- Duplicate file handling (skip/overwrite/rename)
- Recursive folder operations

[1.0.0]: https://github.com/conniecombs/pCloudTool/releases/tag/v1.0.0
[0.3.0]: https://github.com/conniecombs/pCloudTool/releases/tag/v0.3.0
[0.2.0]: https://github.com/conniecombs/pCloudTool/releases/tag/v0.2.0
[0.1.0]: https://github.com/conniecombs/pCloudTool/releases/tag/v0.1.0
