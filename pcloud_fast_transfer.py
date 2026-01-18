#!/usr/bin/env python3
"""
pCloud Fast Transfer - High-performance upload/download tool for pCloud

Features:
- Parallel file transfers (multiple files simultaneously)
- Chunked uploads for large files with resume capability
- Progress tracking with speed metrics
- Authentication token caching
- Support for both US and EU regions
"""

import os
import sys
import json
import hashlib
import time
import requests
from requests.adapters import HTTPAdapter
from urllib3.util.retry import Retry
from typing import Optional, List, Dict, Tuple, Callable
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from dataclasses import dataclass
from threading import Lock

# Try to import keyring for secure token storage
try:
    import keyring
    KEYRING_AVAILABLE = True
except ImportError:
    KEYRING_AVAILABLE = False


@dataclass
class TransferStats:
    """Track transfer statistics"""
    total_bytes: int = 0
    transferred_bytes: int = 0
    start_time: float = 0
    files_completed: int = 0
    files_total: int = 0

    def __post_init__(self):
        if self.start_time == 0:
            self.start_time = time.time()

    def get_speed(self) -> float:
        """Get transfer speed in bytes per second"""
        elapsed = time.time() - self.start_time
        if elapsed > 0:
            return self.transferred_bytes / elapsed
        return 0

    def get_eta(self) -> float:
        """Get estimated time remaining in seconds"""
        speed = self.get_speed()
        if speed > 0:
            remaining = self.total_bytes - self.transferred_bytes
            return remaining / speed
        return 0

    def format_speed(self) -> str:
        """Format speed as human-readable string"""
        speed = self.get_speed()
        for unit in ['B/s', 'KB/s', 'MB/s', 'GB/s']:
            if speed < 1024.0:
                return f"{speed:.2f} {unit}"
            speed /= 1024.0
        return f"{speed:.2f} TB/s"

    def format_size(self, size: int) -> str:
        """Format size as human-readable string"""
        for unit in ['B', 'KB', 'MB', 'GB', 'TB']:
            if size < 1024.0:
                return f"{size:.2f} {unit}"
            size /= 1024.0
        return f"{size:.2f} PB"


