# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- Improved code quality with idiomatic Rust patterns
- Enhanced documentation with comprehensive rustdoc comments
- Reorganized Cargo.toml with better dependency grouping

## [1.0.0] - 2025-01-30

### Highlights

This is the first stable production release of pCloud Fast Transfer. It provides
a complete solution for managing files on pCloud with both GUI and CLI interfaces.

### Added

#### Core Library
- **Parallel file transfers** with configurable 1–32 concurrent workers
- **Adaptive concurrency** that auto-configures based on CPU cores and memory
- **Streaming I/O** for constant ~10 MB memory usage regardless of file size
- **Resumable transfers** with state persistence and integrity validation
- **Bidirectional sync** with optional SHA-256 checksum verification
- **Chunked uploads** for files larger than 2 GB
- **Per-file timeouts** with automatic retry and exponential backoff
- **Duplicate handling** with skip, overwrite, and rename modes
- **Zero unsafe code** for memory safety guarantees

#### CLI (`pcloud-cli`)
- Upload and download files and folders
- List, create, delete, and rename remote items
- Sync folders with multiple direction modes
- Resume interrupted transfers
- Environment variable support for credentials
- Progress bars with transfer speed metrics

#### GUI (`pcloud-gui`)
- Windows Fluent-inspired design with light/dark themes
- Tabbed interface for login, upload, download, and browse
- Real-time transfer progress with per-file tracking
- Keyboard shortcuts for efficient navigation
- Configurable worker count and duplicate handling
- File browser with sorting and filtering

### Technical Details
- Built with Rust 1.70+ for performance and safety
- Async runtime powered by Tokio
- GUI framework: Iced 0.13
- CLI framework: clap 4.5
- Comprehensive error handling with thiserror

### Platform Support
- Linux (x86_64, aarch64)
- macOS (x86_64, aarch64)
- Windows (x86_64)

---

## [0.3.0] - 2025-01-15

### Added
- Adaptive worker count based on system resources
- Chunked uploads for large files (>2GB)
- Per-file timeouts with automatic retry
- State file validation and repair mechanism
- Transfer state checksum for integrity verification

### Changed
- Improved memory efficiency for large transfers
- Enhanced error messages with more context
- Optimized connection pooling for parallel transfers

### Fixed
- Connection timeout issues on slow networks
- Memory leak in long-running sync operations

---

## [0.2.0] - 2025-01-01

### Added
- Bidirectional folder sync with SHA-256 checksum comparison
- Resume interrupted transfers functionality
- Per-file progress tracking in GUI and CLI
- Transfer state persistence to JSON

### Changed
- Improved progress display in CLI with better formatting
- Enhanced GUI with transfer statistics panel

### Fixed
- Path calculation bug in recursive uploads
- Incorrect file size reporting for empty files

---

## [0.1.0] - 2024-12-15

### Added
- Initial release
- Basic upload and download functionality
- Parallel file transfers with worker pool
- CLI interface with subcommands
- GUI interface with basic file browser
- Duplicate file handling (skip/overwrite/rename)
- Recursive folder operations

---

[Unreleased]: https://github.com/conniecombs/pCloudTool/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/conniecombs/pCloudTool/releases/tag/v1.0.0
[0.3.0]: https://github.com/conniecombs/pCloudTool/releases/tag/v0.3.0
[0.2.0]: https://github.com/conniecombs/pCloudTool/releases/tag/v0.2.0
[0.1.0]: https://github.com/conniecombs/pCloudTool/releases/tag/v0.1.0
