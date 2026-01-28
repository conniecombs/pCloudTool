# pCloudTool Code Analysis Report

**Date:** 2026-01-28
**Updated:** 2026-01-28
**Analyzed Files:** `src/lib.rs`, `src/bin/cli.rs`, `src/bin/gui.rs`, `Cargo.toml`, `tests/integration_test.rs`, `.github/workflows/ci.yml`

---

## Executive Summary

pCloudTool is a well-architected Rust application for pCloud file transfers with both CLI and GUI interfaces. The codebase demonstrates good practices in many areas, but several issues and improvement opportunities were identified during this analysis.

**Overall Assessment:** Good quality code with room for improvement in error handling, testing, and some edge cases.

### Update: Issues Fixed

The following high-priority issues have been addressed:

| Issue | Status | Fix Description |
|-------|--------|-----------------|
| Retry logic bug | ✅ Fixed | Refactored to check retryable first, then max retries |
| Token exposure in CLI | ✅ Fixed | Removed token printing from CLI output |
| Silent errors in download_folder_tree | ✅ Fixed | Added tracing warnings for failed folders |
| Missing worker validation | ✅ Fixed | Added clamp(1, 32) validation |
| No per-file timeout | ✅ Fixed | Added FileTimeoutConfig with size-based timeouts |

Additional improvements implemented:
- ✅ State file validation with SHA256 checksums
- ✅ State file repair for corrupted files
- ✅ Adaptive worker count based on CPU/memory
- ✅ Chunked uploads for large files (>2GB)
- ✅ Per-file timeout with automatic retry

---

## Table of Contents

