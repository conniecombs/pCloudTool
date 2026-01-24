use pcloud_rust::{PCloudClient, Region};

#[tokio::main]
async fn main() {
    println!("pCloud Directory List Test");
    println!("===========================\n");

    // Get credentials from environment or prompt
    let username = std::env::var("PCLOUD_USERNAME")
        .expect("Set PCLOUD_USERNAME environment variable");
    let password = std::env::var("PCLOUD_PASSWORD")
        .expect("Set PCLOUD_PASSWORD environment variable");

    println!("Creating client...");
    let mut client = PCloudClient::new(None, Region::US, 4);

    println!("Authenticating...");
    match client.login(&username, &password).await {
        Ok(token) => {
            println!("✓ Authenticated successfully");
            println!("  Token: {}...", &token[..20.min(token.len())]);
        }
        Err(e) => {
            eprintln!("✗ Authentication failed: {}", e);
            std::process::exit(1);
        }
    }

    println!("\nListing root folder '/'...");
    match client.list_folder("/").await {
        Ok(items) => {
            println!("✓ Success! Found {} items:", items.len());
            for item in items.iter().take(10) {
                let icon = if item.isfolder { "📁" } else { "📄" };
                println!("  {} {}", icon, item.name);
            }
            if items.len() > 10 {
                println!("  ... and {} more items", items.len() - 10);
            }
        }
        Err(e) => {
            eprintln!("✗ Failed to list folder: {}", e);
            std::process::exit(1);
        }
    }

    println!("\n✓ Test completed successfully!");
}
