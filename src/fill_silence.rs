use rand::Rng;
use serenity::model::id::{ChannelId, UserId};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{debug, info};

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

    /// Last time we checked for spontaneous interjections for each channel
    last_check: Arc<RwLock<HashMap<ChannelId, Instant>>>,

    /// Tracks if the bot was the last speaker in a channel
    bot_was_last_speaker: Arc<RwLock<HashMap<ChannelId, bool>>>,
}

impl FillSilenceManager {
    /// Create a new FillSilenceManager
    pub fn new(enabled: bool, start_hours: f64, max_hours: f64) -> Self {
        Self {
            enabled,
            start_hours,
            max_hours,
            last_activity: Arc::new(RwLock::new(HashMap::new())),
            last_check: Arc::new(RwLock::new(HashMap::new())),
            bot_was_last_speaker: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Update the last activity time for a channel
    pub async fn update_activity(&self, channel_id: ChannelId, user_id: UserId) {
        if !self.enabled {
            return;
        }

        let mut last_activity = self.last_activity.write().await;
        last_activity.insert(channel_id, (Instant::now(), user_id));

        debug!(
            "Updated last activity for channel {} by user {}",
            channel_id, user_id
        );
    }

    /// Mark that the bot was the last speaker in a channel
    pub async fn mark_bot_as_last_speaker(&self, channel_id: ChannelId) {
        if !self.enabled {
            return;
        }

        let mut bot_last = self.bot_was_last_speaker.write().await;
        bot_last.insert(channel_id, true);

        debug!("Marked bot as last speaker in channel {}", channel_id);
    }

    /// Mark that a user (not the bot) was the last speaker in a channel
    pub async fn mark_user_as_last_speaker(&self, channel_id: ChannelId) {
        if !self.enabled {
            return;
        }

        let mut bot_last = self.bot_was_last_speaker.write().await;
        bot_last.insert(channel_id, false);

        debug!("Marked user as last speaker in channel {}", channel_id);
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

        // Calculate the multiplier based on hours of silence
        // We'll use the hours_elapsed directly as a multiplier, with some constraints

        // Base multiplier is the number of hours elapsed
        let hours_multiplier = hours_elapsed;

        // Cap the multiplier at a reasonable maximum (e.g., 24 hours = 24x)
        let max_multiplier = 24.0;
        let capped_multiplier = hours_multiplier.min(max_multiplier);

        // If we've exceeded max_hours, add an additional boost to ensure high probability
        let final_multiplier = if hours_elapsed >= self.max_hours {
            // Add an extra boost to make very likely (but not 100% guaranteed)
            capped_multiplier * 2.0
        } else {
            capped_multiplier
        };

        info!(
            "Channel {} has been silent for {:.2} hours, probability multiplier: {:.2}x",
            channel_id, hours_elapsed, final_multiplier
        );

        final_multiplier
    }

    /// Check if we should make a spontaneous interjection
    /// Returns true if enough time has passed since the last check and the channel has been inactive
    pub async fn should_check_spontaneous_interjection(
        &self,
        channel_id: ChannelId,
        bot_id: UserId,
    ) -> bool {
        if !self.enabled {
            return false;
        }

        // Check if the bot was the last speaker
        let bot_last = self.bot_was_last_speaker.read().await;
        if bot_last.get(&channel_id).unwrap_or(&false) == &true {
            // Bot was the last speaker, don't make another interjection until someone else speaks
            debug!(
                "Bot was last speaker in channel {}, skipping spontaneous interjection check",
                channel_id
            );
            return false;
        }

        // Get the last time we checked this channel
        let mut should_check = false;
        let mut new_check_time = None;

        {
            let last_check = self.last_check.read().await;
            let now = Instant::now();

            // If we've never checked this channel, or it's been at least 1 minute since the last check
            if let Some(last_check_time) = last_check.get(&channel_id) {
                let minutes_since_check = last_check_time.elapsed().as_secs() / 60;

                // Random interval between 1 and 15 minutes
                let random_interval = rand::thread_rng().gen_range(1..=15);

                if minutes_since_check >= random_interval as u64 {
                    should_check = true;
                    new_check_time = Some(now);
                }
            } else {
                // First time checking this channel
                should_check = true;
                new_check_time = Some(now);
            }
        }

        // If we should check, update the last check time
        if should_check {
            if let Some(check_time) = new_check_time {
                let mut last_check = self.last_check.write().await;
                last_check.insert(channel_id, check_time);
            }

            // Now check if the channel has been inactive long enough
            let multiplier = self.get_probability_multiplier(channel_id, bot_id).await;

            // Only consider making a spontaneous interjection if the multiplier is > 1.0
            // (meaning we're in the period where probabilities are increasing)
            if multiplier > 1.0 {
                info!(
                    "Considering spontaneous interjection for channel {} (multiplier: {:.2}x)",
                    channel_id, multiplier
                );
                return true;
            }
        }

        false
    }
}
