use clap::{Parser, Subcommand};
use pcloud_rust::{DuplicateMode, PCloudClient, Region, SyncDirection, TransferState};
use std::path::Path;
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "pcloud-cli")]
#[command(author, version, about = "pCloud Fast Transfer CLI - High-performance upload/download tool", long_about = None)]
struct Cli {
    /// Enable verbose logging (can also use RUST_LOG env var)
    #[arg(short, long)]
    verbose: bool,

    /// pCloud username (email) - can also use PCLOUD_USERNAME env var
    #[arg(short, long, env = "PCLOUD_USERNAME")]
    username: Option<String>,

    /// pCloud password - can also use PCLOUD_PASSWORD env var
    #[arg(short, long, env = "PCLOUD_PASSWORD")]
    password: Option<String>,

    /// pCloud auth token - can also use PCLOUD_TOKEN env var
    #[arg(short, long, env = "PCLOUD_TOKEN")]
    token: Option<String>,

    /// API region (us or eu)
    #[arg(short, long, default_value = "us")]
    region: String,

    /// Number of parallel workers
    #[arg(short, long, default_value = "8")]
    workers: usize,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Upload files or folders to pCloud
    Upload {
        /// Files or directories to upload
        files: Vec<String>,

        /// Remote folder path
        #[arg(short = 'd', long, default_value = "/")]
        remote_path: String,

        /// Create remote folder if it doesn't exist
        #[arg(short, long)]
        create_folder: bool,

        /// How to handle duplicates: skip, overwrite, or rename
        #[arg(long, default_value = "rename")]
        duplicate_mode: String,
    },

    /// Download files or folders from pCloud
    Download {
        /// Files to download (or use --all)
        files: Vec<String>,

        /// Remote folder path
        #[arg(short = 'd', long, default_value = "/")]
        remote_path: String,

        /// Local destination path
        #[arg(short = 'o', long, default_value = "./downloads")]
        local_path: String,

        /// Download all files from remote folder
        #[arg(short, long)]
        all: bool,

        /// Download entire folder recursively
        #[arg(long)]
        recursive: bool,

        /// How to handle duplicates: skip, overwrite, or rename
        #[arg(long, default_value = "rename")]
        duplicate_mode: String,
    },

    /// List folder contents
    List {
        /// Folder path to list
        #[arg(default_value = "/")]
        path: String,
    },

    /// Create a folder
    CreateFolder {
        /// Folder path to create
        path: String,
    },

