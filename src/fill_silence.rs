use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{info, debug};
use serenity::model::id::{ChannelId, UserId};

/// Manages the "fill silence" feature, which increases interjection probabilities
/// after periods of inactivity in a channel.
pub struct FillSilenceManager {
    /// Whether the fill silence feature is enabled
    enabled: bool,
    
    /// Start increasing probabilities after this many hours of silence
    start_hours: f64,
    
    /// Reach 100% probability after this many hours of silence
    max_hours: f64,
    
    /// Last activity time for each channel, keyed by channel ID
    last_activity: Arc<RwLock<HashMap<ChannelId, (Instant, UserId)>>>,
}

impl FillSilenceManager {
    /// Create a new FillSilenceManager
    pub fn new(enabled: bool, start_hours: f64, max_hours: f64) -> Self {
        Self {
            enabled,
            start_hours,
            max_hours,
            last_activity: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    /// Update the last activity time for a channel
    pub async fn update_activity(&self, channel_id: ChannelId, user_id: UserId) {
        if !self.enabled {
            return;
        }
        
        let mut last_activity = self.last_activity.write().await;
        last_activity.insert(channel_id, (Instant::now(), user_id));
        debug!("Updated last activity for channel {} by user {}", channel_id, user_id);
    }
    
    /// Calculate the probability multiplier for a channel based on inactivity time
    /// Returns a multiplier between 1.0 (normal probability) and a value that would
    /// make the probability 100% (after max_hours of inactivity)
    pub async fn get_probability_multiplier(&self, channel_id: ChannelId, bot_id: UserId) -> f64 {
        if !self.enabled {
            return 1.0;
        }
        
        let last_activity = self.last_activity.read().await;
        
        // If we don't have a record for this channel, use the current time
        let (last_time, last_user_id) = match last_activity.get(&channel_id) {
            Some(data) => data,
            None => {
                // No activity recorded yet, use normal probability
                return 1.0;
            }
        };
        
        // If the last message was from the bot, use normal probability
        if *last_user_id == bot_id {
            return 1.0;
        }
        
        // Calculate how many hours have passed since the last activity
        let elapsed = last_time.elapsed();
        let hours_elapsed = elapsed.as_secs_f64() / 3600.0;
        
        // If less than start_hours have passed, use normal probability
        if hours_elapsed < self.start_hours {
            return 1.0;
        }
        
        // If more than max_hours have passed, use maximum probability (100%)
        if hours_elapsed >= self.max_hours {
            // Calculate what multiplier would make the probability 100%
            // This depends on the base probability, but we'll use a very high value
            return 1000.0;
        }
        
        // Otherwise, scale the probability linearly between start_hours and max_hours
        let silence_range = self.max_hours - self.start_hours;
        let silence_progress = (hours_elapsed - self.start_hours) / silence_range;
        
        // Scale from 1.0 to 1000.0 (effectively 100% probability)
        let multiplier = 1.0 + (999.0 * silence_progress);
        
        info!(
            "Channel {} has been silent for {:.2} hours, probability multiplier: {:.2}",
            channel_id, hours_elapsed, multiplier
        );
        
        multiplier
    }
}
