// src/progress_manager.rs
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
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
    last_downloaded: Arc<Mutex<u64>>, // 用于计算瞬时速度
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
            last_downloaded: Arc::new(Mutex::new(0)),
        }
    }

    /// Initialize main progress bar
    pub async fn init_main_progress(&self, total_size: u64, initial_downloaded: u64) {
        let pb = self.multi_progress.add(ProgressBar::new(total_size));
        pb.set_position(initial_downloaded);

        // 创建更丰富的进度条样式
        pb.set_style(
            ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes:>9}/{total_bytes:>9} ({percent:>3}%) {msg}")
                .unwrap()
                .progress_chars("█▉▊▋▌▍▎▏  ")
        );

        // 设置消息显示当前状态
        pb.set_message("Initializing...".to_string());

        *self.main_progress.lock().await = Some(pb);
        *self.start_time.lock().await = Some(Instant::now());
        *self.downloaded_bytes.lock().await = initial_downloaded;
        *self.last_downloaded.lock().await = initial_downloaded;
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

    /// Format speed with proper units
    fn format_speed(speed_bytes_per_sec: f64) -> String {
        const KB: f64 = 1024.0;
        const MB: f64 = 1024.0 * 1024.0;
        const GB: f64 = 1024.0 * 1024.0 * 1024.0;

        if speed_bytes_per_sec >= GB {
            format!("{:.1} GiB/s", speed_bytes_per_sec / GB)
        } else if speed_bytes_per_sec >= MB {
            format!("{:.1} MiB/s", speed_bytes_per_sec / MB)
        } else if speed_bytes_per_sec >= KB {
            format!("{:.1} KiB/s", speed_bytes_per_sec / KB)
        } else {
            format!("{:.0} B/s", speed_bytes_per_sec)
        }
    }

    /// Update main progress bar with accurate speed calculation
    pub async fn update_main_progress(&self, increment: u64) {
        if let Some(pb) = &*self.main_progress.lock().await {
            pb.inc(increment);

            // 更新下载字节数
            let mut downloaded = self.downloaded_bytes.lock().await;
            *downloaded += increment;

            let now = Instant::now();
            let last_update = *self.last_update.lock().await;
            let mut last_downloaded = self.last_downloaded.lock().await;

            // 计算瞬时速度（基于最近2秒的数据，更稳定）
            let time_diff = now.duration_since(last_update).as_secs_f64();

            if time_diff >= 2.0 {
                // 至少2秒才更新速度，避免抖动
                let downloaded_diff = *downloaded - *last_downloaded;
                let speed_bytes_per_sec = (downloaded_diff as f64) / time_diff;

                // 格式化速度显示
                let speed_str = if speed_bytes_per_sec > 0.0 {
                    Self::format_speed(speed_bytes_per_sec)
                } else {
                    "0 B/s".to_string()
                };

                let msg = format!(
                    "Speed: {} | Active threads: {}",
                    speed_str,
                    self.get_active_threads_count().await
                );
                pb.set_message(msg);

                // 重置计数器和时间
                *last_downloaded = *downloaded;
                *self.last_update.lock().await = now;
            } else if time_diff >= 0.5 {
                // 如果时间较短但已经有数据，显示估算速度
                let downloaded_diff = *downloaded - *last_downloaded;
                if downloaded_diff > 0 {
                    let speed_bytes_per_sec = (downloaded_diff as f64) / time_diff;
                    let speed_str = Self::format_speed(speed_bytes_per_sec);

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
            // 计算平均速度
            if let Some(start_time) = *self.start_time.lock().await {
                let elapsed = start_time.elapsed().as_secs_f64();
                let total_downloaded = *self.downloaded_bytes.lock().await;

                if elapsed > 0.0 {
                    let avg_speed = total_downloaded as f64 / elapsed;
                    let avg_speed_str = Self::format_speed(avg_speed);

                    println!("📊 Average speed: {}", avg_speed_str);
                }
            }

            pb.finish_with_message(message.to_string());

            // 隐藏所有分块进度条
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
        last_update.elapsed() > Duration::from_secs(120) // 2分钟无活动认为卡住
    }

    /// Get total downloaded bytes
    pub async fn get_total_downloaded(&self) -> u64 {
        *self.downloaded_bytes.lock().await
    }

    /// Create simple progress bar for single-thread downloads
    pub fn create_simple_progress(&self, total_size: u64) -> ProgressBar {
        let pb = self.multi_progress.add(ProgressBar::new(total_size));

        pb.set_style(
            ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] [{bar:40}] {bytes:>9}/{total_bytes:>9} ({percent:>3}%) {msg}")
                .unwrap()
                .progress_chars("█▉▊▋▌▍▎▏ ")
        );

        pb
    }

    /// 手动更新单线程下载的速度显示
    pub async fn update_simple_progress_speed(&self, pb: &ProgressBar, downloaded: u64) {
        if let Some(start_time) = *self.start_time.lock().await {
            let elapsed = start_time.elapsed().as_secs_f64();
            if elapsed > 0.0 {
                let speed = downloaded as f64 / elapsed;
                let speed_str = Self::format_speed(speed);
                pb.set_message(format!("Speed: {}", speed_str));
            }
        }
    }

    // 新增公共方法：设置下载字节数
    pub async fn set_downloaded_bytes(&self, bytes: u64) {
        *self.downloaded_bytes.lock().await = bytes;
    }

    // 新增公共方法：设置开始时间
    pub async fn set_start_time(&self, start_time: Instant) {
        *self.start_time.lock().await = Some(start_time);
    }

    // // 新增公共方法：获取开始时间
    // pub async fn get_start_time(&self) -> Option<Instant> {
    //     *self.start_time.lock().await
    // }
}
