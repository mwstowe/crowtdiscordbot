use serenity::prelude::*;
use std::time::{Duration, Instant};
use tokio::time::sleep;
use tracing::info;
// Removed unused import
use serenity::model::id::ChannelId;

/// Calculates and applies a realistic typing delay based on response length
/// Also shows typing indicator in the channel during the delay
pub async fn apply_realistic_delay(response: &str, ctx: &Context, channel_id: ChannelId) {
    // Record when we got the response
    let response_received = Instant::now();

    // Calculate the number of words in the response
    let word_count = response.split_whitespace().count();

    // Calculate the delay: 0.2 seconds per word
    let calculated_delay = word_count as f32 * 0.2;

    // Apply minimum and maximum constraints (2-5 seconds)
    let delay_seconds = calculated_delay.clamp(2.0, 5.0);
    let delay = Duration::from_secs_f32(delay_seconds);

    // Start typing indicator
    if let Err(e) = channel_id.broadcast_typing(&ctx.http).await {
        info!("Failed to send typing indicator: {:?}", e);
    } else {
        info!("Started typing indicator in channel {}", channel_id);
    }

    // Calculate when we should send the response
    let send_time = response_received + delay;

    // Check if we need to wait
    let now = Instant::now();
    if now < send_time {
        // Calculate how much longer we need to wait
        let remaining_delay = send_time - now;

        info!(
            "Applying realistic typing delay: {} words = {:.1} seconds (clamped to {:.1}s, waiting {:.1} more seconds)",
            word_count,
            calculated_delay,
            delay_seconds,
            remaining_delay.as_secs_f32()
        );

        // Wait for the remaining time
        sleep(remaining_delay).await;
    } else {
        info!(
            "Response ready to send immediately: {} words = {:.1} seconds (clamped to {:.1}s, already elapsed)",
            word_count,
            calculated_delay,
            delay_seconds
        );
    }
}
