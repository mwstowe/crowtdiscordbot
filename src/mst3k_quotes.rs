use anyhow::Result;
use serenity::all::*;
use serenity::model::channel::Message;
use tracing::{error, info};
use rand::seq::SliceRandom;

// Helper function for fallback MST3K quotes when database query fails
pub async fn fallback_mst3k_quote(ctx: &Context, msg: &Message) -> Result<()> {
    let mst3k_quotes = [
        "Watch out for snakes!",
        "It's the amazing Rando!",
        "Normal view... Normal view... NORMAL VIEW!",
        "Hi-keeba!",
        "I'm different!",
        "Rowsdower!",
        "Mitchell!",
        "Deep hurting...",
        "Trumpy, you can do magic things!",
        "Torgo's theme intensifies",
    ];
            
    let quote = mst3k_quotes.choose(&mut rand::thread_rng()).unwrap_or(&"I'm different!").to_string();
    let quote_text = quote.clone(); // Clone for logging
    if let Err(e) = msg.channel_id.say(&ctx.http, quote).await {
        error!("Error sending fallback MST3K quote: {:?}", e);
    } else {
        info!("Fallback MST3K quote sent: {}", quote_text);
    }
    
    Ok(())
}
