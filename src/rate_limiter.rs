use std::sync::Arc;
use tokio::sync::Mutex;
use std::time::{Duration, Instant};
use std::collections::VecDeque;
use chrono::{DateTime, Utc};
use anyhow::{Result, anyhow};
use tracing::info;

/// A rate limiter that enforces both per-minute and per-day limits
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
        while day_requests.front().map_or(false, |t| *t < day_ago) {
            day_requests.pop_front();
        }
        
        // Check if we've hit the daily limit
        if day_requests.len() >= self.day_limit as usize {
            // Calculate when the oldest request will expire
            if let Some(oldest) = day_requests.front() {
                let reset_time = *oldest + chrono::Duration::days(1);
                let wait_duration = reset_time - now_utc;
                
                // Format the wait time in a human-readable way
                let hours = wait_duration.num_hours();
                let minutes = wait_duration.num_minutes() % 60;
                let seconds = wait_duration.num_seconds() % 60;
                
                let wait_message = format!(
                    "Daily rate limit reached ({}/{}). Try again in {}h {}m {}s.", 
                    day_requests.len(), 
                    self.day_limit,
                    hours,
                    minutes,
                    seconds
                );
                
                return Err(anyhow!(wait_message));
            }
        }
        
        // Now check the per-minute limit
        let now = Instant::now();
        let mut minute_requests = self.minute_requests.lock().await;
        
        // Clean up old minute requests (older than 60 seconds)
        let minute_ago = now - Duration::from_secs(60);
        while minute_requests.front().map_or(false, |t| *t < minute_ago) {
            minute_requests.pop_front();
        }
        
        // Check if we've hit the per-minute limit
        if minute_requests.len() >= self.minute_limit as usize {
            // Calculate when the oldest request will expire
            if let Some(oldest) = minute_requests.front() {
                let time_since_oldest = now.duration_since(*oldest);
                let wait_duration = Duration::from_secs(60).saturating_sub(time_since_oldest);
                
                let wait_message = format!(
                    "Per-minute rate limit reached ({}/{}). Try again in {} seconds.", 
                    minute_requests.len(), 
                    self.minute_limit,
                    wait_duration.as_secs()
                );
                
                return Err(anyhow!(wait_message));
            }
        }
        
        Ok(())
    }
    
    /// Record that a request was made
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
        loop {
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
                        // Extract the wait time
                        if let Some(wait_seconds) = extract_wait_seconds(&error_msg) {
                            info!("{}", error_msg);
                            
                            // Wait for the specified time plus a small buffer
                            tokio::time::sleep(Duration::from_secs(wait_seconds + 1)).await;
                            continue; // Try again after waiting
                        }
                    }
                    
                    // For daily limit or any other error, just return the error
                    return Err(e);
                }
            }
        }
    }
}

// Helper function to extract wait seconds from error message
fn extract_wait_seconds(message: &str) -> Option<u64> {
    let parts: Vec<&str> = message.split("Try again in ").collect();
    if parts.len() < 2 {
        return None;
    }
    
    let seconds_part = parts[1].split(" seconds").next()?;
    seconds_part.trim().parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::sleep;
    
    #[tokio::test]
    async fn test_minute_rate_limit() {
        let limiter = RateLimiter::new(3, 10); // 3 per minute, 10 per day
        
        // First 3 requests should succeed
        for _ in 0..3 {
            assert!(limiter.acquire().await.is_ok());
        }
        
        // 4th request should fail with per-minute limit
        let result = limiter.check().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Per-minute rate limit reached"));
    }
    
    #[tokio::test]
    async fn test_day_rate_limit() {
        let limiter = RateLimiter::new(100, 5); // 100 per minute, 5 per day
        
        // First 5 requests should succeed
        for _ in 0..5 {
            assert!(limiter.acquire().await.is_ok());
        }
        
        // 6th request should fail with daily limit
        let result = limiter.check().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Daily rate limit reached"));
    }
}
