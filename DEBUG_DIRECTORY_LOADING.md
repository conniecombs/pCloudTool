# Debugging Directory Loading Issues

If the program is failing to load remote pCloud directories, follow these steps to diagnose and fix the issue.

## Quick Diagnosis

### Step 1: Test with CLI (Recommended)

The CLI provides immediate error feedback:

```bash
# Set your credentials
export PCLOUD_USERNAME="your-email@example.com"
export PCLOUD_PASSWORD="your-password"

# Try listing root directory
./target/release/pcloud-cli list /

# Try listing a specific folder
./target/release/pcloud-cli list /YourFolder
```

**Expected output:**
```
Contents of '/':

Type       Name                                     Size
----------------------------------------------------------------------
DIR        Documents                                -
DIR        Photos                                   -
FILE       readme.txt                               1.50 KB
```

**If it fails**, you'll see an error message like:
- `Error: Not authenticated` → Credentials are wrong
- `Error: API error: ...` → Check the specific API error
- `Error: Network error: ...` → Connection issue

### Step 2: Run GUI with Debug Output

```bash
# Run GUI and watch stderr for debug messages
./target/release/pcloud-gui 2>&1 | tee gui-debug.log
```

**What to look for:**
- `DEBUG: API result code: 0` → Success
- `DEBUG: API result code: 2000` → Authentication failure
- `DEBUG: API result code: 2005` → Folder not found
- `ERROR: list_folder failed: ...` → See specific error

### Step 3: Check Debug Output

The code now prints detailed debug information:

```
DEBUG: Listing folder: '/'
DEBUG: API URL: https://api.pcloud.com/listfolder
DEBUG: Token present: true
DEBUG: Got HTTP response, status: 200
DEBUG: API result code: 0
DEBUG: Successfully retrieved 5 items
```

## Common Issues and Fixes

### Issue 1: Authentication Failures

**Symptoms:**
- Error: "Not authenticated"
- API result code: 2000

**Solutions:**
1. Verify credentials:
   ```bash
   echo $PCLOUD_USERNAME
   echo $PCLOUD_PASSWORD
   ```

2. Check region (US vs EU):
   ```bash
   # For EU accounts
   ./target/release/pcloud-cli list / --region eu
   ```

3. Try generating a new auth token manually

### Issue 2: Empty Directory Listing

**Symptoms:**
- No error but file_list is empty
- "Found 0 items" in debug output

**Possible causes:**
1. The folder actually is empty
2. Path format is wrong (should start with `/`)
3. Permissions issue with the folder

**Test:**
```bash
# List root to verify connection works
./target/release/pcloud-cli list /

# Then try your specific path
./target/release/pcloud-cli list /YourFolderName
```

### Issue 3: Network/Connection Errors

**Symptoms:**
- `ERROR: Network request failed: ...`
- Timeout errors

**Solutions:**
1. Check internet connection
2. Try different region:
   ```bash
   # US (default)
   ./target/release/pcloud-cli list / --region us

   # EU
   ./target/release/pcloud-cli list / --region eu
   ```

3. Check firewall/proxy settings

### Issue 4: JSON Parsing Errors

**Symptoms:**
- `ERROR: Failed to parse JSON response: ...`

**This indicates:**
- pCloud API returned unexpected format
- Possible pCloud API change
- Network proxy interfering

**Debug:**
```bash
# Enable verbose logging
RUST_LOG=debug ./target/release/pcloud-cli list /
```

## Testing Authentication

Create a simple test to verify your credentials work:

```bash
# Test login only
export PCLOUD_USERNAME="your@email.com"
export PCLOUD_PASSWORD="password"

./target/release/pcloud-cli list / 2>&1 | head -20
```

**Success looks like:**
```
DEBUG: Listing folder: '/'
DEBUG: API URL: https://api.pcloud.com/listfolder
DEBUG: Token present: true
DEBUG: Got HTTP response, status: 200
DEBUG: API result code: 0
DEBUG: Successfully retrieved X items

Type       Name                Size
--------------------------------
...
```

## Advanced Debugging

### Enable Full Request Logging

```rust
// Add to src/lib.rs temporarily for debugging
eprintln!("DEBUG: Full params: {:?}", params);
eprintln!("DEBUG: Auth token: {}...", &auth[..20]);
```

### Check API Response Raw

```bash
# Use curl to test API directly
curl "https://api.pcloud.com/listfolder?auth=YOUR_TOKEN&path=/"
```

### Verify Token

```bash
# The token should be in the login response
export PCLOUD_TOKEN="your-token-here"
./target/release/pcloud-cli list / --token $PCLOUD_TOKEN
```

## Reporting Issues

If the issue persists, collect this information:

1. **Error output:**
   ```bash
   ./target/release/pcloud-cli list / 2>&1 | tee error.log
   ```

2. **Environment:**
   - OS: `uname -a`
   - Rust version: `rustc --version`
   - Region: US or EU?

3. **Debug log from GUI:**
   ```bash
   ./target/release/pcloud-gui 2>&1 | tee gui-error.log
   ```

## Quick Fixes to Try

1. **Rebuild from scratch:**
   ```bash
   cargo clean
   cargo build --release
   ```

2. **Try EU endpoint:**
   ```bash
   ./target/release/pcloud-cli list / --region eu
   ```

3. **Clear any cached credentials:**
   ```bash
   rm -rf ~/.pcloud_fast_transfer/
   ```

5. **Verify the path format:**
   - Paths must start with `/`
   - Use `/` for root
   - Use `/FolderName` for folders
   - Case-sensitive!

## Next Steps

If none of the above helps, the debug output will show exactly what's failing:
- Authentication error → Check credentials/region
- Network error → Check connection/firewall
- API error → Check path format/permissions
- Parsing error → Possible API change (report as bug)

The enhanced debug logging will pinpoint the exact failure point!
