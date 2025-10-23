use clap::Parser;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Command line interface configuration
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// URL to download
    pub url: String,

    /// Output file path
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Number of concurrent threads, default is the number of CPU cores
    #[arg(short, long)]
    pub threads: Option<usize>,

    /// User agent
    #[arg(long, default_value = concat!("nget/", env!("CARGO_PKG_VERSION")))]
    pub user_agent: String,

    /// Timeout in seconds, default no timeout
    #[arg(long)]
    pub timeout: Option<u64>,

    /// Enable resume capability
    #[arg(short, long)]
    pub resume: bool,

    /// Force new download, ignore existing state
    #[arg(long)]
    pub force: bool,
}

/// Download state persistence structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadState {
    pub url: String,
    pub output_path: PathBuf,
    pub total_size: u64,
    pub ranges: Vec<DownloadRange>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

/// Download range information for concurrent downloading
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadRange {
    pub start: u64,
    pub end: u64,
    pub downloaded: u64,
    pub completed: bool,
}

impl DownloadRange {
    pub fn new(start: u64, end: u64) -> Self {
        Self {
            start,
            end,
            downloaded: 0,
            completed: false,
        }
    }

    pub fn remaining(&self) -> u64 {
        if self.completed {
            0
        } else {
            (self.end - self.start + 1) - self.downloaded
        }
    }

    pub fn is_completed(&self) -> bool {
        self.completed || self.downloaded >= (self.end - self.start + 1)
    }
}

/// Main download configuration
#[derive(Debug, Clone)]
pub struct DownloadConfig {
    pub url: String,
    pub output_path: PathBuf,
    pub num_threads: usize,
    pub user_agent: String,
    pub timeout: Option<u64>,
    pub resume: bool,
    pub force: bool,
}

impl From<Cli> for DownloadConfig {
    fn from(cli: Cli) -> Self {
        let output_path = cli
            .output
            .unwrap_or_else(|| PathBuf::from(cli.url.split('/').last().unwrap_or("download.bin")));

        Self {
            url: cli.url,
            output_path,
            num_threads: if let Some(threads) = cli.threads {
                threads
            } else {
                num_cpus::get()
            },
            user_agent: cli.user_agent,
            timeout: cli.timeout,
            resume: cli.resume,
            force: cli.force,
        }
    }
}
