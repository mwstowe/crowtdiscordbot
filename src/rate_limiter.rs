use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{error, info, warn};

/// SQLite database file used for all bot persistence (shared with message history).
const PERSISTENCE_DB: &str = "message_history.db";

/// A rate limiter that enforces both per-minute and per-day limits
#[derive(Clone)]
pub struct RateLimiter {
    // Per-minute tracking
    minute_limit: u32,
    minute_requests: Arc<Mutex<VecDeque<Instant>>>,

    // Per-day tracking
    day_limit: u32,
    day_requests: Arc<Mutex<VecDeque<DateTime<Utc>>>>,

    // Persistence: a stable bucket name identifying this limiter's quota
    // (e.g. "gemini_text_quota"). Timestamps are stored in the shared SQLite
    // database in the rate_limit_events table.
    persistence_bucket: Option<String>,
}

impl RateLimiter {
    /// Create a new rate limiter with SQLite-backed persistence.
    ///
    /// `persistence_bucket` is a stable identifier for this limiter's daily
    /// quota (historically a filename like "gemini_text_quota.json"; any
    /// ".json" suffix is stripped so existing call sites keep working).
    pub fn new_with_persistence(
        minute_limit: u32,
        day_limit: u32,
        persistence_bucket: String,
    ) -> Self {
        let bucket = persistence_bucket
            .strip_suffix(".json")
            .unwrap_or(&persistence_bucket)
            .to_string();

        // Load today's persisted daily usage up front so we can seed the
        // day_requests queue directly - no locking or block_in_place needed.
        let initial_day_requests = match Self::load_daily_usage(&bucket) {
            Ok(v) => v,
            Err(e) => {
                warn!("Failed to load daily usage from persistence: {}", e);
                VecDeque::new()
            }
        };

        Self {
            minute_limit,
            minute_requests: Arc::new(Mutex::new(VecDeque::new())),
            day_limit,
            day_requests: Arc::new(Mutex::new(initial_day_requests)),
            persistence_bucket: Some(bucket),
        }
    }

    /// Open the shared SQLite database and ensure the rate_limit_events table exists.
    fn open_db() -> Result<rusqlite::Connection> {
        let conn = rusqlite::Connection::open(PERSISTENCE_DB)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS rate_limit_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                bucket TEXT NOT NULL,
                ts INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_rate_limit_bucket_ts
                ON rate_limit_events (bucket, ts);",
        )?;
        Ok(conn)
    }

    /// Load today's (UTC) persisted daily usage for a bucket from SQLite,
    /// pruning older events. Returns the timestamps in chronological order.
    fn load_daily_usage(bucket: &str) -> Result<VecDeque<DateTime<Utc>>> {
        let today_start = Utc::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();

        let conn = Self::open_db()?;

        // Prune events older than today for this bucket to keep the table small
        conn.execute(
            "DELETE FROM rate_limit_events WHERE bucket = ?1 AND ts < ?2",
            rusqlite::params![bucket, today_start],
        )?;

        // Load today's timestamps
        let mut stmt = conn.prepare(
            "SELECT ts FROM rate_limit_events WHERE bucket = ?1 AND ts >= ?2 ORDER BY ts",
        )?;
        let rows = stmt.query_map(rusqlite::params![bucket, today_start], |row| {
            row.get::<_, i64>(0)
        })?;

        let mut valid: VecDeque<DateTime<Utc>> = VecDeque::new();
        for ts in rows.flatten() {
            if let Some(dt) = DateTime::<Utc>::from_timestamp(ts, 0) {
                valid.push_back(dt);
            }
        }

        info!(
            "Loaded {} daily requests for bucket '{}' from SQLite",
            valid.len(),
            bucket
        );
        Ok(valid)
    }

    /// Persist a single request timestamp to SQLite for the daily quota.
    async fn record_daily_event(&self, ts: DateTime<Utc>) -> Result<()> {
        if let Some(bucket) = &self.persistence_bucket {
            let bucket = bucket.clone();
            let epoch = ts.timestamp();
            // rusqlite is synchronous; run it on the blocking pool.
            tokio::task::spawn_blocking(move || -> Result<()> {
                let conn = Self::open_db()?;
                conn.execute(
                    "INSERT INTO rate_limit_events (bucket, ts) VALUES (?1, ?2)",
                    rusqlite::params![bucket, epoch],
                )?;
                Ok(())
            })
            .await??;
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

        // Persist this request to SQLite for the daily quota
        if let Err(e) = self.record_daily_event(now_utc).await {
            error!("Failed to persist daily usage to SQLite: {}", e);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Recording requests should persist to SQLite and be restored by a fresh
    /// limiter instance using the same bucket.
    #[tokio::test]
    async fn daily_usage_persists_across_instances() {
        // Unique bucket so the test doesn't collide with real quotas or other tests
        let bucket = format!(
            "test_bucket_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );

        // Clean any stray rows for this bucket first (defensive)
        {
            let conn = RateLimiter::open_db().expect("open db");
            let _ = conn.execute(
                "DELETE FROM rate_limit_events WHERE bucket = ?1",
                rusqlite::params![bucket],
            );
        }

        // First limiter: record two requests
        let limiter = RateLimiter::new_with_persistence(100, 100, bucket.clone());
        limiter.record_request().await;
        limiter.record_request().await;

        let (_, _, day_used, _) = limiter.get_usage_stats().await;
        assert_eq!(day_used, 2, "in-memory daily count should be 2");

        // Fresh limiter with the same bucket should reload the 2 events
        let reloaded = RateLimiter::new_with_persistence(100, 100, bucket.clone());
        let (_, _, reloaded_day_used, _) = reloaded.get_usage_stats().await;
        assert_eq!(
            reloaded_day_used, 2,
            "reloaded limiter should restore persisted daily count"
        );

        // Cleanup
        let conn = RateLimiter::open_db().expect("open db");
        let _ = conn.execute(
            "DELETE FROM rate_limit_events WHERE bucket = ?1",
            rusqlite::params![bucket],
        );
    }

    /// The ".json" suffix on legacy bucket names is normalized away so both
    /// spellings refer to the same persistence bucket.
    #[test]
    fn json_suffix_is_stripped_from_bucket() {
        let limiter = RateLimiter::new_with_persistence(1, 1, "foo_quota.json".to_string());
        assert_eq!(limiter.persistence_bucket.as_deref(), Some("foo_quota"));
    }
}
