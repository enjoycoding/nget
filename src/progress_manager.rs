use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressState, ProgressStyle};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Manages progress bar display for download operations
#[derive(Clone)]
pub struct ProgressManager {
    multi_progress: Arc<MultiProgress>,
    main_progress: Arc<Mutex<Option<ProgressBar>>>,
    chunk_progresses: Arc<Mutex<Vec<ProgressBar>>>,
    start_time: Arc<Mutex<Option<Instant>>>,
    downloaded_bytes: Arc<Mutex<u64>>,
    last_update: Arc<Mutex<Instant>>,
}

impl ProgressManager {
    pub fn new() -> Self {
        let multi_progress = MultiProgress::new();

        // Set more frequent refresh rate (every 100ms)
        multi_progress.set_draw_target(ProgressDrawTarget::stdout_with_hz(10));

        Self {
            multi_progress: Arc::new(multi_progress),
            main_progress: Arc::new(Mutex::new(None)),
            chunk_progresses: Arc::new(Mutex::new(Vec::new())),
            start_time: Arc::new(Mutex::new(None)),
            downloaded_bytes: Arc::new(Mutex::new(0)),
            last_update: Arc::new(Mutex::new(Instant::now())),
        }
    }

    /// Initialize main progress bar
    pub async fn init_main_progress(&self, total_size: u64, initial_downloaded: u64) {
        let pb = self.multi_progress.add(ProgressBar::new(total_size));
        pb.set_position(initial_downloaded);

        // Create rich progress bar style
        pb.set_style(
            ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes:>9}/{total_bytes:>9} ({percent:>3}%) {binary_bytes_per_sec:>12} ETA: {eta:>6} {msg}")
                .unwrap()
                .progress_chars("█▉▊▋▌▍▎▏  ")
                .with_key("eta", |state: &ProgressState, w: &mut dyn std::fmt::Write| {
                    write!(w, "{:.1}s", state.eta().as_secs_f64()).unwrap()
                })
                .with_key("binary_bytes_per_sec", |state: &ProgressState, w: &mut dyn std::fmt::Write| {
                    write!(w, "{:.1}/s", humansize::format_size(state.per_sec() as u64, humansize::BINARY)).unwrap()
                })
        );

        // Set initial message
        pb.set_message("Downloading...".to_string());

        *self.main_progress.lock().await = Some(pb);
        *self.start_time.lock().await = Some(Instant::now());
    }

    /// Add progress bar for a download chunk
    pub async fn add_chunk_progress(&self, thread_id: usize, total_size: u64) -> ProgressBar {
        let pb = self.multi_progress.add(ProgressBar::new(total_size));

        pb.set_style(
            ProgressStyle::with_template(&format!(
                "Thread {:2}: [{{bar:30.cyan}}] {{bytes:>7}}/{{total_bytes:>7}} ({{percent:>2}}%)",
                thread_id
            ))
            .unwrap()
            .progress_chars("█▉▊▋▌▍▎▏ "),
        );

        let mut chunk_progresses = self.chunk_progresses.lock().await;
        if chunk_progresses.len() <= thread_id {
            chunk_progresses.resize(thread_id + 1, ProgressBar::hidden());
        }
        chunk_progresses[thread_id] = pb.clone();

        pb
    }

    /// Update main progress bar
    pub async fn update_main_progress(&self, increment: u64) {
        if let Some(pb) = &*self.main_progress.lock().await {
            pb.inc(increment);

            // Update downloaded bytes counter
            let mut downloaded = self.downloaded_bytes.lock().await;
            *downloaded += increment;

            // Update last update time
            *self.last_update.lock().await = Instant::now();

            // Calculate and display download speed
            if let Some(start_time) = *self.start_time.lock().await {
                let elapsed = start_time.elapsed().as_secs_f64();
                if elapsed > 0.0 {
                    let speed = *downloaded as f64 / elapsed;
                    let speed_str = format!(
                        "{:.1}/s",
                        humansize::format_size(speed as u64, humansize::BINARY)
                    );

                    // Update message with more information
                    let msg = format!(
                        "Speed: {} | Active threads: {}",
                        speed_str,
                        self.get_active_threads_count().await
                    );
                    pb.set_message(msg);
                }
            }
        }
    }

    /// Finish main progress bar with completion message
    pub async fn finish_main_progress(&self, message: &str) {
        if let Some(pb) = &*self.main_progress.lock().await {
            pb.finish_with_message(message.to_string());

            // Hide all chunk progress bars
            let mut chunk_progresses = self.chunk_progresses.lock().await;
            for chunk_pb in chunk_progresses.iter_mut() {
                chunk_pb.finish_and_clear();
            }
        }
    }

    /// Set main progress bar message
    pub async fn set_main_message(&self, message: &str) {
        if let Some(pb) = &*self.main_progress.lock().await {
            pb.set_message(message.to_string());
        }
    }

    /// Get count of active download threads
    pub async fn get_active_threads_count(&self) -> usize {
        let chunk_progresses = self.chunk_progresses.lock().await;
        chunk_progresses
            .iter()
            .filter(|pb| !pb.is_finished())
            .count()
    }

    /// Check if download seems stalled
    pub async fn check_stalled(&self) -> bool {
        let last_update = *self.last_update.lock().await;
        last_update.elapsed() > Duration::from_secs(30) // Consider stalled after 30 seconds
    }

    /// Get total downloaded bytes
    pub async fn get_total_downloaded(&self) -> u64 {
        *self.downloaded_bytes.lock().await
    }

    /// Create simple progress bar for single-thread downloads
    pub fn create_simple_progress(&self, total_size: u64) -> ProgressBar {
        let pb = self.multi_progress.add(ProgressBar::new(total_size));

        pb.set_style(
            ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] [{bar:40}] {bytes:>9}/{total_bytes:>9} ({percent:>3}%) {binary_bytes_per_sec:>12} ETA: {eta:>6}")
                .unwrap()
                .progress_chars("█▉▊▋▌▍▎▏ ")
                .with_key("eta", |state: &ProgressState, w: &mut dyn std::fmt::Write| {
                    write!(w, "{:.1}s", state.eta().as_secs_f64()).unwrap()
                })
                .with_key("binary_bytes_per_sec", |state: &ProgressState, w: &mut dyn std::fmt::Write| {
                    write!(w, "{:.1}/s", humansize::format_size(state.per_sec() as u64, humansize::BINARY)).unwrap()
                })
        );

        pb
    }
}
