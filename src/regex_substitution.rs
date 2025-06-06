use anyhow::Result;
use serenity::model::channel::Message;
use serenity::prelude::*;
use tracing::{error, info, warn};
use crate::display_name::get_best_display_name;
use regex::Regex;

// URL pattern for detecting URLs in text
const URL_PATTERN: &str = r"https?://[^\s/$.?#].[^\s]*";

// Handle regex substitution for messages starting with !s/, .s/, !/, or ./
pub async fn handle_regex_substitution(ctx: &Context, msg: &Message) -> Result<()> {
    // Extract the regex pattern and replacement
    let content = &msg.content;
    
    // Parse the substitution command: s/pattern/replacement[/flags] or /pattern/replacement[/flags]
    // First, find the second and third forward slashes
    let parts: Vec<&str> = content.splitn(4, '/').collect();
    
    if parts.len() < 3 {
        // Not enough parts for a valid substitution
        return Ok(());
    }
    
    // Extract pattern and replacement
    let pattern = parts[1];
    
    // The replacement might have a trailing slash that we need to handle
    let replacement = if parts.len() > 3 {
        // If we have flags, the replacement is just parts[2]
        parts[2]
    } else {
        // If we don't have flags, the replacement might have a trailing slash
        // that got included in parts[2]
        let replacement_part = parts[2];
        if replacement_part.ends_with('/') {
            // Remove the trailing slash
            &replacement_part[0..replacement_part.len()-1]
        } else {
            // No trailing slash
            replacement_part
        }
    };
    
    // Extract flags if present
    let flags = if parts.len() > 3 { parts[3] } else { "" };
    let case_insensitive = flags.contains('i');
    
    // Log the substitution attempt
    info!("Regex substitution attempt: pattern='{}', replacement='{}', flags='{}'", 
          pattern, replacement, flags);
    
    // Get the last four messages from the channel
    let builder = serenity::builder::GetMessages::new().before(msg.id).limit(4);
    let messages = msg.channel_id.messages(&ctx.http, builder).await?;
    
    // Get the bot's user ID
    let bot_id = ctx.http.get_current_user().await?.id;
    
    // Check if the most recent message is a bot regex response
    let is_bot_regex_response = messages.first()
        .map(|m| {
            m.author.id == bot_id && 
            (m.content.contains(" meant: ") || m.content.contains(" *really* meant: "))
        })
        .unwrap_or(false);
    
    // Debug log the messages we're considering
    for (i, m) in messages.iter().enumerate() {
        info!("Message {}: author={}, content={}", i, m.author.name, m.content);
    }
    
    // Filter out commands and bot messages (except regex responses if they're the most recent)
    let valid_messages: Vec<&Message> = messages.iter()
        .enumerate()
        .filter(|(i, m)| {
            (!m.content.starts_with('!') && 
             !m.content.starts_with('.')) ||
            (*i == 0 && is_bot_regex_response) // Allow the most recent message if it's a bot regex response
        })
        .map(|(_, m)| m)
        .collect();
    
    // Try to build the regex
    let regex_result = if case_insensitive {
        regex::RegexBuilder::new(pattern)
            .case_insensitive(true)
            .build()
    } else {
        regex::RegexBuilder::new(pattern)
            .build()
    };
    
    // Compile URL detection regex
    let url_regex = Regex::new(URL_PATTERN).expect("Invalid URL pattern regex");
    
    match regex_result {
        Ok(re) => {
            // Try each message in order from most recent to least recent
            for (i, prev_msg) in valid_messages.iter().enumerate() {
                // Apply the substitution
                let content_to_modify = if i == 0 && is_bot_regex_response {
                    // If this is a bot regex response, extract just the message content without the prefix
                    if let Some(content_start) = prev_msg.content.find(" meant: ") {
                        prev_msg.content[(content_start + " meant: ".len())..].to_string()
                    } else if let Some(content_start) = prev_msg.content.find(" *really* meant: ") {
                        prev_msg.content[(content_start + " *really* meant: ".len())..].to_string()
                    } else {
                        prev_msg.content.clone()
                    }
                } else {
                    prev_msg.content.clone()
                };
                
                // Apply regex to the cleaned content
                let new_content = re.replace_all(&content_to_modify, replacement);
                
                // If the content changed, check if we modified any URLs
                if new_content != content_to_modify {
                    // Get all URLs from original message
                    let original_urls: Vec<&str> = url_regex.find_iter(&content_to_modify)
                        .map(|m| m.as_str())
                        .collect();
                        
                    // Get all URLs from new message
                    let new_urls: Vec<&str> = url_regex.find_iter(&new_content)
                        .map(|m| m.as_str())
                        .collect();
                    
                    // Check if any URLs were modified
                    if original_urls != new_urls {
                        warn!("Regex substitution would modify URLs - skipping");
                        continue;  // Try next message
                    }
                    
                    // Get the display name of the original message author
                    let display_name = if i == 0 && is_bot_regex_response {
                        // If this is a bot regex response, extract the original author's name
                        if let Some(name_end) = prev_msg.content.find(" meant: ") {
                            let extracted_name = prev_msg.content[0..name_end].to_string();
                            info!("Extracted author name from bot response: {}", extracted_name);
                            extracted_name
                        } else if let Some(name_end) = prev_msg.content.find(" *really* meant: ") {
                            let extracted_name = prev_msg.content[0..name_end].to_string();
                            info!("Extracted author name from bot response: {}", extracted_name);
                            extracted_name
                        } else {
                            let name = get_best_display_name(ctx, prev_msg).await;
                            info!("Using display name for bot response: {}", name);
                            name
                        }
                    } else {
                        // For regular messages, get the display name of the original author
                        let name = get_best_display_name(ctx, prev_msg).await;
                        info!("Using display name for message {}: {}", i, name);
                        name
                    };
                    
                    // Clean the display name
                    let clean_display_name = crate::display_name::clean_display_name(&display_name).trim().to_string();
                    
                    // Format and send the response
                    let response = if i == 0 && is_bot_regex_response {
                        format!("{} *really* meant: {}", clean_display_name, new_content)
                    } else {
                        format!("{} meant: {}", clean_display_name, new_content)
                    };
                    
                    if let Err(e) = msg.channel_id.say(&ctx.http, response).await {
                        error!("Error sending regex substitution response: {:?}", e);
                    }
                    
                    // Stop after first successful substitution
                    return Ok(());
                }
            }
            // If we get here, no substitutions worked - silently give up
        },
        Err(e) => {
            error!("Invalid regex pattern: {:?}", e);
            // Silently fail - don't notify the user of regex errors
        }
    }
    
    Ok(())
}
