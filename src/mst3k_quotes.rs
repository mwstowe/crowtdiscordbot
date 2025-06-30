use anyhow::Result;
use serenity::all::*;
use serenity::model::channel::Message;
use tracing::{error, info};

// Helper function for when MST3K database query fails - now just logs the issue
pub async fn fallback_mst3k_quote(_ctx: &Context, _msg: &Message) -> Result<()> {
    // Log the issue but don't send any message to the channel
    error!("MST3K quote database query failed - no fallback message sent");
    info!("Suppressing fallback MST3K quote as configured");
    
    Ok(())
}
