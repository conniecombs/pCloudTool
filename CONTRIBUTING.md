# Contributing to pCloud Fast Transfer

Thank you for your interest in contributing to pCloud Fast Transfer! This document provides guidelines and information for contributors.

## Code of Conduct

Please be respectful and constructive in all interactions. We welcome contributors of all experience levels.

## Getting Started

### Prerequisites

- Rust 1.70 or later ([Install Rust](https://rustup.rs))
- OpenSSL development libraries:
  ```bash
  # Ubuntu/Debian
  sudo apt-get install libssl-dev pkg-config

  # macOS
  brew install openssl

  # Windows
  # OpenSSL is typically bundled with Rust on Windows
  ```

### Setting Up the Development Environment

1. Fork and clone the repository:
   ```bash
   git clone https://github.com/YOUR_USERNAME/pCloudTool.git
   cd pCloudTool
   ```

2. Build the project:
   ```bash
   cargo build
   ```

3. Run tests:
   ```bash
   cargo test --lib
   ```

4. Run clippy for linting:
   ```bash
   cargo clippy --all-targets -- -D warnings
   ```

## Making Changes

### Branching Strategy

- Create feature branches from `main`
- Use descriptive branch names: `feature/add-checksum-verification`, `fix/upload-timeout`

### Code Style

- Follow Rust formatting conventions using `cargo fmt`
- All code must pass `cargo clippy` with warnings as errors
- No unsafe code is allowed (enforced by `#![forbid(unsafe_code)]`)
- Add documentation comments for public APIs
- Write meaningful commit messages

### Before Submitting

1. **Format your code:**
   ```bash
   cargo fmt
   ```

2. **Run clippy:**
   ```bash
   cargo clippy --all-targets -- -D warnings
   ```

3. **Run tests:**
   ```bash
   cargo test --lib
   ```

4. **Build release binaries:**
   ```bash
   cargo build --release
   ```

5. **Update documentation** if you've added or changed functionality

## Pull Request Process

1. Create a pull request with a clear title and description
2. Reference any related issues
3. Ensure all CI checks pass
4. Request review from maintainers
5. Address any feedback

### PR Title Format

Use conventional commit style:
- `feat: add bidirectional sync`
- `fix: handle timeout in chunked uploads`
- `docs: update API documentation`
- `refactor: simplify error handling`
- `test: add integration tests for sync`

## Testing

### Unit Tests

Unit tests are in the main source files and can be run without credentials:

```bash
cargo test --lib
```

### Integration Tests

Integration tests require pCloud credentials:

```bash
export PCLOUD_USERNAME="your-email@example.com"
export PCLOUD_PASSWORD="your-password"
cargo test --test integration_test -- --ignored
```

**Note:** Integration tests create and delete files in your pCloud account. Consider using a test account.

## Architecture Overview

### Project Structure

```
src/
├── lib.rs          # Core library with PCloudClient
└── bin/
    ├── cli.rs      # CLI application
    └── gui.rs      # GUI application
tests/
└── integration_test.rs
```

### Key Components

- **PCloudClient**: Main client struct for API operations
- **TransferState**: Persistence layer for resume capability
- **Region**: API endpoint selection (US/EU)
- **DuplicateMode**: Strategy for handling existing files

## Reporting Issues

When reporting bugs, please include:

1. Operating system and version
2. Rust version (`rustc --version`)
3. Steps to reproduce
4. Expected vs actual behavior
5. Error messages and logs

## Feature Requests

Feature requests are welcome! Please:

1. Check existing issues to avoid duplicates
2. Describe the use case and motivation
3. Consider implementation complexity

## Questions?

Feel free to open an issue for questions about the codebase or contribution process.

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
