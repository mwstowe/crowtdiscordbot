use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{error, info, warn};

/// A rate limiter that enforces both per-minute and per-day limits
#[derive(Clone)]
pub struct RateLimiter {
    // Per-minute tracking
    minute_limit: u32,
    minute_requests: Arc<Mutex<VecDeque<Instant>>>,

    // Per-day tracking
    day_limit: u32,
    day_requests: Arc<Mutex<VecDeque<DateTime<Utc>>>>,

    // Persistence
    persistence_file: Option<String>,
}

impl RateLimiter {
    /// Create a new rate limiter with persistence
    pub fn new_with_persistence(
        minute_limit: u32,
        day_limit: u32,
        persistence_file: String,
    ) -> Self {
        let limiter = Self {
            minute_limit,
            minute_requests: Arc::new(Mutex::new(VecDeque::new())),
            day_limit,
            day_requests: Arc::new(Mutex::new(VecDeque::new())),
            persistence_file: Some(persistence_file),
        };

        // Load existing daily usage on startup
        if let Err(e) = limiter.load_daily_usage() {
            warn!("Failed to load daily usage from persistence: {}", e);
        }

        limiter
    }

    /// Load daily usage from persistence file
    fn load_daily_usage(&self) -> Result<()> {
        if let Some(file_path) = &self.persistence_file {
            if Path::new(file_path).exists() {
                let content = std::fs::read_to_string(file_path)?;
                let timestamps: Vec<DateTime<Utc>> = serde_json::from_str(&content)?;

                // Only keep timestamps from today (current UTC day)
                let today_start = Utc::now()
                    .date_naive()
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .and_utc();
                let valid_timestamps: VecDeque<DateTime<Utc>> = timestamps
                    .into_iter()
                    .filter(|t| *t >= today_start)
                    .collect();

                // Update the day_requests with loaded data
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                        let mut day_requests = self.day_requests.lock().await;
                        *day_requests = valid_timestamps;
                    })
                });

                info!(
                    "Loaded {} daily requests from persistence",
                    self.day_requests.try_lock().map(|r| r.len()).unwrap_or(0)
                );
            }
        }
        Ok(())
    }

    /// Save daily usage to persistence file
    async fn save_daily_usage(&self) -> Result<()> {
        if let Some(file_path) = &self.persistence_file {
            let day_requests = self.day_requests.lock().await;
            let timestamps: Vec<DateTime<Utc>> = day_requests.iter().cloned().collect();
            drop(day_requests);

            let content = serde_json::to_string(&timestamps)?;
            tokio::fs::write(file_path, content).await?;
        }
        Ok(())
    }

    /// Get current usage statistics
    pub async fn get_usage_stats(&self) -> (u32, u32, u32, u32) {
        let now_utc = Utc::now();
        let now_instant = Instant::now();

        // Clean up and count minute requests
        let mut minute_requests = self.minute_requests.lock().await;
        let minute_ago = now_instant - Duration::from_secs(60);
        while minute_requests.front().is_some_and(|t| *t < minute_ago) {
            minute_requests.pop_front();
        }
        let minute_used = minute_requests.len() as u32;
        drop(minute_requests);

        // Clean up and count day requests (current UTC day only)
        let mut day_requests = self.day_requests.lock().await;
        let today_start = now_utc.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
        while day_requests.front().is_some_and(|t| *t < today_start) {
            day_requests.pop_front();
        }
        let day_used = day_requests.len() as u32;
        drop(day_requests);

        (minute_used, self.minute_limit, day_used, self.day_limit)
    }

    /// Check if a request can be made, and if not, how long to wait
    pub async fn check(&self) -> Result<()> {
        // First check the daily limit
        let now_utc = Utc::now();
        let mut day_requests = self.day_requests.lock().await;

        // Clean up old day requests (before today's start)
        let today_start = now_utc.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
        while day_requests.front().is_some_and(|t| *t < today_start) {
            day_requests.pop_front();
        }

        // Check if we've hit the daily limit
        if day_requests.len() >= self.day_limit as usize {
            // Daily quota resets at midnight UTC (start of next day)
            let tomorrow_start = (now_utc.date_naive() + chrono::Duration::days(1))
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc();
            let wait_duration = tomorrow_start - now_utc;
            let hours = wait_duration.num_hours();
            let minutes = wait_duration.num_minutes() % 60;

            let error_msg = format!(
                "⛔ Daily rate limit reached ({} requests). Reset in {hours} hours {minutes} minutes",
                self.day_limit
            );
            warn!("{}", error_msg);
            return Err(anyhow!(error_msg));
        }

        // Then check the per-minute limit
        let now = Instant::now();
        let mut minute_requests = self.minute_requests.lock().await;

        // Clean up old minute requests (older than 1 minute)
        while minute_requests
            .front()
            .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60))
        {
            minute_requests.pop_front();
        }

        // Check if we've hit the per-minute limit
        if minute_requests.len() >= self.minute_limit as usize {
            // Calculate when the oldest request will expire
            if let Some(oldest) = minute_requests.front() {
                let elapsed = now.duration_since(*oldest);
                let wait_duration = if elapsed >= Duration::from_secs(60) {
                    // This shouldn't happen due to cleanup above, but handle it gracefully
                    Duration::from_secs(1)
                } else {
                    Duration::from_secs(60) - elapsed
                };

                // Ensure minimum wait time of 1 second
                let wait_secs = std::cmp::max(wait_duration.as_secs(), 1);

                let error_msg = format!(
                    "⏳ Per-minute rate limit reached ({} requests). Try again in {wait_secs} seconds",
                    self.minute_limit
                );
                warn!("{}", error_msg);
                return Err(anyhow!(error_msg));
            }
        }

        Ok(())
    }

    /// Record a successful request
    pub async fn record_request(&self) {
        // Record the request for per-minute tracking
        let now = Instant::now();
        let mut minute_requests = self.minute_requests.lock().await;
        minute_requests.push_back(now);
        drop(minute_requests);

        // Record the request for per-day tracking
        let now_utc = Utc::now();
        let mut day_requests = self.day_requests.lock().await;
        day_requests.push_back(now_utc);
        drop(day_requests);

        // Save daily usage to persistence
        if let Err(e) = self.save_daily_usage().await {
            error!("Failed to save daily usage to persistence: {}", e);
        }
    }

    /// Wait until a request can be made, then record it
    pub async fn acquire(&self) -> Result<()> {
        let mut attempts = 0;
        const MAX_ATTEMPTS: u32 = 5;
        const RETRY_DELAY: u64 = 15;

        loop {
            attempts += 1;
            match self.check().await {
                Ok(()) => {
                    // We can make a request now
                    self.record_request().await;
                    return Ok(());
                }
                Err(e) => {
                    let error_msg = e.to_string();

                    // Check if it's a per-minute limit error
                    if error_msg.contains("Per-minute rate limit reached") {
                        if attempts > MAX_ATTEMPTS {
                            warn!(
                                "⛔ Giving up after {} attempts to acquire rate limit slot",
                                MAX_ATTEMPTS
                            );
                            return Err(anyhow!("Max retry attempts ({}) exceeded", MAX_ATTEMPTS));
                        }

                        // Log retry attempt and wait
                        info!(
                            "🔄 Rate limit retry attempt {}/{}: waiting {} seconds",
                            attempts, MAX_ATTEMPTS, RETRY_DELAY
                        );

                        // Wait for the specified time plus a small buffer
                        tokio::time::sleep(Duration::from_secs(RETRY_DELAY)).await;
                        continue; // Try again after waiting
                    }

                    // For daily limit or any other error, just return the error
                    return Err(e);
                }
            }
        }
    }
}
