//! Basic usage example for concurrent-downloader
//!
//! This example shows how to use the downloader programmatically

use nget::{DownloadConfig, Downloader};
use std::path::PathBuf;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = DownloadConfig {
        url: "https://httpbin.org/bytes/1048576".to_string(), // 1MB test file
        output_path: PathBuf::from("test_download.bin"),
        num_threads: 4,
        user_agent: "nget/1.0".to_string(),
        timeout: 0,
        resume: true,
        force: false,
    };

    let downloader = Downloader::new(config)?;
    downloader.download().await?;

    println!("Download completed successfully!");
    Ok(())
}
