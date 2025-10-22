use anyhow::{anyhow, Result};
use futures::StreamExt;
use reqwest::{header, Client};
use std::cmp::min;
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use crate::config::{DownloadConfig, DownloadRange as ConfigRange, DownloadState};
use crate::progress_manager::ProgressManager;
use crate::state_manager::StateManager;

/// Main downloader implementation
pub struct Downloader {
    pub(crate) client: Client,
    pub(crate) config: DownloadConfig,
    state_manager: StateManager,
    progress_manager: ProgressManager,
}

impl Downloader {
    pub fn new(config: DownloadConfig) -> Result<Self> {
        let mut client_builder = Client::builder().user_agent(&config.user_agent);
        if config.timeout > 0 {
            client_builder = client_builder.timeout(Duration::from_secs(config.timeout));
        }
        let client = client_builder.build()?;

        let state_manager = StateManager::new(&config.output_path);
        let progress_manager = ProgressManager::new();

        Ok(Self {
            client,
            config,
            state_manager,
            progress_manager,
        })
    }

    /// Main download entry point
    pub async fn download(&self) -> Result<()> {
        println!("Starting download: {}", self.config.url);

        // Check if we should resume a previous download
        let should_resume =
            self.config.resume && self.state_manager.state_exists() && !self.config.force;

        if should_resume {
            println!("Resuming previous download...");
            self.resume_download().await
        } else {
            if self.config.force && self.state_manager.state_exists() {
                self.state_manager.cleanup()?;
            }
            println!("Starting new download...");
            self.new_download().await
        }
    }

    /// Start a new download from scratch
    async fn new_download(&self) -> Result<()> {
        let (supports_range, etag, last_modified, total_size) = self.check_download_info().await?;

        if supports_range && self.config.num_threads > 1 {
            self.concurrent_download(total_size, etag, last_modified, false)
                .await
        } else {
            self.single_thread_download(false).await
        }
    }

    /// Resume an interrupted download
    async fn resume_download(&self) -> Result<()> {
        let state = self
            .state_manager
            .load_state()?
            .ok_or_else(|| anyhow!("No download state found"))?;

        // Verify URL matches
        if state.url != self.config.url {
            return Err(anyhow!(
                "URL mismatch. Cannot resume download from different URL"
            ));
        }

        println!(
            "Resuming download: {} bytes remaining",
            state.ranges.iter().map(|r| r.remaining()).sum::<u64>()
        );

        // Check if server resource has changed
        let (supports_range, current_etag, current_last_modified, total_size) =
            self.check_download_info().await?;

        // Verify resource hasn't changed
        if let Some(etag) = &state.etag {
            if current_etag.as_ref() != Some(etag) {
                return Err(anyhow!("File has changed on server. Cannot resume."));
            }
        }

        if let Some(last_modified) = &state.last_modified {
            if current_last_modified.as_ref() != Some(last_modified) {
                return Err(anyhow!("File has been modified on server. Cannot resume."));
            }
        }

        if total_size != state.total_size {
            return Err(anyhow!("File size has changed. Cannot resume."));
        }

        if supports_range && self.config.num_threads > 1 {
            self.concurrent_download(total_size, current_etag, current_last_modified, true)
                .await
        } else {
            self.single_thread_download(true).await
        }
    }

    /// Check server capabilities and file information
    async fn check_download_info(&self) -> Result<(bool, Option<String>, Option<String>, u64)> {
        let response = self.client.head(&self.config.url).send().await?;

        let supports_range = response
            .headers()
            .get(header::ACCEPT_RANGES)
            .map(|h| h == "bytes")
            .unwrap_or(false);

        let etag = response
            .headers()
            .get(header::ETAG)
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());

