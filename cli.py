#!/usr/bin/env python3
"""
CLI interface for pCloud Fast Transfer
"""

import argparse
import os
import sys
from pathlib import Path
from getpass import getpass
from pcloud_fast_transfer import PCloudClient


def upload_command(args):
    """Handle upload command"""
    # Get credentials
    username = args.username or os.getenv('PCLOUD_USERNAME')
    password = args.password or os.getenv('PCLOUD_PASSWORD')
    auth_token = args.token or os.getenv('PCLOUD_TOKEN')

    if not auth_token and not (username and password):
        print("Error: Authentication required!")
        print("Provide either:")
        print("  1. --username and --password")
        print("  2. --token")
        print("  3. Set PCLOUD_USERNAME and PCLOUD_PASSWORD environment variables")
        print("  4. Set PCLOUD_TOKEN environment variable")
        sys.exit(1)

    # Prompt for password if username provided but not password
    if username and not password and not auth_token:
        password = getpass("Enter pCloud password: ")

    # Initialize client
    client = PCloudClient(
        username=username,
        password=password,
        auth_token=auth_token,
        region=args.region,
        workers=args.workers,
        chunk_size=args.chunk_size * 1024 * 1024,  # Convert MB to bytes
        duplicate_mode=args.duplicate_mode
    )

    # Collect files to upload
    files_to_upload = []

    for item in args.files:
        item_path = Path(item)

        if item_path.is_file():
            files_to_upload.append(str(item_path))
        elif item_path.is_dir():
            # Recursively add all files in directory
            for file_path in item_path.rglob('*'):
                if file_path.is_file():
                    files_to_upload.append(str(file_path))
        else:
            print(f"Warning: {item} not found, skipping")

    if not files_to_upload:
        print("Error: No files to upload")
        sys.exit(1)

    # Create remote folder if needed
    if args.create_folder:
        folder_id = client.create_folder(args.remote_path)
        if folder_id:
            print(f"✓ Created folder: {args.remote_path} (ID: {folder_id})")

    # Upload files
    uploaded, skipped, failed = client.upload_files(files_to_upload, args.remote_path)

    if failed > 0:
        sys.exit(1)


def download_command(args):
    """Handle download command"""
    # Get credentials
    username = args.username or os.getenv('PCLOUD_USERNAME')
    password = args.password or os.getenv('PCLOUD_PASSWORD')
    auth_token = args.token or os.getenv('PCLOUD_TOKEN')

    if not auth_token and not (username and password):
        print("Error: Authentication required!")
        sys.exit(1)

    # Prompt for password if username provided but not password
    if username and not password and not auth_token:
        password = getpass("Enter pCloud password: ")

    # Initialize client
    client = PCloudClient(
        username=username,
        password=password,
        auth_token=auth_token,
        region=args.region,
        workers=args.workers,
        duplicate_mode=args.duplicate_mode
    )

    # List remote files if directory provided
    remote_files = []

    if args.all:
        # Download all files from remote folder
        contents = client.list_folder(args.remote_path)
        for item in contents:
            if not item.get('isfolder'):
                remote_path = f"{args.remote_path.rstrip('/')}/{item['name']}"
                local_path = os.path.join(args.local_path, item['name'])
                remote_files.append((remote_path, local_path))
    else:
        # Download specific files
        for filename in args.files:
            remote_path = f"{args.remote_path.rstrip('/')}/{filename}"
            local_path = os.path.join(args.local_path, filename)
            remote_files.append((remote_path, local_path))

    if not remote_files:
        print("Error: No files to download")
        sys.exit(1)

    # Create local directory
    Path(args.local_path).mkdir(parents=True, exist_ok=True)

    # Download files
    downloaded, skipped, failed = client.download_files(remote_files)

    if failed > 0:
        sys.exit(1)


