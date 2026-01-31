# Contributing to pCloud Fast Transfer

Thank you for your interest in contributing! This guide will help you get started.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Development Workflow](#development-workflow)
- [Code Style](#code-style)
- [Testing](#testing)
- [Submitting Changes](#submitting-changes)
- [Architecture Overview](#architecture-overview)

## Code of Conduct

We are committed to providing a welcoming and inclusive environment. Please:

- Be respectful and constructive in all interactions
- Welcome newcomers and help them get started
- Focus on the code, not the person
- Accept constructive criticism gracefully

## Getting Started

### Prerequisites

- **Rust 1.70+** — Install from [rustup.rs](https://rustup.rs)
- **Git** — For version control
- **OpenSSL** (Linux):
  ```bash
  # Ubuntu/Debian
  sudo apt-get install libssl-dev pkg-config

  # Fedora/RHEL
  sudo dnf install openssl-devel
  ```

### Setting Up

1. **Fork** the repository on GitHub

2. **Clone** your fork:
   ```bash
   git clone https://github.com/YOUR_USERNAME/pCloudTool.git
   cd pCloudTool
   ```

3. **Add upstream** remote:
   ```bash
   git remote add upstream https://github.com/conniecombs/pCloudTool.git
   ```

4. **Build** the project:
   ```bash
   cargo build
   ```

5. **Run tests**:
   ```bash
   cargo test
   ```

## Development Workflow

### Branching Strategy

- Create feature branches from `main`
- Use descriptive names: `feat/add-checksum-caching`, `fix/timeout-handling`
- Keep branches focused on a single feature or fix

### Making Changes

1. **Sync** with upstream:
   ```bash
   git fetch upstream
   git checkout main
   git merge upstream/main
   ```

2. **Create** a feature branch:
   ```bash
   git checkout -b feat/my-feature
   ```

3. **Make** your changes

4. **Test** thoroughly:
   ```bash
   cargo test
   cargo clippy
   ```

5. **Commit** with a clear message (see [Commit Messages](#commit-messages))

6. **Push** to your fork:
   ```bash
   git push origin feat/my-feature
   ```

7. **Open** a Pull Request

## Code Style

### Rust Conventions

- Follow the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- Run `cargo fmt` before committing
- All code must pass `cargo clippy` without warnings
- No `unsafe` code (enforced by `#![forbid(unsafe_code)]`)

### Documentation

- Add doc comments (`///`) for all public items
- Include examples in doc comments where helpful
- Update README.md for user-facing changes

### Naming Conventions

| Item | Convention | Example |
|------|------------|---------|
| Types | PascalCase | `TransferState` |
| Functions | snake_case | `upload_file` |
| Constants | SCREAMING_SNAKE_CASE | `MAX_WORKERS` |
| Modules | snake_case | `transfer_state` |

### Error Handling

- Use `Result<T, PCloudError>` for fallible operations
- Provide descriptive error messages
- Avoid `unwrap()` and `expect()` in library code

## Testing

### Unit Tests

Unit tests are located alongside the code:

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

> **Note:** Use a test account—integration tests create/delete files.

### Test Coverage

- Add tests for new functionality
- Cover edge cases and error conditions
- Aim for meaningful tests, not just coverage metrics

## Submitting Changes

### Commit Messages

Use [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

**Types:**
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation only
- `style`: Code style (formatting, etc.)
- `refactor`: Code change that neither fixes a bug nor adds a feature
- `perf`: Performance improvement
- `test`: Adding or updating tests
- `chore`: Maintenance tasks

**Examples:**
```
feat(sync): add checksum caching for faster comparisons

Implement an LRU cache for file checksums to avoid
recalculating them on subsequent sync operations.

Closes #123
```

```
fix(upload): handle network timeout correctly

Previously, timeouts were not being caught by the retry
logic. This fix wraps the upload in a tokio timeout that
properly triggers retries.
```

### Pull Request Process

1. **Title**: Use the same format as commit messages
2. **Description**: Explain what and why, not how
3. **Link**: Reference related issues with `Closes #123`
4. **Size**: Keep PRs focused; split large changes into multiple PRs
5. **Tests**: Include tests for new functionality
6. **Documentation**: Update docs if needed

### Code Review

- Address all feedback
- Explain your reasoning when disagreeing
- Be patient—reviews take time
- Keep discussions focused and constructive

## Architecture Overview

### Project Structure

```
src/
├── lib.rs              # Core library
│   ├── PCloudClient    # Main API client
│   ├── TransferState   # Resume capability
│   └── Types           # DuplicateMode, Region, etc.
└── bin/
    ├── cli.rs          # CLI application
    └── gui.rs          # GUI application
```

### Key Components

| Component | Purpose |
|-----------|---------|
| `PCloudClient` | HTTP client for pCloud API |
| `TransferState` | Persistence for resumable transfers |
| `FileProgressCallback` | Progress tracking during transfers |
| `Region` | API endpoint selection (US/EU) |
| `DuplicateMode` | Strategy for existing files |

### Data Flow

```
User Request → CLI/GUI → PCloudClient → pCloud API
                                ↓
                         Progress Callbacks
                                ↓
                         TransferState (persistence)
```

## Questions?

- Open an issue for questions about the codebase
- Check existing issues before creating a new one
- Join discussions on open PRs

---

Thank you for contributing!
