use clap::{Parser, Subcommand};
use pcloud_rust::{PCloudClient, Region, DuplicateMode};
use std::path::Path;
use std::process;

#[derive(Parser)]
#[command(name = "pcloud-cli")]
#[command(author, version, about = "pCloud Fast Transfer CLI - High-performance upload/download tool", long_about = None)]
struct Cli {
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

    let region = parse_region(&cli.region);

    match cli.command {
        Commands::Upload {
            files,
            remote_path,
            create_folder,
            duplicate_mode,
        } => {
            let mut client = authenticate_client(
                cli.username,
                cli.password,
                cli.token,
                region,
                cli.workers,
            )
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
            let mut client = authenticate_client(
                cli.username,
                cli.password,
                cli.token,
                region,
                cli.workers,
            )
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
            let client = authenticate_client(
                cli.username,
                cli.password,
                cli.token,
                region,
                cli.workers,
            )
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
            let client = authenticate_client(
                cli.username,
                cli.password,
                cli.token,
                region,
                cli.workers,
            )
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
    }

    Ok(())
}