def list_command(args):
    """Handle list command"""
    # Get credentials
    username = args.username or os.getenv('PCLOUD_USERNAME')
    password = args.password or os.getenv('PCLOUD_PASSWORD')
    auth_token = args.token or os.getenv('PCLOUD_TOKEN')

    if not auth_token and not (username and password):
        print("Error: Authentication required!")
        sys.exit(1)

    # Prompt for password if username provided but not password
    if username and not password and not auth_token:
        password = getpass("Enter pCloud password: ")

    # Initialize client
    client = PCloudClient(
        username=username,
        password=password,
        auth_token=auth_token,
        region=args.region
    )

    # List folder contents
    contents = client.list_folder(args.path)

    if not contents:
        print(f"Folder '{args.path}' is empty or does not exist")
        return

    print(f"\nContents of '{args.path}':\n")
    print(f"{'Type':<10} {'Name':<40} {'Size':<15}")
    print("-" * 70)

    for item in contents:
        item_type = "DIR" if item.get('isfolder') else "FILE"
        name = item.get('name', 'Unknown')
        size = item.get('size', 0)

        # Format size
        if item.get('isfolder'):
            size_str = "-"
        else:
            size_str = format_size(size)

        print(f"{item_type:<10} {name:<40} {size_str:<15}")

    print()


def format_size(size: int) -> str:
    """Format size as human-readable string"""
    for unit in ['B', 'KB', 'MB', 'GB', 'TB']:
        if size < 1024.0:
            return f"{size:.2f} {unit}"
        size /= 1024.0
    return f"{size:.2f} PB"


def main():
    """Main CLI entry point"""
    parser = argparse.ArgumentParser(
        description='pCloud Fast Transfer - High-performance upload/download tool',
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  # Upload files
  %(prog)s upload file1.txt file2.txt --username user@example.com --remote-path /MyFolder

  # Upload entire directory
  %(prog)s upload ./mydir --username user@example.com --remote-path /Backup

  # Download files
  %(prog)s download file1.txt file2.txt --remote-path /MyFolder --local-path ./downloads

  # Download all files from folder
  %(prog)s download --all --remote-path /MyFolder --local-path ./downloads

  # List folder contents
  %(prog)s list /MyFolder

Environment Variables:
  PCLOUD_USERNAME    pCloud username (email)
  PCLOUD_PASSWORD    pCloud password
  PCLOUD_TOKEN       pCloud auth token (alternative to username/password)
        """
    )

    # Global arguments
    parser.add_argument('--username', '-u', help='pCloud username (email)')
    parser.add_argument('--password', '-p', help='pCloud password')
    parser.add_argument('--token', '-t', help='pCloud auth token')
    parser.add_argument('--region', '-r', choices=['us', 'eu'], default='us',
                       help='API region (default: us)')
    parser.add_argument('--workers', '-w', type=int, default=4,
                       help='Number of parallel workers (default: 4)')

    # Subcommands
    subparsers = parser.add_subparsers(dest='command', help='Command to execute')

    # Upload command
    upload_parser = subparsers.add_parser('upload', help='Upload files to pCloud')
    upload_parser.add_argument('files', nargs='+', help='Files or directories to upload')
    upload_parser.add_argument('--remote-path', '-d', default='/',
                              help='Remote folder path (default: /)')
    upload_parser.add_argument('--create-folder', '-c', action='store_true',
                              help='Create remote folder if it doesn\'t exist')
    upload_parser.add_argument('--chunk-size', type=int, default=10,
                              help='Chunk size in MB for large files (default: 10)')
    upload_parser.add_argument('--duplicate-mode', choices=['skip', 'overwrite', 'rename'],
                              default='rename',
                              help='How to handle duplicates: skip (skip if exists), overwrite (replace existing), rename (auto-rename) (default: rename)')
    upload_parser.set_defaults(func=upload_command)

    # Download command
    download_parser = subparsers.add_parser('download', help='Download files from pCloud')
    download_parser.add_argument('files', nargs='*', help='Files to download')
    download_parser.add_argument('--remote-path', '-d', default='/',
                                help='Remote folder path (default: /)')
    download_parser.add_argument('--local-path', '-o', default='./downloads',
                                help='Local destination path (default: ./downloads)')
    download_parser.add_argument('--all', '-a', action='store_true',
                                help='Download all files from remote folder')
    download_parser.add_argument('--duplicate-mode', choices=['skip', 'overwrite', 'rename'],
                                default='rename',
                                help='How to handle duplicates: skip (skip if exists), overwrite (replace existing), rename (let OS rename) (default: rename)')
    download_parser.set_defaults(func=download_command)

    # List command
    list_parser = subparsers.add_parser('list', help='List folder contents')
    list_parser.add_argument('path', nargs='?', default='/',
                            help='Folder path to list (default: /)')
    list_parser.set_defaults(func=list_command)

    # Parse arguments
    args = parser.parse_args()

    if not args.command:
        parser.print_help()
        sys.exit(1)

    # Execute command
    args.func(args)


if __name__ == '__main__':
    main()
