use std::time::{Duration, Instant};
use tokio::time::sleep;
use tracing::info;

/// Calculates and applies a realistic typing delay based on response length
pub async fn apply_realistic_delay(response: &str) {
    // Record when we got the response
    let response_received = Instant::now();
    
    // Calculate the number of words in the response
    let word_count = response.split_whitespace().count();
    
    // Calculate the delay: 0.5 seconds per word
    let delay_seconds = word_count as f32 * 0.5;
    let delay = Duration::from_secs_f32(delay_seconds);
    
    // Calculate when we should send the response
    let send_time = response_received + delay;
    
    // Check if we need to wait
    let now = Instant::now();
    if now < send_time {
        // Calculate how much longer we need to wait
        let remaining_delay = send_time - now;
        
        info!(
            "Applying realistic typing delay: {} words = {:.1} seconds (waiting {:.1} more seconds)",
            word_count,
            delay_seconds,
            remaining_delay.as_secs_f32()
        );
        
        // Wait for the remaining time
        sleep(remaining_delay).await;
    } else {
        info!(
            "Response ready to send immediately: {} words = {:.1} seconds (already elapsed)",
            word_count,
            delay_seconds
        );
    }
}
