//! Integration tests for pcloud-rust
//!
//! These tests require environment variables to be set:
//! - PCLOUD_USERNAME: pCloud account email
//! - PCLOUD_PASSWORD: pCloud account password
//!
//! Run with: cargo test --test integration_test -- --ignored

use pcloud_rust::{PCloudClient, Region, DuplicateMode, PCloudError};
use std::env;
use tempfile::TempDir;

/// Helper to create an authenticated client
async fn get_authenticated_client() -> Option<PCloudClient> {
    let username = env::var("PCLOUD_USERNAME").ok()?;
    let password = env::var("PCLOUD_PASSWORD").ok()?;

    let mut client = PCloudClient::new(None, Region::US, 4);
    client.login(&username, &password).await.ok()?;
    Some(client)
}

#[tokio::test]
#[ignore] // Requires credentials
async fn test_login_success() {
    let username = env::var("PCLOUD_USERNAME").expect("PCLOUD_USERNAME not set");
    let password = env::var("PCLOUD_PASSWORD").expect("PCLOUD_PASSWORD not set");

    let mut client = PCloudClient::new(None, Region::US, 4);
    let result = client.login(&username, &password).await;

    assert!(result.is_ok());
    let token = result.unwrap();
    assert!(!token.is_empty());
}

#[tokio::test]
#[ignore] // Requires credentials
async fn test_login_invalid_credentials() {
    let mut client = PCloudClient::new(None, Region::US, 4);
    let result = client.login("invalid@example.com", "wrongpassword").await;

    assert!(result.is_err());
    if let Err(PCloudError::ApiError(msg)) = result {
        // pCloud returns error for invalid credentials
        assert!(!msg.is_empty());
    }
}

#[tokio::test]
#[ignore] // Requires credentials
async fn test_list_root_folder() {
    let client = get_authenticated_client().await.expect("Failed to authenticate");

    let result = client.list_folder("/").await;
    assert!(result.is_ok());

    let items = result.unwrap();
    // Root folder listing should succeed (result is a valid vec, might be empty)
    drop(items);
}

#[tokio::test]
#[ignore] // Requires credentials
async fn test_create_and_list_folder() {
    let client = get_authenticated_client().await.expect("Failed to authenticate");

    // Create a unique test folder
    let test_folder = format!("/test_folder_{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis());

    // Create the folder
    let create_result = client.create_folder(&test_folder).await;
    assert!(create_result.is_ok());

    // List root to verify folder exists
    let list_result = client.list_folder("/").await;
    assert!(list_result.is_ok());

    let items = list_result.unwrap();
    let folder_name = test_folder.trim_start_matches('/');
    assert!(items.iter().any(|item| item.name == folder_name && item.isfolder));
}

#[tokio::test]
#[ignore] // Requires credentials
async fn test_upload_and_download_file() {
    let client = get_authenticated_client().await.expect("Failed to authenticate");

    // Create a temporary directory and file
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let test_file_path = temp_dir.path().join("test_upload.txt");
    let test_content = "Hello, pCloud! This is a test file.";
    std::fs::write(&test_file_path, test_content).expect("Failed to write test file");

    // Upload the file
    let upload_result = client.upload_file(
        test_file_path.to_str().unwrap(),
        "/"
    ).await;
    assert!(upload_result.is_ok());

    // Download the file to a new location
    let download_dir = TempDir::new().expect("Failed to create download dir");
    let download_result = client.download_file(
        "/test_upload.txt",
        download_dir.path().to_str().unwrap()
    ).await;
    assert!(download_result.is_ok());

    // Verify the content
    let downloaded_path = download_dir.path().join("test_upload.txt");
    let downloaded_content = std::fs::read_to_string(&downloaded_path)
        .expect("Failed to read downloaded file");
    assert_eq!(downloaded_content, test_content);
}

#[tokio::test]
#[ignore] // Requires credentials
async fn test_duplicate_mode_skip() {
    let mut client = get_authenticated_client().await.expect("Failed to authenticate");
    client.set_duplicate_mode(DuplicateMode::Skip);

    // Create and upload a test file
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let test_file_path = temp_dir.path().join("skip_test.txt");
    std::fs::write(&test_file_path, "Original content").expect("Failed to write test file");

    // First upload
    let result1 = client.upload_file(test_file_path.to_str().unwrap(), "/").await;
    assert!(result1.is_ok());

    // Second upload with same file (should be skipped)
    let result2 = client.upload_file(test_file_path.to_str().unwrap(), "/").await;
    assert!(result2.is_ok()); // Skip mode doesn't error, just skips
}

#[tokio::test]
async fn test_client_without_auth() {
    let client = PCloudClient::new(None, Region::US, 4);

    // Attempting to list without auth should fail
    let result = client.list_folder("/").await;
    assert!(result.is_err());

    if let Err(PCloudError::NotAuthenticated) = result {
        // Expected
    } else {
        panic!("Expected NotAuthenticated error");
    }
}

#[tokio::test]
async fn test_upload_nonexistent_file() {
    let mut client = PCloudClient::new(Some("fake_token".to_string()), Region::US, 4);
    client.set_token("fake_token".to_string());

    let result = client.upload_file("/nonexistent/file.txt", "/").await;
    assert!(result.is_err());

    if let Err(PCloudError::FileNotFound(_)) = result {
        // Expected
    } else {
        panic!("Expected FileNotFound error");
    }
}
