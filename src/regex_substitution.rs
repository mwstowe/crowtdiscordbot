use anyhow::Result;
use serenity::model::channel::Message;
use serenity::prelude::*;
use tracing::{error, info};
use crate::display_name::get_best_display_name;

// Handle regex substitution for messages starting with !s/ or .s/
pub async fn handle_regex_substitution(ctx: &Context, msg: &Message) -> Result<()> {
    // Extract the regex pattern and replacement
    let content = &msg.content;
    
    // Parse the substitution command: s/pattern/replacement/flags
    // First, find the second and third forward slashes
    let parts: Vec<&str> = content.splitn(4, '/').collect();
    
    if parts.len() < 3 {
        // Not enough parts for a valid substitution
        return Ok(());
    }
    
    // Extract pattern and replacement
    let pattern = parts[1];
    let replacement = parts[2];
    
    // Extract flags if present
    let flags = if parts.len() > 3 { parts[3] } else { "" };
    let case_insensitive = flags.contains('i');
    
    // Log the substitution attempt
    info!("Regex substitution attempt: pattern='{}', replacement='{}', flags='{}'", 
          pattern, replacement, flags);
    
    // Get the previous message from the channel
    let builder = serenity::builder::GetMessages::new().before(msg.id).limit(10);
    let messages = msg.channel_id.messages(&ctx.http, builder).await?;
    
    // Get the bot's user ID
    let bot_id = ctx.http.get_current_user().await?.id;
    
    // Find the first non-command message (not starting with ! or .)
    let previous_msg = messages.iter().find(|m| {
        !m.content.starts_with('!') && 
        !m.content.starts_with('.') && 
        // Skip messages from the bot itself
        m.author.id != bot_id
    });
    
    if let Some(prev_msg) = previous_msg {
        // Try to apply the regex
        let regex_result = if case_insensitive {
            regex::RegexBuilder::new(pattern)
                .case_insensitive(true)
                .build()
        } else {
            regex::RegexBuilder::new(pattern)
                .build()
        };
        
        match regex_result {
            Ok(re) => {
                // Apply the substitution
                let new_content = re.replace_all(&prev_msg.content, replacement);
                
                // If the content changed, send the modified message
                if new_content != prev_msg.content {
                    // Get the display name of the original message author
                    let display_name = get_best_display_name(ctx, prev_msg).await;
                    
                    // Format and send the response
                    let response = format!("{} meant: {}", display_name, new_content);
                    
                    if let Err(e) = msg.channel_id.say(&ctx.http, response).await {
                        error!("Error sending regex substitution response: {:?}", e);
                    }
                }
                // If no change, say nothing as requested
            },
            Err(e) => {
                error!("Invalid regex pattern: {:?}", e);
                // Silently fail - don't notify the user of regex errors
            }
        }
    }
    
    Ok(())
}