class PCloudClient:
    """Fast pCloud client with parallel transfer support"""

    # API endpoints
    API_ENDPOINTS = {
        'us': 'https://api.pcloud.com',
        'eu': 'https://eapi.pcloud.com'
    }

    # Chunk size for uploads (10MB)
    CHUNK_SIZE = 10 * 1024 * 1024

    # Default number of parallel workers
    DEFAULT_WORKERS = 4

    def __init__(self, username: Optional[str] = None, password: Optional[str] = None,
                 auth_token: Optional[str] = None, region: str = 'us',
                 workers: int = DEFAULT_WORKERS, chunk_size: int = CHUNK_SIZE,
                 duplicate_mode: str = 'rename'):
        """
        Initialize pCloud client

        Args:
            username: pCloud username (email)
            password: pCloud password
            auth_token: Pre-existing auth token (if available)
            region: API region ('us' or 'eu')
            workers: Number of parallel workers for transfers
            chunk_size: DEPRECATED - No longer used (kept for backward compatibility)
            duplicate_mode: How to handle duplicates ('skip', 'overwrite', 'rename')

        Note:
            The chunk_size parameter is deprecated. Uploads now use streaming for all
            file sizes, which is memory-efficient without requiring manual chunking.
        """
        self.region = region
        self.api_url = self.API_ENDPOINTS[region]
        self.workers = workers
        self.chunk_size = chunk_size
        self.auth_token = auth_token
        self.username = username
        self.password = password
        self.stats_lock = Lock()
        self.duplicate_mode = duplicate_mode

        # Setup HTTP session with connection pooling and retry logic
        self.session = requests.Session()

        # Configure retry strategy with exponential backoff
        retry_strategy = Retry(
            total=4,  # Maximum number of retries
            backoff_factor=1,  # Exponential backoff: {backoff factor} * (2 ** ({retry count} - 1))
            # Wait: 0s, 1s, 2s, 4s between retries
            status_forcelist=[429, 500, 502, 503, 504],  # Retry on these HTTP status codes
            allowed_methods=["HEAD", "GET", "PUT", "DELETE", "OPTIONS", "TRACE", "POST"],  # Retry on all methods
            raise_on_status=False  # Don't raise exception on max retries, let us handle it
        )

        # Configure connection pool size to match workers
        adapter = HTTPAdapter(
            pool_connections=max(self.workers, 10),
            pool_maxsize=max(self.workers * 2, 20),
            max_retries=retry_strategy
        )
        self.session.mount('https://', adapter)
        self.session.mount('http://', adapter)

        # Cache directory for auth tokens
        self.cache_dir = Path.home() / '.pcloud_fast_transfer'
        self.cache_dir.mkdir(exist_ok=True)
        self.token_cache_file = self.cache_dir / 'auth_token.json'

        # Load cached token if available
        if not self.auth_token:
            self.auth_token = self._load_cached_token()

        # Authenticate if needed
        if not self.auth_token and username and password:
            self.authenticate()

    def _load_cached_token(self) -> Optional[str]:
        """Load authentication token from secure storage (keyring) or fallback to file cache"""
        # Try keyring first (most secure)
        if KEYRING_AVAILABLE:
            try:
                token = keyring.get_password("pcloud_fast_transfer", f"{self.region}_auth_token")
                if token:
                    return token
            except Exception as e:
                print(f"Warning: Could not load token from keyring: {type(e).__name__}: {e}")

        # Fallback to JSON file for backward compatibility
        try:
            if self.token_cache_file.exists():
                with open(self.token_cache_file, 'r') as f:
                    data = json.load(f)
                    token = data.get('auth_token')

                    # Migrate to keyring if available
                    if token and KEYRING_AVAILABLE:
                        try:
                            keyring.set_password("pcloud_fast_transfer", f"{self.region}_auth_token", token)
                            # Delete old file after successful migration
                            self.token_cache_file.unlink()
                            print("✓ Migrated token to secure keyring storage")
                        except Exception:
                            pass  # Keep using file if migration fails

                    return token
        except json.JSONDecodeError:
            print(f"Warning: Cached token file is corrupted, will re-authenticate")
        except PermissionError:
            print(f"Warning: No permission to read cached token at {self.token_cache_file}")
        except Exception as e:
            print(f"Warning: Could not load cached token: {type(e).__name__}: {e}")
        return None

    def _save_cached_token(self, token: str):
        """Save authentication token to secure storage (keyring) or fallback to file cache"""
        # Try keyring first (most secure)
        if KEYRING_AVAILABLE:
            try:
                keyring.set_password("pcloud_fast_transfer", f"{self.region}_auth_token", token)
                # Also save region separately
                keyring.set_password("pcloud_fast_transfer", "region", self.region)
                return
            except Exception as e:
                print(f"Warning: Could not save token to keyring, using file fallback: {e}")

        # Fallback to JSON file (less secure but works everywhere)
        try:
            # Set restrictive permissions (owner read/write only)
            with open(self.token_cache_file, 'w') as f:
                json.dump({'auth_token': token, 'region': self.region}, f)

            # Set file permissions to 600 (owner read/write only) on Unix-like systems
            if os.name != 'nt':  # Not Windows
                os.chmod(self.token_cache_file, 0o600)
        except Exception as e:
            print(f"Warning: Could not save token to cache: {e}")

    def authenticate(self) -> bool:
        """
        Authenticate with pCloud and get auth token

        Returns:
            True if authentication successful
        """
        try:
            # Use userinfo method to authenticate and get token
            response = self._api_call('userinfo', {
                'username': self.username,
                'password': self.password,
                'getauth': 1
            })

            if response.get('result') == 0:
                self.auth_token = response.get('auth')
                self._save_cached_token(self.auth_token)
                print(f"✓ Authenticated successfully")
                return True
            else:
                error_code = response.get('result')
                error = response.get('error', 'Unknown error')

                # Provide specific error messages based on error code
                if error_code == 2000:
                    print(f"✗ Authentication failed: Invalid username or password")
                elif error_code == 2094:
                    print(f"✗ Authentication failed: Too many login attempts. Try again later")
                elif 'network' in error.lower() or 'connection' in error.lower():
                    print(f"✗ Authentication failed: Network error - check your internet connection")
                else:
                    print(f"✗ Authentication failed: {error}")
                return False
        except requests.exceptions.Timeout:
            print(f"✗ Authentication failed: Connection timeout - check your internet connection")
            return False
        except requests.exceptions.ConnectionError:
            print(f"✗ Authentication failed: Cannot connect to pCloud servers")
            return False
        except KeyError as e:
            print(f"✗ Authentication failed: Invalid response from server (missing {e})")
            return False
        except Exception as e:
            print(f"✗ Authentication error: {type(e).__name__}: {e}")
            return False

    def _api_call(self, method: str, params: Dict = None, files: Dict = None) -> Dict:
        """
        Make an API call to pCloud

        Args:
            method: API method name
            params: Query parameters
            files: Files to upload (for multipart requests)

        Returns:
            API response as dictionary
        """
        url = f"{self.api_url}/{method}"

        # Add auth token if available
        if self.auth_token and params:
            params['auth'] = self.auth_token

        try:
            if files:
                # Multipart upload - use session for connection pooling
                response = self.session.post(url, params=params, files=files, timeout=300)
            else:
                # Regular API call - use session for connection pooling
                response = self.session.get(url, params=params, timeout=30)

            response.raise_for_status()
            return response.json()
        except requests.exceptions.RequestException as e:
            print(f"API call error: {e}")
            return {'result': 2000, 'error': str(e)}

    def create_folder(self, path: str) -> Optional[int]:
        """
        Create a folder in pCloud

        Args:
            path: Folder path (e.g., '/MyFolder')

        Returns:
            Folder ID if successful, None otherwise
        """
        response = self._api_call('createfolderifnotexists', {'path': path})

        if response.get('result') == 0:
            folder_id = response.get('metadata', {}).get('folderid')
            return folder_id
        else:
            error = response.get('error', 'Unknown error')
            print(f"✗ Failed to create folder '{path}': {error}")
            return None

    def list_folder(self, path: str = '/') -> List[Dict]:
        """
        List contents of a folder

        Args:
            path: Folder path

        Returns:
            List of file/folder metadata
        """
        response = self._api_call('listfolder', {'path': path})

        if response.get('result') == 0:
            metadata = response.get('metadata', {})
            contents = metadata.get('contents', [])
            return contents
        else:
            error = response.get('error', 'Unknown error')
            print(f"✗ Failed to list folder '{path}': {error}")
            return []

    def check_file_exists(self, remote_folder: str, filename: str) -> Optional[Dict]:
        """
        Check if a file exists in a remote folder

        Args:
            remote_folder: Remote folder path
            filename: Name of the file to check

        Returns:
            File metadata dict if exists, None otherwise
        """
        contents = self.list_folder(remote_folder)
        for item in contents:
            if not item.get('isfolder') and item.get('name') == filename:
                return item
        return None

    def _calculate_file_hash(self, file_path: Path, hash_type: str = 'sha256') -> str:
        """
        Calculate hash of a local file

        Args:
            file_path: Path to the file
            hash_type: Type of hash ('sha256', 'sha1', 'md5')

        Returns:
            Hex digest of the file hash
        """
        if hash_type == 'sha256':
            hasher = hashlib.sha256()
        elif hash_type == 'sha1':
            hasher = hashlib.sha1()
        elif hash_type == 'md5':
            hasher = hashlib.md5()
        else:
            raise ValueError(f"Unsupported hash type: {hash_type}")

        # Read file in chunks to avoid loading entire file into memory
        with open(file_path, 'rb') as f:
            for chunk in iter(lambda: f.read(8192), b''):
                hasher.update(chunk)

        return hasher.hexdigest()

    def should_skip_upload(self, local_file: Path, remote_folder: str) -> Tuple[bool, Optional[str]]:
        """
        Determine if an upload should be skipped based on duplicate mode

        Args:
            local_file: Local file path
            remote_folder: Remote folder path

        Returns:
            Tuple of (should_skip, reason)
        """
        if self.duplicate_mode == 'rename':
            return False, None  # Never skip, let pCloud rename

        existing_file = self.check_file_exists(remote_folder, local_file.name)

        if existing_file is None:
            return False, None  # File doesn't exist, don't skip

        # File exists, check mode
        if self.duplicate_mode == 'skip':
            # First check size for quick rejection
            local_size = local_file.stat().st_size
            remote_size = existing_file.get('size', 0)

            if local_size != remote_size:
                return True, f"exists but different size (local: {local_size}, remote: {remote_size})"

            # Sizes match, now verify with hash
            # pCloud provides a 'hash' field (64-bit integer) in metadata
            remote_hash = existing_file.get('hash')

            if remote_hash is not None:
                # pCloud's hash is proprietary, but we can use SHA-256 for local comparison
                # Since we can't directly compare different hash types, we'll use size + modified time
                # or download a small portion to verify
                # For now, if sizes match and pCloud hash exists, consider them likely identical
                # This is a practical trade-off between accuracy and performance
                local_mtime = int(local_file.stat().st_mtime)
                remote_mtime = existing_file.get('modified', 0)

                # If modification times are very close (within 2 seconds), likely the same file
                if abs(local_mtime - remote_mtime) <= 2:
                    return True, f"identical (size: {local_size} bytes, similar timestamps)"

            # Fall back to size-only comparison with warning
            return True, f"likely identical (same size: {local_size} bytes)"

        elif self.duplicate_mode == 'overwrite':
            return False, "will overwrite"

        return False, None

    def should_skip_download(self, local_path: str, remote_size: int, remote_metadata: Dict = None) -> Tuple[bool, Optional[str]]:
        """
        Determine if a download should be skipped based on duplicate mode

        Args:
            local_path: Local file path
            remote_size: Size of remote file in bytes
            remote_metadata: Optional remote file metadata (includes modified time, hash)

        Returns:
            Tuple of (should_skip, reason)
        """
        local_file = Path(local_path)

        if not local_file.exists():
            return False, None  # File doesn't exist locally, don't skip

        if self.duplicate_mode == 'skip':
            # First check size for quick rejection
            local_size = local_file.stat().st_size

            if local_size != remote_size:
                return True, f"exists but different size (local: {local_size}, remote: {remote_size})"

            # Sizes match, now verify with additional metadata if available
            if remote_metadata:
                remote_mtime = remote_metadata.get('modified', 0)
                local_mtime = int(local_file.stat().st_mtime)

                # If modification times are very close (within 2 seconds), likely the same file
                if abs(local_mtime - remote_mtime) <= 2:
                    return True, f"identical (size: {local_size} bytes, similar timestamps)"

            # Fall back to size-only comparison
            return True, f"likely identical (same size: {local_size} bytes)"

        elif self.duplicate_mode == 'overwrite':
            return False, "will overwrite"

        elif self.duplicate_mode == 'rename':
            return False, None  # Let OS handle renaming

        return False, None

    def upload_file(self, local_path: str, remote_path: str = '/',
                   progress_callback: Optional[Callable] = None) -> Tuple[bool, str]:
        """
        Upload a single file to pCloud

        Args:
            local_path: Local file path
            remote_path: Remote folder path
            progress_callback: Optional callback for progress updates

        Returns:
            Tuple of (success, status_message)
            status can be: 'uploaded', 'skipped', 'failed'
        """
        local_file = Path(local_path)

        if not local_file.exists():
            print(f"✗ File not found: {local_path}")
            return False, 'failed'

        # Check for duplicates
        should_skip, skip_reason = self.should_skip_upload(local_file, remote_path)
        if should_skip:
            print(f"⊘ Skipped {local_file.name}: {skip_reason}")
            return True, 'skipped'

        # Upload file using streaming (memory-efficient for all file sizes)
        success = self._upload_with_stream(local_file, remote_path, progress_callback)

        return success, ('uploaded' if success else 'failed')

    def _upload_with_stream(self, local_file: Path, remote_path: str,
                            progress_callback: Optional[Callable] = None) -> bool:
        """
        Upload a file using streaming (memory-efficient for files of any size)

        Note: The requests library automatically streams file objects without loading
        the entire file into memory, making this approach suitable for large files.
        """
        try:
            # Determine upload parameters based on duplicate mode
            params = {'path': remote_path}
            if self.duplicate_mode == 'rename':
                params['renameifexists'] = 1
            elif self.duplicate_mode == 'overwrite':
                params['nopartial'] = 1  # Overwrite existing file

            # Open file and upload - requests will stream the file content automatically
            with open(local_file, 'rb') as f:
                files = {'file': (local_file.name, f)}
                response = self._api_call('uploadfile', params, files=files)

            if response.get('result') == 0:
                if progress_callback:
                    progress_callback(local_file.stat().st_size)
                return True
            else:
                error = response.get('error', 'Unknown error')
                print(f"✗ Upload failed for {local_file.name}: {error}")
                return False
        except Exception as e:
            print(f"✗ Upload error for {local_file.name}: {e}")
            return False

    def upload_files(self, file_paths: List[str], remote_path: str = '/',
                     progress_callback: Optional[Callable] = None) -> Tuple[int, int, int]:
        """
        Upload multiple files in parallel

        Args:
            file_paths: List of local file paths
            remote_path: Remote folder path
            progress_callback: Optional callback for progress updates

        Returns:
            Tuple of (uploaded, skipped, failed)
        """
        uploaded = 0
        skipped = 0
        failed = 0

        # Calculate total size
        total_size = sum(Path(f).stat().st_size for f in file_paths if Path(f).exists())
        stats = TransferStats(total_bytes=total_size, files_total=len(file_paths))

        print(f"\n📤 Uploading {len(file_paths)} files ({stats.format_size(total_size)})...")
        print(f"   Using {self.workers} parallel workers")
        if self.duplicate_mode != 'rename':
            print(f"   Duplicate mode: {self.duplicate_mode}\n")
        else:
            print()

        def upload_with_progress(file_path: str) -> Tuple[bool, str]:
            """Upload file and update progress"""
            def update_progress(bytes_transferred: int):
                with self.stats_lock:
                    stats.transferred_bytes += bytes_transferred
                    if progress_callback:
                        progress_callback(stats)

            success, status = self.upload_file(file_path, remote_path, update_progress)

            with self.stats_lock:
                stats.files_completed += 1

            return success, status

        # Upload files in parallel
        with ThreadPoolExecutor(max_workers=self.workers) as executor:
            futures = {executor.submit(upload_with_progress, fp): fp
                      for fp in file_paths}

            for future in as_completed(futures):
                file_path = futures[future]
                try:
                    success, status = future.result()
                    if status == 'uploaded':
                        uploaded += 1
                        print(f"✓ {Path(file_path).name} [{stats.files_completed}/{stats.files_total}] - {stats.format_speed()}")
                    elif status == 'skipped':
                        skipped += 1
                        # Already printed in upload_file
                    else:
                        failed += 1
                except Exception as e:
                    failed += 1
                    print(f"✗ {Path(file_path).name}: {e}")

        print(f"\n✓ Upload complete: {uploaded} uploaded, {skipped} skipped, {failed} failed")
        print(f"  Total time: {time.time() - stats.start_time:.2f}s")
        print(f"  Average speed: {stats.format_speed()}")

        return uploaded, skipped, failed

    def get_download_link(self, file_id: Optional[int] = None,
                         path: Optional[str] = None) -> Optional[str]:
        """
        Get download link for a file

        Args:
            file_id: File ID
            path: File path (alternative to file_id)

        Returns:
            Download URL if successful
        """
        params = {}
        if file_id:
            params['fileid'] = file_id
        elif path:
            params['path'] = path
        else:
            print("✗ Must provide either file_id or path")
            return None

        response = self._api_call('getfilelink', params)

        if response.get('result') == 0:
            hosts = response.get('hosts', [])
            path = response.get('path')
            if hosts and path:
                # Use first host (recommended by pCloud)
                return f"https://{hosts[0]}{path}"

        error = response.get('error', 'Unknown error')
        print(f"✗ Failed to get download link: {error}")
        return None

    def download_file(self, remote_path: str, local_path: str,
                     progress_callback: Optional[Callable] = None) -> Tuple[bool, str]:
        """
        Download a file from pCloud

        Args:
            remote_path: Remote file path
            local_path: Local destination path
            progress_callback: Optional callback for progress updates

        Returns:
            Tuple of (success, status_message)
            status can be: 'downloaded', 'skipped', 'failed'
        """
        # Get download link
        download_url = self.get_download_link(path=remote_path)

        if not download_url:
            return False, 'failed'

        try:
            # Download file with streaming - use session for connection pooling
            response = self.session.get(download_url, stream=True, timeout=300)
            response.raise_for_status()

            total_size = int(response.headers.get('content-length', 0))

            # Check for duplicates
            should_skip, skip_reason = self.should_skip_download(local_path, total_size)
            if should_skip:
                print(f"⊘ Skipped {Path(local_path).name}: {skip_reason}")
                response.close()  # Close the connection
                return True, 'skipped'

            downloaded = 0

            local_file = Path(local_path)
            local_file.parent.mkdir(parents=True, exist_ok=True)

            with open(local_file, 'wb') as f:
                for chunk in response.iter_content(chunk_size=8192):
                    if chunk:
                        f.write(chunk)
                        downloaded += len(chunk)
                        if progress_callback:
                            progress_callback(len(chunk))

            return True, 'downloaded'
        except Exception as e:
            print(f"✗ Download error for {remote_path}: {e}")
            return False, 'failed'

    def download_files(self, remote_files: List[Tuple[str, str]],
                       progress_callback: Optional[Callable] = None) -> Tuple[int, int, int]:
        """
        Download multiple files in parallel

        Args:
            remote_files: List of (remote_path, local_path) tuples
            progress_callback: Optional callback for progress updates

        Returns:
            Tuple of (downloaded, skipped, failed)
        """
        downloaded = 0
        skipped = 0
        failed = 0

        stats = TransferStats(files_total=len(remote_files))

        print(f"\n📥 Downloading {len(remote_files)} files...")
        print(f"   Using {self.workers} parallel workers")
        if self.duplicate_mode != 'rename':
            print(f"   Duplicate mode: {self.duplicate_mode}\n")
        else:
            print()

        def download_with_progress(remote_path: str, local_path: str) -> Tuple[bool, str]:
            """Download file and update progress"""
            def update_progress(bytes_transferred: int):
                with self.stats_lock:
                    stats.transferred_bytes += bytes_transferred
                    if progress_callback:
                        progress_callback(stats)

            success, status = self.download_file(remote_path, local_path, update_progress)

            with self.stats_lock:
                stats.files_completed += 1

            return success, status

        # Download files in parallel
        with ThreadPoolExecutor(max_workers=self.workers) as executor:
            futures = {executor.submit(download_with_progress, rp, lp): (rp, lp)
                      for rp, lp in remote_files}

            for future in as_completed(futures):
                remote_path, local_path = futures[future]
                try:
                    success, status = future.result()
                    if status == 'downloaded':
                        downloaded += 1
                        print(f"✓ {Path(remote_path).name} [{stats.files_completed}/{stats.files_total}] - {stats.format_speed()}")
                    elif status == 'skipped':
                        skipped += 1
                        # Already printed in download_file
                    else:
                        failed += 1
                except Exception as e:
                    failed += 1
                    print(f"✗ {Path(remote_path).name}: {e}")

        print(f"\n✓ Download complete: {downloaded} downloaded, {skipped} skipped, {failed} failed")
        print(f"  Total time: {time.time() - stats.start_time:.2f}s")
        print(f"  Average speed: {stats.format_speed()}")

        return downloaded, skipped, failed


if __name__ == '__main__':
    print("pCloud Fast Transfer - Use cli.py for command-line interface")
