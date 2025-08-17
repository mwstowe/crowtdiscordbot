use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{info, warn};

/// A rate limiter that enforces both per-minute and per-day limits
#[derive(Clone)]
pub struct RateLimiter {
    // Per-minute tracking
    minute_limit: u32,
    minute_requests: Arc<Mutex<VecDeque<Instant>>>,

    // Per-day tracking
    day_limit: u32,
    day_requests: Arc<Mutex<VecDeque<DateTime<Utc>>>>,
}

impl RateLimiter {
    /// Create a new rate limiter with specified limits
    pub fn new(minute_limit: u32, day_limit: u32) -> Self {
        Self {
            minute_limit,
            minute_requests: Arc::new(Mutex::new(VecDeque::new())),
            day_limit,
            day_requests: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Check if a request can be made, and if not, how long to wait
    pub async fn check(&self) -> Result<()> {
        // First check the daily limit
        let now_utc = Utc::now();
        let mut day_requests = self.day_requests.lock().await;

        // Clean up old day requests (older than 24 hours)
        let day_ago = now_utc - chrono::Duration::days(1);
        while day_requests.front().is_some_and(|t| *t < day_ago) {
            day_requests.pop_front();
        }

        // Check if we've hit the daily limit
        if day_requests.len() >= self.day_limit as usize {
            // Calculate when the oldest request will expire
            if let Some(oldest) = day_requests.front() {
                let reset_time = *oldest + chrono::Duration::days(1);
                let wait_duration = reset_time - now_utc;
                let hours = wait_duration.num_hours();
                let minutes = wait_duration.num_minutes() % 60;

                let error_msg = format!(
                    "⛔ Daily rate limit reached ({} requests). Reset in {hours} hours {minutes} minutes",
                    self.day_limit
                );
                warn!("{}", error_msg);
                return Err(anyhow!(error_msg));
            }
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
                let wait_duration = Duration::from_secs(60) - now.duration_since(*oldest);
                let wait_secs = wait_duration.as_secs();

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

        // Record the request for per-day tracking
        let now_utc = Utc::now();
        let mut day_requests = self.day_requests.lock().await;
        day_requests.push_back(now_utc);
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
