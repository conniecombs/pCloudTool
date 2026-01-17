#!/usr/bin/env python3
"""
Example usage of pCloud Fast Transfer

This script demonstrates how to use the PCloudClient programmatically.
"""

from pcloud_fast_transfer import PCloudClient
import os


def example_upload():
    """Example: Upload files to pCloud"""
    # Initialize client (credentials from environment variables)
    client = PCloudClient(
        username=os.getenv('PCLOUD_USERNAME'),
        password=os.getenv('PCLOUD_PASSWORD'),
        region='us',
        workers=4
    )

    # Create a folder
    print("\n=== Creating Folder ===")
    folder_id = client.create_folder('/TestFolder')
    if folder_id:
        print(f"Created folder with ID: {folder_id}")

    # Upload single file
    print("\n=== Uploading Single File ===")
    # Create a test file first
    test_file = 'test_upload.txt'
    with open(test_file, 'w') as f:
        f.write('Hello pCloud! This is a test upload.\n' * 100)

    success = client.upload_file(test_file, '/TestFolder')
    print(f"Upload {'succeeded' if success else 'failed'}")

    # Clean up test file
    os.remove(test_file)


def example_download():
    """Example: Download files from pCloud"""
    # Initialize client
    client = PCloudClient(
        username=os.getenv('PCLOUD_USERNAME'),
        password=os.getenv('PCLOUD_PASSWORD'),
        region='us',
        workers=4
    )

    # List folder contents
    print("\n=== Listing Folder Contents ===")
    contents = client.list_folder('/TestFolder')
    for item in contents:
        item_type = "DIR" if item.get('isfolder') else "FILE"
        print(f"{item_type}: {item['name']}")

    # Download files
    if contents:
        print("\n=== Downloading Files ===")
        remote_files = []
        for item in contents:
            if not item.get('isfolder'):
                remote_path = f"/TestFolder/{item['name']}"
                local_path = f"./downloads/{item['name']}"
                remote_files.append((remote_path, local_path))

        if remote_files:
            successful, failed = client.download_files(remote_files)
            print(f"\nDownloaded {successful} files, {failed} failed")


def example_parallel_upload():
    """Example: Upload multiple files in parallel"""
    # Initialize client with more workers
    client = PCloudClient(
        username=os.getenv('PCLOUD_USERNAME'),
        password=os.getenv('PCLOUD_PASSWORD'),
        region='us',
        workers=8  # Use 8 parallel workers
    )

    # Create multiple test files
    print("\n=== Creating Test Files ===")
    test_files = []
    for i in range(10):
        filename = f'test_file_{i}.txt'
        with open(filename, 'w') as f:
            f.write(f'Test file {i}\n' * 1000)
        test_files.append(filename)
        print(f"Created {filename}")

    # Upload all files in parallel
    print("\n=== Uploading Files in Parallel ===")
    successful, failed = client.upload_files(test_files, '/TestFolder')
    print(f"\nUploaded {successful} files, {failed} failed")

    # Clean up test files
    print("\n=== Cleaning Up ===")
    for f in test_files:
        os.remove(f)
        print(f"Removed {f}")


def example_with_progress():
    """Example: Upload with progress tracking"""
    from pcloud_fast_transfer import TransferStats

    client = PCloudClient(
        username=os.getenv('PCLOUD_USERNAME'),
        password=os.getenv('PCLOUD_PASSWORD'),
        region='us',
        workers=4
    )

    # Create a test file
    test_file = 'large_test.txt'
    print(f"\n=== Creating Test File ===")
    with open(test_file, 'w') as f:
        # Write 5MB of data
        for i in range(5 * 1024):
            f.write('X' * 1024)
    print(f"Created {test_file} (5MB)")

    # Custom progress callback
    def progress_callback(stats):
        percent = (stats.transferred_bytes / stats.total_bytes * 100) if stats.total_bytes > 0 else 0
        print(f"\rProgress: {percent:.1f}% - {stats.format_speed()}", end='', flush=True)

    # Upload with progress
    print("\n=== Uploading with Progress ===")
    success = client.upload_file(test_file, '/TestFolder', progress_callback)
    print(f"\nUpload {'succeeded' if success else 'failed'}")

    # Clean up
    os.remove(test_file)


if __name__ == '__main__':
    print("pCloud Fast Transfer - Examples")
    print("=" * 50)
    print("\nMake sure to set environment variables:")
    print("  export PCLOUD_USERNAME='your-email@example.com'")
    print("  export PCLOUD_PASSWORD='your-password'")
    print("\nOr use:")
    print("  export PCLOUD_TOKEN='your-auth-token'")
    print("=" * 50)

    # Check if credentials are set
    if not os.getenv('PCLOUD_USERNAME') and not os.getenv('PCLOUD_TOKEN'):
        print("\n❌ Error: No credentials found!")
        print("Set PCLOUD_USERNAME and PCLOUD_PASSWORD or PCLOUD_TOKEN")
        exit(1)

    # Run examples
    try:
        example_upload()
        # example_download()
        # example_parallel_upload()
        # example_with_progress()

        print("\n" + "=" * 50)
        print("✓ Examples completed successfully!")
        print("=" * 50)
    except Exception as e:
        print(f"\n❌ Error: {e}")
        import traceback
        traceback.print_exc()