1. [Potential Bugs](#1-potential-bugs)
2. [Security Concerns](#2-security-concerns)
3. [Code Quality Issues](#3-code-quality-issues)
4. [Performance Improvements](#4-performance-improvements)
5. [Error Handling Issues](#5-error-handling-issues)
6. [Testing Gaps](#6-testing-gaps)
7. [Documentation Issues](#7-documentation-issues)
8. [Usability Improvements](#8-usability-improvements)

---

## 1. Potential Bugs

### 1.1 ~~Retry Logic Bug in `with_retry` Method~~ ✅ FIXED

**Location:** `src/lib.rs:411-444`

**Status:** ✅ **FIXED** in commit `6bf591c`

**Issue:** The retry logic had a duplicate check for `attempt > self.retry_config.max_retries` which created confusing control flow.

**Fix Applied:**
```rust
// Now checks retryable first, then max retries
let is_retryable = match &e {
    PCloudError::NetworkError(_) => true,
    PCloudError::ApiError(s) => s.starts_with("HTTP error: 5"),
    _ => false,
};

if !is_retryable || attempt > self.retry_config.max_retries {
    return Err(e);
}
```

### 1.2 ~~Silent Error Suppression in `download_folder_tree`~~ ✅ FIXED

**Location:** `src/lib.rs:900`

**Status:** ✅ **FIXED** in commit `6bf591c`

**Issue:** Errors during folder listing were silently ignored.

**Fix Applied:** Now logs warnings via tracing when folders fail to list, tracks failed folders, and logs summary at end.

### 1.3 Potential Integer Overflow in `format_bytes`

**Location:** `src/bin/gui.rs:1744-1750`

**Issue:** The `exp` calculation could cause issues with very large numbers:
```rust
let exp = ((b as f64).ln() / 1024f64.ln()).floor() as usize;
let exp = exp.min(UNITS.len());
let unit_index = exp.saturating_sub(1);
```

**Impact:** With `UNITS.len() = 6`, edge cases near the boundary might produce incorrect results.

**Recommendation:** Add bounds checking and handle edge cases explicitly.

### 1.4 ~~Missing Validation for Workers Count~~ ✅ FIXED

**Location:** `src/lib.rs:329`, `src/bin/cli.rs:35`

**Status:** ✅ **FIXED** in commit `9341a75`

**Issue:** The `workers` parameter had no upper bound validation in the library.

**Fix Applied:**
```rust
// Now clamps workers to valid range
let workers = workers.clamp(MIN_WORKERS, MAX_WORKERS); // 1-32
```

Additionally, added `new_adaptive()` constructor that auto-configures based on system resources.

### 1.5 Double-Click Detection Race Condition

**Location:** `src/bin/gui.rs:826-838`

**Issue:** The double-click detection uses `std::time::Instant` for GUI events, but Iced provides `iced::time::Instant` which is already imported.

**Impact:** Minor inconsistency; could cause issues in certain async scenarios.

---

## 2. Security Concerns

### 2.1 ~~Auth Token Exposed in Login Output~~ ✅ FIXED

**Location:** `src/bin/cli.rs:213`

**Status:** ✅ **FIXED** in commit `6bf591c`

**Issue:** The authentication token was printed to stdout.

**Fix Applied:** Removed token printing from CLI output. Added comment explaining token can be reused via env var.

### 2.2 No TLS Certificate Validation Configuration

**Location:** `src/lib.rs:329-336`

**Issue:** The HTTP client is created with defaults without explicit TLS configuration:
```rust
let client = Client::builder()
    // ... no TLS options specified
    .build()
    .unwrap_or_default();
```

**Impact:** While reqwest uses secure defaults, explicit configuration would be more robust.

**Recommendation:** Add explicit TLS configuration:
```rust
.danger_accept_invalid_certs(false)
.min_tls_version(reqwest::tls::Version::TLS_1_2)
```

### 2.3 Password Visible in Memory

**Location:** `src/bin/gui.rs:379`, `src/bin/cli.rs:209`

**Issue:** Passwords are stored as regular `String` types which remain in memory until garbage collected.

**Impact:** Sensitive data could be extracted from memory dumps.

**Recommendation:** Consider using `secrecy` crate for sensitive data handling.

### 2.4 No Rate Limiting

**Location:** Throughout `src/lib.rs`

**Issue:** No rate limiting is implemented for API calls, which could trigger pCloud API rate limits.

**Impact:** Users could get temporarily blocked by pCloud's API.

**Recommendation:** Add configurable rate limiting or implement backoff when receiving 429 responses.

---

## 3. Code Quality Issues

### 3.1 Duplicate Size Formatting Functions

**Location:** `src/bin/cli.rs:182-191` and `src/bin/gui.rs:1735-1751`

**Issue:** Two similar but different implementations of byte formatting exist:
- CLI: `format_size()`
- GUI: `format_bytes()`

**Impact:** Inconsistent formatting between CLI and GUI outputs.

**Recommendation:** Move to shared library or create a common utility module.

### 3.2 Magic Numbers

**Location:** Various locations

**Issue:** Several magic numbers without named constants:
- `65536` (buffer size) in `lib.rs:1205`
- `400` (double-click threshold) in `gui.rs:25`
- `100` (progress update interval) in `gui.rs:189`

**Recommendation:** Define named constants for clarity.

### 3.3 Inconsistent Error Message Formatting

**Location:** Throughout the codebase

**Issue:** Error messages use different formats:
- Some use `✗ Error: ...`
- Some use `Error: ...`
- Some use just the message

**Recommendation:** Standardize error message format across the application.

### 3.4 Unused Imports/Variables

**Location:** `src/lib.rs:72`

**Issue:** The `FileProgressCallback` type alias is defined but the callback mechanism could be simplified.

### 3.5 Clone-Heavy Code in GUI

**Location:** `src/bin/gui.rs` multiple locations

**Issue:** Extensive use of `.clone()` in async tasks and closures. While necessary in some cases, some clones could be avoided with better lifetime management.

---

## 4. Performance Improvements

### 4.1 Inefficient Vector Operations in `TransferState`

**Location:** `src/lib.rs:109-123`

**Issue:** Linear searches using `contains()` on vectors:
```rust
if !self.completed_files.contains(&file_path.to_string()) {
    self.completed_files.push(file_path.to_string());
}
```

**Impact:** O(n) lookup for each completed file; slow with many files.

**Recommendation:** Use `HashSet` instead of `Vec` for `completed_files` and `failed_files`.

### 4.2 Redundant File Metadata Reads

**Location:** `src/lib.rs:966-969`, `src/bin/gui.rs:579-582`

**Issue:** File metadata is read multiple times:
```rust
let total_bytes: u64 = tasks
    .iter()
    .map(|(p, _)| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
    .sum();
```

**Impact:** Extra syscalls for each file.

**Recommendation:** Cache metadata during initial file collection.

### 4.3 Synchronous File I/O in Async Context

**Location:** `src/lib.rs:1285`, `src/bin/gui.rs:581`

**Issue:** Using `std::fs::metadata` in async contexts instead of `tokio::fs::metadata`.

**Impact:** Blocks the async runtime threadpool.

**Recommendation:** Use async file operations consistently.

### 4.4 Full File List Clone in GUI

**Location:** `src/bin/gui.rs:1431-1439`

**Issue:** File list is cloned for filtering:
```rust
let filtered_items: Vec<FileItem> = if self.search_filter.is_empty() {
    (*self.file_list).clone()  // Full clone
} else {
    // ...
};
```

**Impact:** Memory allocation on every view render.

**Recommendation:** Use iterator-based filtering without intermediate collection when possible.

---

## 5. Error Handling Issues

### 5.1 `unwrap_or_default()` Hides Client Creation Failures

**Location:** `src/lib.rs:335-336`

**Issue:**
```rust
.build()
.unwrap_or_default();
```

**Impact:** If client creation fails, a default (potentially non-functional) client is used silently.

**Recommendation:** Propagate the error or at least log the failure.

### 5.2 Missing Error Context

**Location:** Multiple locations

**Issue:** Errors lose context when converted:
```rust
.map_err(|e| e.to_string())
```

**Impact:** Makes debugging harder as original error type is lost.

**Recommendation:** Use `anyhow` with context, or preserve error chains.

### 5.3 Ignored Result in Upload Internal

**Location:** `src/lib.rs:662`

**Issue:**
```rust
let _ = self.delete_file(&full_remote).await;
```

**Impact:** If delete fails during overwrite operation, the user won't know.

### 5.4 ~~No Timeout Handling for Individual Operations~~ ✅ FIXED

**Location:** `src/lib.rs`

**Status:** ✅ **FIXED** in commit `9341a75`

**Issue:** While global timeouts existed, individual operations didn't have specific timeout handling.

**Fix Applied:** Added `FileTimeoutConfig` with size-based timeout calculation:
- `upload_file_with_timeout()` and `download_file_with_timeout()`
- `upload_files_with_timeout()` and `download_files_with_timeout()` with per-file retry
- Configurable: base_timeout + (size_mb * secs_per_mb), capped at max_timeout

---

## 6. Testing Gaps

### 6.1 No Unit Tests

**Location:** `src/lib.rs`

**Issue:** The library has no unit tests, only integration tests that require credentials.

**Impact:** Core logic (path manipulation, state management, parsing) is untested in CI.

**Recommendation:** Add unit tests for:
- Path manipulation functions
- `TransferState` serialization/deserialization
- `format_size` / `format_bytes` functions
- Retry logic
- Duplicate detection logic

### 6.2 Integration Tests Don't Clean Up

**Location:** `tests/integration_test.rs`

**Issue:** Tests create folders/files but don't clean them up:
```rust
// Create a unique test folder
let test_folder = format!("/test_folder_{}", ...);
// Created but never deleted
```

**Impact:** Test artifacts accumulate in pCloud accounts.

**Recommendation:** Add cleanup in test teardown or use a dedicated test folder that gets cleaned.

### 6.3 No GUI Tests

**Issue:** No automated tests for the GUI application.

**Recommendation:** Consider snapshot testing or basic state machine tests.

### 6.4 No Concurrent Test Coverage

**Issue:** Parallel transfer logic isn't tested.

**Recommendation:** Add tests that verify concurrent operations work correctly.

---

## 7. Documentation Issues

### 7.1 Missing Public API Documentation

**Location:** `src/lib.rs`

**Issue:** Many public types and methods lack documentation:
- `AccountInfo::available()` - undocumented
- `TransferState` fields - missing field-level docs
- `SyncResult` - missing example usage

**Recommendation:** Add `///` documentation for all public items.

### 7.2 No Architecture Documentation

**Issue:** No documentation explaining the overall architecture, data flow, or design decisions.

**Recommendation:** Add an `ARCHITECTURE.md` file explaining:
- Component interactions
- Async patterns used
- State management in GUI

### 7.3 CLI Help Could Be More Descriptive

**Location:** `src/bin/cli.rs`

**Issue:** Some CLI flags lack detailed descriptions.

**Recommendation:** Add examples to help text:
```rust
/// Remote folder path (e.g., "/Documents/Backup")
#[arg(short = 'd', long, default_value = "/")]
remote_path: String,
```

---

## 8. Usability Improvements

### 8.1 No Progress Persistence Across Restarts

**Location:** `src/bin/gui.rs`

**Issue:** If the GUI is closed during transfer, progress is lost. Transfer state is available but not automatically saved.

**Recommendation:** Auto-save transfer state periodically and offer resume on restart.

### 8.2 No Cancel Confirmation for Active Transfers

**Location:** `src/bin/gui.rs:770-774`

**Issue:** Cancel button immediately stops transfer without confirmation.

**Impact:** Users might accidentally cancel long-running transfers.

**Recommendation:** Add confirmation dialog for active transfers.

### 8.3 No Bandwidth Limiting

**Issue:** No way to limit upload/download speeds.

**Impact:** Transfers can saturate network connections.

**Recommendation:** Add bandwidth throttling option.

### 8.4 Missing Drag-and-Drop Support

**Location:** `src/bin/gui.rs`

**Issue:** Files can only be uploaded via file picker, not drag-and-drop.

**Recommendation:** Implement drag-and-drop file support using Iced's capabilities.

### 8.5 No Notification on Transfer Complete

**Issue:** No system notification when long transfers complete.

**Recommendation:** Add optional system notifications via `notify-rust` crate.

---

## Summary of Recommendations

### High Priority (Bugs/Security) - ✅ ALL FIXED

1. ✅ ~~Fix retry logic in `with_retry` method~~ - Fixed in commit `6bf591c`
2. ✅ ~~Remove token printing from CLI output~~ - Fixed in commit `6bf591c`
3. ✅ ~~Add proper error handling for silent failures~~ - Fixed in commit `6bf591c`
4. ✅ ~~Validate worker count bounds~~ - Fixed in commit `9341a75`

### Medium Priority (Code Quality)

5. ⏳ Consolidate duplicate size formatting functions
6. ⏳ Replace magic numbers with named constants
7. ⏳ Use `HashSet` for completed/failed file tracking
8. ⏳ Add unit tests for core logic

### Low Priority (Enhancements)

9. ⏳ Add bandwidth throttling
10. ⏳ Implement drag-and-drop in GUI
11. ⏳ Add system notifications
12. ⏳ Improve CLI help text with examples

---

## Additional Improvements Implemented

Beyond the original recommendations, the following enhancements were added:

### Resume Robustness (commit `9341a75`)
- State file checksum validation (SHA256)
- State file repair for corrupted files
- Version field for forward compatibility
- Validation checks: duplicates, counts, bytes, UUID

### Performance Optimizations (commit `9341a75`)
- Adaptive worker count based on CPU cores and memory
- Chunked uploads for large files (>2GB)
- Per-file timeout with automatic retry
- Exponential backoff between retries

---

## Files Changed

| Commit | Files Modified | Description |
|--------|----------------|-------------|
| `6bf591c` | `src/lib.rs`, `src/bin/cli.rs` | High priority bug/security fixes |
| `9341a75` | `src/lib.rs`, `Cargo.toml` | Resume robustness and performance |

---

## How to Address Remaining Issues

For each remaining issue, I recommend:

1. Create a GitHub issue with the details
2. Prioritize based on severity (bugs > security > quality > enhancements)
3. Address in separate PRs for easier review
4. Add tests alongside fixes

---

*Generated by code analysis on 2026-01-28*
*Updated with fixes on 2026-01-28*