        let last_modified = response
            .headers()
            .get(header::LAST_MODIFIED)
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());

        let content_length = response
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(|| anyhow!("Could not determine content length"))?;

        Ok((supports_range, etag, last_modified, content_length))
    }

    /// Concurrent multi-threaded download implementation
    async fn concurrent_download(
        &self,
        total_size: u64,
        etag: Option<String>,
        last_modified: Option<String>,
        is_resume: bool,
    ) -> Result<()> {
        println!(
            "🚀 Starting {}download",
            if is_resume { "resumed " } else { "" }
        );
        println!(
            "📦 Total size: {}",
            humansize::format_size(total_size, humansize::BINARY)
        );
        println!("🎯 Threads: {}", self.config.num_threads);

        // Create or open file
        let file = if is_resume {
            std::fs::OpenOptions::new()
                .write(true)
                .open(&self.config.output_path)?
        } else {
            let file = File::create(&self.config.output_path)?;
            file.set_len(total_size)?;
            file
        };

        // Calculate download ranges
        let ranges = if is_resume {
            let state = self
                .state_manager
                .load_state()?
                .ok_or_else(|| anyhow!("No state found for resume"))?;
            state.ranges
        } else {
            let chunk_size = total_size / self.config.num_threads as u64;
            let mut ranges = Vec::new();

            for i in 0..self.config.num_threads {
                let start = i as u64 * chunk_size;
                let end = if i == self.config.num_threads - 1 {
                    total_size - 1
                } else {
                    start + chunk_size - 1
                };
                ranges.push(ConfigRange::new(start, end));
            }
            ranges
        };

        // Calculate initial downloaded size
        let initial_downloaded: u64 = ranges.iter().map(|r| r.downloaded).sum();

        // Initialize progress bars
        self.progress_manager
            .init_main_progress(total_size, initial_downloaded)
            .await;

        if initial_downloaded > 0 {
            println!(
                "📊 Resuming from {} downloaded",
                humansize::format_size(initial_downloaded, humansize::BINARY)
            );
        }

        // Create initial state
        let initial_state = DownloadState {
            url: self.config.url.clone(),
            output_path: self.config.output_path.clone(),
            total_size,
            ranges: ranges.clone(),
            etag: etag.clone(),
            last_modified: last_modified.clone(),
        };

        if !is_resume {
            self.state_manager.save_state(&initial_state)?;
        }

        let mut handles = Vec::new();
        let shared_file = Arc::new(Mutex::new(file));
        let shared_ranges = Arc::new(Mutex::new(ranges));
        let shared_state_manager = Arc::new(self.state_manager.clone());
        let progress_manager = self.progress_manager.clone();

        // Create progress bars for each chunk
        {
            let ranges = shared_ranges.lock().await;
            for (thread_id, range) in ranges.iter().enumerate() {
                if !range.is_completed() {
                    let chunk_size = range.end - range.start + 1;
                    progress_manager
                        .add_chunk_progress(thread_id, chunk_size)
                        .await;
                }
            }
        }

        // Start download threads
        for thread_id in 0..self.config.num_threads {
            let client = self.client.clone();
            let url = self.config.url.clone();
            let file = Arc::clone(&shared_file);
            let ranges = Arc::clone(&shared_ranges);
            let state_manager = Arc::clone(&shared_state_manager);
            let progress_manager = progress_manager.clone();

            let handle = tokio::spawn(async move {
                Self::download_chunk(
                    client,
                    url,
                    file,
                    ranges,
                    thread_id,
                    progress_manager,
                    state_manager,
                )
                .await
            });
            handles.push(handle);
        }

        // Start monitoring task to check for stalls
        let monitor_handle = self.start_progress_monitor();

        // Wait for all threads to complete
        let mut results = Vec::new();
        for handle in handles {
            results.push(handle.await?);
        }

        // Stop monitoring task
        monitor_handle.abort();

        // Check for errors
        for result in results {
            if let Err(e) = result {
                eprintln!("❌ Download error: {}", e);
                return Err(e);
            }
        }

        // Clean up state file
        self.state_manager.cleanup()?;
        self.progress_manager
            .finish_main_progress("✅ Download completed!")
            .await;

        let total_downloaded = self.progress_manager.get_total_downloaded().await;
        println!(
            "🎉 Download finished: {} successfully downloaded",
            humansize::format_size(total_downloaded, humansize::BINARY)
        );

        Ok(())
    }

    /// Download a single chunk of the file
    async fn download_chunk(
        client: Client,
        url: String,
        file: Arc<Mutex<File>>,
        ranges: Arc<Mutex<Vec<ConfigRange>>>,
        thread_id: usize,
        progress_manager: ProgressManager,
        state_manager: Arc<StateManager>,
    ) -> Result<()> {
        let (range, start_pos) = {
            let ranges = ranges.lock().await;
            let range = &ranges[thread_id];

            // If this range is already completed, return early
            if range.is_completed() {
                return Ok(());
            }

            (range.clone(), range.start + range.downloaded)
        };

        let range_header = format!("bytes={}-{}", start_pos, range.end);

        let response = client
            .get(&url)
            .header(header::RANGE, range_header)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(anyhow!("HTTP error: {}", response.status()));
        }

        let mut stream = response.bytes_stream();
        let mut downloaded_in_chunk = 0;
        let mut last_save = 0u64;

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result?;
            let chunk_size = chunk.len() as u64;

            // Write to file
            {
                let mut file = file.lock().await;
                file.seek(SeekFrom::Start(start_pos + downloaded_in_chunk))?;
                file.write_all(&chunk)?;
            }

            // Update progress
            downloaded_in_chunk += chunk_size;

            // Update main progress bar
            progress_manager.update_main_progress(chunk_size).await;

            // Update range state
            {
                let mut ranges = ranges.lock().await;
                let range = &mut ranges[thread_id];
                range.downloaded += chunk_size;

                // Periodically save state (every 5MB)
                if downloaded_in_chunk - last_save > 5 * 1024 * 1024 {
                    if let Ok(Some(mut state)) = state_manager.load_state() {
                        state.ranges = ranges.clone();
                        let _ = state_manager.save_state(&state);
                    }
                    last_save = downloaded_in_chunk;
                }
            }
        }

        // Mark range as completed and save final state
        {
            let mut ranges = ranges.lock().await;
            ranges[thread_id].completed = true;

            if let Ok(Some(mut state)) = state_manager.load_state() {
                state.ranges = ranges.clone();
                let _ = state_manager.save_state(&state);
            }
        }

        Ok(())
    }

    /// Single-threaded download implementation
    async fn single_thread_download(&self, is_resume: bool) -> Result<()> {
        println!("🔶 Using single thread download");

        let response = self.client.get(&self.config.url).send().await?;

        if !response.status().is_success() {
            return Err(anyhow!("HTTP error: {}", response.status()));
        }

        let total_size = response
            .content_length()
            .ok_or_else(|| anyhow!("Could not determine content length"))?;

        let file = if is_resume {
            std::fs::OpenOptions::new()
                .write(true)
                .open(&self.config.output_path)?
        } else {
            File::create(&self.config.output_path)?
        };

        let mut file = file;
        let start_pos = if is_resume {
            file.seek(SeekFrom::End(0))?
        } else {
            0
        };

        if start_pos >= total_size {
            println!("✅ Download already completed");
            return Ok(());
        }

        // Create progress bar
        let pb = self.progress_manager.create_simple_progress(total_size);
        pb.set_position(start_pos);

        // Set up range request for resume
        let response = if is_resume {
            let range_header = format!("bytes={}-", start_pos);
            self.client
                .get(&self.config.url)
                .header(header::RANGE, range_header)
                .send()
                .await?
        } else {
            response
        };

        let mut downloaded = start_pos;
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk)?;
            downloaded = min(downloaded + (chunk.len() as u64), total_size);
            pb.set_position(downloaded);
        }

        pb.finish_with_message("✅ Download completed!");
        println!(
            "🎉 Single thread download finished: {}",
            humansize::format_size(downloaded, humansize::BINARY)
        );

        Ok(())
    }

    /// Start a background task to monitor download progress
    fn start_progress_monitor(&self) -> tokio::task::JoinHandle<()> {
        let progress_manager = self.progress_manager.clone();

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(10)).await;

                // Check if download seems stalled
                if progress_manager.check_stalled().await {
                    progress_manager
                        .set_main_message("⚠️  Download seems stalled...")
                        .await;
                }
            }
        })
    }
}

impl Clone for Downloader {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            config: self.config.clone(),
            state_manager: self.state_manager.clone(),
            progress_manager: self.progress_manager.clone(),
        }
    }
}
