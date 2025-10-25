mod config;
mod downloader;
mod progress_manager;
mod state_manager;

use anyhow::Result;
use clap::Parser;
use config::Cli;
use downloader::Downloader;

#[tokio::main]
async fn main() -> Result<()> {
    // Set up better panic messages
    better_panic::install();

    // Parse command line arguments
    let cli = Cli::parse();
    let config = cli.into();

    // Create downloader instance
    let downloader = Downloader::new(config)?;

    // Set up Ctrl+C handler for graceful interruption
    ctrlc::set_handler(move || {
        println!("\n⚠️  Download interrupted by user. You can resume with --resume flag.");
        std::process::exit(1);
    })?;

    // Execute download
    if let Err(e) = downloader.download().await {
        eprintln!("❌ Download failed: {}", e);
        eprintln!("💡 You can resume the download using: --resume");
        std::process::exit(1);
    }

    Ok(())
}
