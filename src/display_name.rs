use serenity::model::channel::Message;
use serenity::model::id::{UserId, GuildId};
use serenity::prelude::*;
use regex::Regex;
use tracing::{error, debug, info};
use lazy_static::lazy_static;

// Regular expression for extracting gateway usernames from bot messages
lazy_static! {
    static ref GATEWAY_USERNAME_REGEX: Regex = Regex::new(r"\[(?:irc|matrix|slack|discord)\] <([^>]+)>").unwrap();
}

// Helper function to check if a message is from a gateway bot and extract the real username
pub fn extract_gateway_username(msg: &Message) -> Option<String> {
    // Check if the message content starts with a gateway format like "[irc] <username>"
    if let Some(captures) = GATEWAY_USERNAME_REGEX.captures(&msg.content) {
        if let Some(username) = captures.get(1) {
            return Some(username.as_str().to_string());
        }
    }
    
    // Check if the author name is in gateway format like "<username>"
    let username = &msg.author.name;
    if username.starts_with('<') && username.ends_with('>') {
        return Some(username[1..username.len()-1].to_string());
    }
    
    None
}

// Helper function to get the best display name for a user
pub async fn get_best_display_name(ctx: &Context, msg: &Message) -> String {
    // First check if this is a gateway bot message
    if let Some(gateway_username) = extract_gateway_username(msg) {
        debug!("Found gateway username: {}", gateway_username);
        return gateway_username;
    }
    
    let user_id = msg.author.id;
    
    // Prioritize server nickname over global name over username
    if let Some(guild_id) = msg.guild_id {
        // Get member data which includes the nickname
        match guild_id.member(&ctx.http, user_id).await {
            Ok(member) => {
                // Use nickname if available, otherwise fall back to global name or username
                if let Some(nick) = &member.nick {
                    return nick.clone();
                }
            },
            Err(e) => {
                error!("Failed to get member data for {} in guild {}: {:?}", user_id, guild_id, e);
            }
        }
    }
    
    // Fall back to global name or username if no nickname or couldn't get member data
    msg.author.global_name.clone().unwrap_or_else(|| msg.author.name.clone())
}

// Get the best display name for a user with explicit guild ID
pub async fn get_best_display_name_with_guild(ctx: &Context, user_id: UserId, guild_id: GuildId) -> String {
    // Get member data which includes the nickname
    match guild_id.member(&ctx.http, user_id).await {
        Ok(member) => {
            // Use nickname if available
            if let Some(nick) = &member.nick {
                debug!("Using server nickname for {} in guild {}", user_id, guild_id);
                return nick.clone();
            }
            
            // Fall back to global name or username
            if let Some(global_name) = &member.user.global_name {
                return global_name.clone();
            }
            
            member.user.name
        },
        Err(e) => {
            error!("Failed to get member data for {} in guild {}: {:?}", user_id, guild_id, e);
            
            // Try to get user data directly
            match ctx.http.get_user(user_id).await {
                Ok(user) => {
                    user.global_name.clone().unwrap_or_else(|| user.name.clone())
                },
                Err(e) => {
                    error!("Failed to get user data for {}: {:?}", user_id, e);
                    
                    // This might be a gateway bot user - check if we have a cached gateway username
                    if let Some(gateway_username) = get_cached_gateway_username(user_id) {
                        info!("Using cached gateway username for {}: {}", user_id, gateway_username);
                        return gateway_username;
                    }
                    
                    // Instead of returning just the user ID, use a more user-friendly fallback
                    format!("User-{}", user_id.to_string().chars().take(4).collect::<String>())
                }
            }
        }
    }
}

// Function to cache gateway usernames
pub fn cache_gateway_username(user_id: UserId, username: &str) {
    // In a real implementation, this would store the username in a cache
    // For now, we'll just log it
    info!("Caching gateway username for {}: {}", user_id, username);
    // TODO: Implement actual caching
}

// Function to get cached gateway username
pub fn get_cached_gateway_username(_user_id: UserId) -> Option<String> {
    // In a real implementation, this would retrieve the username from a cache
    // For now, we'll just return None
    None
    // TODO: Implement actual caching
}

// Clean a display name by removing IRC formatting, brackets, and pronouns
pub fn clean_display_name(name: &str) -> String {
    // If the name is already in gateway format (within <> brackets), strip the brackets
    if name.starts_with('<') && name.ends_with('>') {
        return name[1..name.len()-1].to_string();
    }
    
    // First remove IRC formatting
    let mut clean_name = name.to_string();
    
    // Remove IRC formatting codes (bold, italic, underline, color)
    let irc_formatting = Regex::new(r"[\x02\x1D\x1F\x03\x0F](?:\d{1,2}(?:,\d{1,2})?)?").unwrap();
    clean_name = irc_formatting.replace_all(&clean_name, "").to_string();
    
    // Remove pronouns in parentheses at the end of the name
    let pronouns_regex = Regex::new(r"\s*\([^)]+\)\s*$").unwrap();
    clean_name = pronouns_regex.replace(&clean_name, "").to_string();
    
    clean_name
}

// Extract pronouns from a display name
pub fn extract_pronouns(name: &str) -> Option<String> {
    // Look for pronouns in parentheses at the end of the name
    let pronouns_regex = Regex::new(r"\s*\(([^)]+)\)\s*$").unwrap();
    if let Some(captures) = pronouns_regex.captures(name) {
        if let Some(pronouns) = captures.get(1) {
            return Some(pronouns.as_str().to_string());
        }
    }
    None
}

// Check if a string looks like a user ID (all digits)
pub fn is_user_id(s: &str) -> bool {
    s.chars().all(|c| c.is_digit(10))
}