    /// Delete a file or folder
    Delete {
        /// Remote path to delete
        path: String,

        /// Delete a folder (and all contents) instead of a file
        #[arg(short, long)]
        folder: bool,

        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Move or rename a file or folder
    Move {
        /// Source path
        from: String,

        /// Destination path
        to: String,

        /// Move a folder instead of a file
        #[arg(short, long)]
        folder: bool,
    },

    /// Show account status and quota information
    Status,

    /// Sync a local folder with a remote folder
    Sync {
        /// Local folder path
        local_path: String,

        /// Remote folder path
        #[arg(short = 'd', long, default_value = "/")]
        remote_path: String,

        /// Sync direction: upload, download, or both
        #[arg(long, default_value = "both")]
        direction: String,

        /// Use checksums for comparison (slower but more accurate)
        #[arg(long)]
        checksum: bool,

        /// Sync recursively through subfolders
        #[arg(short, long)]
        recursive: bool,
    },

    /// Resume an interrupted transfer
    Resume {
        /// Path to the transfer state file (.transfer-state.json)
        state_file: String,
    },
}

fn parse_region(region_str: &str) -> Region {
    match region_str.to_lowercase().as_str() {
        "eu" => Region::EU,
        _ => Region::US,
    }
}

fn parse_duplicate_mode(mode_str: &str) -> DuplicateMode {
    match mode_str.to_lowercase().as_str() {
        "skip" => DuplicateMode::Skip,
        "overwrite" => DuplicateMode::Overwrite,
        _ => DuplicateMode::Rename,
    }
}

fn parse_sync_direction(direction_str: &str) -> SyncDirection {
    match direction_str.to_lowercase().as_str() {
        "upload" => SyncDirection::Upload,
        "download" => SyncDirection::Download,
        _ => SyncDirection::Bidirectional,
    }
}

fn format_size(size: u64) -> String {
    let mut size = size as f64;
    for unit in ["B", "KB", "MB", "GB", "TB"] {
        if size < 1024.0 {
            return format!("{:.2} {}", size, unit);
        }
        size /= 1024.0;
    }
    format!("{:.2} PB", size)
}

async fn authenticate_client(
    username: Option<String>,
    password: Option<String>,
    token: Option<String>,
    region: Region,
    workers: usize,
) -> Result<PCloudClient, Box<dyn std::error::Error>> {
    let mut client = PCloudClient::new(token.clone(), region, workers);

    // If we have a token, use it
    if let Some(t) = token {
        client.set_token(t);
        return Ok(client);
    }

    // Otherwise, authenticate with username/password
    if let (Some(user), Some(pass)) = (username, password) {
        match client.login(&user, &pass).await {
            Ok(token) => {
                println!("✓ Authenticated successfully");
                println!("✓ Token: {}", token);
                return Ok(client);
            }
            Err(e) => {
                eprintln!("✗ Authentication failed: {}", e);
                process::exit(1);
            }
        }
    }

    eprintln!("✗ Authentication required!");
    eprintln!("Provide either:");
    eprintln!("  1. --username and --password");
    eprintln!("  2. --token");
    eprintln!("  3. Set PCLOUD_USERNAME and PCLOUD_PASSWORD environment variables");
    eprintln!("  4. Set PCLOUD_TOKEN environment variable");
    process::exit(1);
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Initialize logging
    let filter = if cli.verbose {
        EnvFilter::new("pcloud_rust=debug,pcloud_cli=debug")
    } else {
        EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("pcloud_rust=info,pcloud_cli=info"))
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    let region = parse_region(&cli.region);

    match cli.command {
        Commands::Upload {
            files,
            remote_path,
            create_folder,
            duplicate_mode,
        } => {
            let mut client =
                authenticate_client(cli.username, cli.password, cli.token, region, cli.workers)
                    .await?;

            client.set_duplicate_mode(parse_duplicate_mode(&duplicate_mode));

            if create_folder {
                match client.create_folder(&remote_path).await {
                    Ok(_) => println!("✓ Created folder: {}", remote_path),
                    Err(e) => eprintln!("Warning: Could not create folder: {}", e),
                }
            }

            // Process files and directories
            let mut upload_tasks = Vec::new();

            for file_path in &files {
                let path = Path::new(file_path);

                if !path.exists() {
                    eprintln!("✗ Not found: {}", file_path);
                    continue;
                }

                if path.is_dir() {
                    // Upload entire directory tree
                    println!("📁 Scanning directory: {}", file_path);
                    match client
                        .upload_folder_tree(file_path.clone(), remote_path.clone())
                        .await
                    {
                        Ok(tasks) => {
                            println!("   Found {} files to upload", tasks.len());
                            upload_tasks.extend(tasks);
                        }
                        Err(e) => {
                            eprintln!("✗ Error scanning {}: {}", file_path, e);
                        }
                    }
                } else {
                    // Single file upload
                    upload_tasks.push((file_path.clone(), remote_path.clone()));
                }
            }

            if upload_tasks.is_empty() {
                eprintln!("✗ No files to upload");
                process::exit(1);
            }

            println!("\n📤 Uploading {} files...\n", upload_tasks.len());
            let (uploaded, failed) = client.upload_files(upload_tasks).await;

            println!(
                "\n✓ Upload complete: {} uploaded, {} failed",
                uploaded, failed
            );

            if failed > 0 {
                process::exit(1);
            }
        }

        Commands::Download {
            files,
            remote_path,
            local_path,
            all,
            recursive,
            duplicate_mode,
        } => {
            let mut client =
                authenticate_client(cli.username, cli.password, cli.token, region, cli.workers)
                    .await?;

            client.set_duplicate_mode(parse_duplicate_mode(&duplicate_mode));

            // Create local directory
            tokio::fs::create_dir_all(&local_path).await?;

            let mut download_tasks = Vec::new();

            if recursive {
                // Download entire folder tree
                if files.is_empty() {
                    eprintln!("✗ Specify folder name to download recursively");
                    process::exit(1);
                }

                for folder_name in &files {
                    let full_remote_path = if remote_path == "/" {
                        format!("/{}", folder_name)
                    } else {
                        format!("{}/{}", remote_path.trim_end_matches('/'), folder_name)
                    };

                    println!("📁 Scanning remote folder: {}", full_remote_path);
                    match client
                        .download_folder_tree(full_remote_path, local_path.clone())
                        .await
                    {
                        Ok(tasks) => {
                            println!("   Found {} files to download", tasks.len());
                            download_tasks.extend(tasks);
                        }
                        Err(e) => {
                            eprintln!("✗ Error scanning {}: {}", folder_name, e);
                        }
                    }
                }
            } else if all {
                // Download all files from remote folder (non-recursive)
                match client.list_folder(&remote_path).await {
                    Ok(items) => {
                        for item in items {
                            if !item.isfolder {
                                let full_remote_path = if remote_path == "/" {
                                    format!("/{}", item.name)
                                } else {
                                    format!("{}/{}", remote_path.trim_end_matches('/'), item.name)
                                };
                                download_tasks.push((full_remote_path, local_path.clone()));
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("✗ Error listing folder: {}", e);
                        process::exit(1);
                    }
                }
            } else {
                // Download specific files
                for filename in &files {
                    let full_remote_path = if remote_path == "/" {
                        format!("/{}", filename)
                    } else {
                        format!("{}/{}", remote_path.trim_end_matches('/'), filename)
                    };
                    download_tasks.push((full_remote_path, local_path.clone()));
                }
            }

            if download_tasks.is_empty() {
                eprintln!("✗ No files to download");
                process::exit(1);
            }

            println!("\n📥 Downloading {} files...\n", download_tasks.len());
            let (downloaded, failed) = client.download_files(download_tasks).await;

            println!(
                "\n✓ Download complete: {} downloaded, {} failed",
                downloaded, failed
            );

            if failed > 0 {
                process::exit(1);
            }
        }

        Commands::List { path } => {
            let client =
                authenticate_client(cli.username, cli.password, cli.token, region, cli.workers)
                    .await?;

            match client.list_folder(&path).await {
                Ok(items) => {
                    if items.is_empty() {
                        println!("Folder '{}' is empty", path);
                        return Ok(());
                    }

                    println!("\nContents of '{}':\n", path);
                    println!("{:<10} {:<40} {:<15}", "Type", "Name", "Size");
                    println!("{}", "-".repeat(70));

                    for item in items {
                        let item_type = if item.isfolder { "DIR" } else { "FILE" };
                        let size_str = if item.isfolder {
                            "-".to_string()
                        } else {
                            format_size(item.size)
                        };

                        println!("{:<10} {:<40} {:<15}", item_type, item.name, size_str);
                    }

                    println!();
                }
                Err(e) => {
                    eprintln!("✗ Error listing folder: {}", e);
                    process::exit(1);
                }
            }
        }

        Commands::CreateFolder { path } => {
            let client =
                authenticate_client(cli.username, cli.password, cli.token, region, cli.workers)
                    .await?;

            match client.create_folder(&path).await {
                Ok(_) => {
                    println!("✓ Created folder: {}", path);
                }
                Err(e) => {
                    eprintln!("✗ Error creating folder: {}", e);
                    process::exit(1);
                }
            }
        }

        Commands::Delete { path, folder, yes } => {
            let client =
                authenticate_client(cli.username, cli.password, cli.token, region, cli.workers)
                    .await?;

            // Confirmation prompt unless --yes is specified
            if !yes {
                let item_type = if folder { "folder" } else { "file" };
                eprintln!(
                    "⚠️  Are you sure you want to delete {} '{}'?",
                    item_type, path
                );
                if folder {
                    eprintln!("   This will delete all contents recursively!");
                }
                eprint!("Type 'yes' to confirm: ");

                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                if input.trim().to_lowercase() != "yes" {
                    eprintln!("Aborted.");
                    process::exit(0);
                }
            }

            let result = if folder {
                client.delete_folder(&path).await
            } else {
                client.delete_file(&path).await
            };

            match result {
                Ok(_) => {
                    let item_type = if folder { "folder" } else { "file" };
                    println!("✓ Deleted {}: {}", item_type, path);
                }
                Err(e) => {
                    eprintln!("✗ Error deleting: {}", e);
                    process::exit(1);
                }
            }
        }

        Commands::Move { from, to, folder } => {
            let client =
                authenticate_client(cli.username, cli.password, cli.token, region, cli.workers)
                    .await?;

            let result = if folder {
                client.rename_folder(&from, &to).await
            } else {
                client.rename_file(&from, &to).await
            };

            match result {
                Ok(_) => {
                    println!("✓ Moved: {} -> {}", from, to);
                }
                Err(e) => {
                    eprintln!("✗ Error moving: {}", e);
                    process::exit(1);
                }
            }
        }

        Commands::Status => {
            let client =
                authenticate_client(cli.username, cli.password, cli.token, region, cli.workers)
                    .await?;

            match client.get_account_info().await {
                Ok(info) => {
                    println!("\n📊 Account Status\n");
                    println!("Email:     {}", info.email);
                    println!(
                        "Plan:      {}",
                        if info.premium { "Premium" } else { "Free" }
                    );
                    println!();
                    println!("Storage:");
                    println!("  Used:      {}", format_size(info.used_quota));
                    println!("  Available: {}", format_size(info.available()));
                    println!("  Total:     {}", format_size(info.quota));
                    println!("  Usage:     {:.1}%", info.usage_percent());
                    println!();

                    // Visual progress bar
                    let bar_width = 40;
                    let filled = (info.usage_percent() / 100.0 * bar_width as f64) as usize;
                    let empty = bar_width - filled;
                    println!("  [{}{}]", "█".repeat(filled), "░".repeat(empty));
                    println!();
                }
                Err(e) => {
                    eprintln!("✗ Error getting account info: {}", e);
                    process::exit(1);
                }
            }
        }

        Commands::Sync {
            local_path,
            remote_path,
            direction,
            checksum,
            recursive,
        } => {
            let client =
                authenticate_client(cli.username, cli.password, cli.token, region, cli.workers)
                    .await?;

            let sync_direction = parse_sync_direction(&direction);

            // Validate local path exists
            if !Path::new(&local_path).exists() {
                eprintln!("✗ Local path does not exist: {}", local_path);
                process::exit(1);
            }

            let direction_str = match sync_direction {
                SyncDirection::Upload => "upload only",
                SyncDirection::Download => "download only",
                SyncDirection::Bidirectional => "bidirectional",
            };

            println!("\n🔄 Syncing folders...");
            println!("   Local:     {}", local_path);
            println!("   Remote:    {}", remote_path);
            println!("   Direction: {}", direction_str);
            println!(
                "   Checksum:  {}",
                if checksum {
                    "enabled"
                } else {
                    "disabled (size comparison)"
                }
            );
            println!("   Recursive: {}", if recursive { "yes" } else { "no" });
            println!();

            let result = if recursive {
                client
                    .sync_folder_recursive(&local_path, &remote_path, sync_direction, checksum)
                    .await
            } else {
                client
                    .sync_folder(&local_path, &remote_path, sync_direction, checksum)
                    .await
            };

            match result {
                Ok(sync_result) => {
                    println!("\n✓ Sync complete!");
                    println!("  Uploaded:   {} files", sync_result.uploaded);
                    println!("  Downloaded: {} files", sync_result.downloaded);
                    println!("  Skipped:    {} files", sync_result.skipped);
                    if sync_result.failed > 0 {
                        println!("  Failed:     {} files", sync_result.failed);
                    }
                    println!();

                    if sync_result.failed > 0 {
                        process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!("✗ Sync failed: {}", e);
                    process::exit(1);
                }
            }
        }

        Commands::Resume { state_file } => {
            // Load transfer state
            let mut state = match TransferState::load_from_file(&state_file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("✗ Failed to load transfer state: {}", e);
                    process::exit(1);
                }
            };

            println!("\n🔄 Resuming transfer...");
            println!("   Transfer ID: {}", state.id);
            println!("   Direction:   {}", state.direction);
            println!(
                "   Completed:   {}/{} files",
                state.completed_files.len(),
                state.total_files
            );
            println!("   Pending:     {} files", state.pending_files.len());
            println!("   Failed:      {} files", state.failed_files.len());
            println!();

            if state.pending_files.is_empty() {
                println!("✓ Transfer already complete!");
                return Ok(());
            }

            let client =
                authenticate_client(cli.username, cli.password, cli.token, region, cli.workers)
                    .await?;

            let bytes_progress = Arc::new(AtomicU64::new(0));
            let bp_clone = bytes_progress.clone();

            // Progress display task
            let progress_handle = tokio::spawn(async move {
                let mut last_bytes = 0u64;
                let start = std::time::Instant::now();
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    let current = bp_clone.load(Ordering::Relaxed);
                    let elapsed = start.elapsed().as_secs_f64();
                    let speed = if elapsed > 0.0 {
                        current as f64 / elapsed / 1_000_000.0
                    } else {
                        0.0
                    };
                    if current != last_bytes {
                        print!(
                            "\r  Progress: {} ({:.2} MB/s)     ",
                            format_size(current),
                            speed
                        );
                        let _ = std::io::Write::flush(&mut std::io::stdout());
                        last_bytes = current;
                    }
                }
            });

            let (completed, failed) = if state.direction == "upload" {
                client
                    .resume_upload(&mut state, bytes_progress.clone(), None)
                    .await
            } else {
                client
                    .resume_download(&mut state, bytes_progress.clone(), None)
                    .await
            };

            progress_handle.abort();
            println!();

            // Save updated state
            if let Err(e) = state.save_to_file(&state_file) {
                eprintln!("Warning: Could not save transfer state: {}", e);
            }

            println!("\n✓ Resume complete!");
            println!("  Completed: {} files", completed);
            if failed > 0 {
                println!("  Failed:    {} files", failed);
            }
            println!();

            if !state.pending_files.is_empty() {
                println!(
                    "Note: {} files still pending. Run resume again to continue.",
                    state.pending_files.len()
                );
            }

            if failed > 0 {
                process::exit(1);
            }
        }
    }

    Ok(())
}
