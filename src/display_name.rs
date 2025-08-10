use serenity::model::channel::Message;
use serenity::model::id::{UserId, GuildId};
use serenity::prelude::*;
use regex::Regex;
use tracing::{error, debug};
use lazy_static::lazy_static;

// Regular expression for extracting gateway usernames from bot messages
lazy_static! {
    // Match patterns like "[irc] <username>" in the message content
    static ref GATEWAY_USERNAME_REGEX: Regex = Regex::new(r"\[(?:irc|matrix|slack|discord)\] <([^>]+)>").unwrap();
    
    // Match patterns like "<username>" in the author name
    static ref AUTHOR_USERNAME_REGEX: Regex = Regex::new(r"<([^>]+)>").unwrap();
}

// Helper function to check if a message is from a gateway bot and extract the real username
pub fn extract_gateway_username(msg: &Message) -> Option<String> {
    // Don't use cached usernames for gateway bots since multiple users can share the same bot ID
    // Always extract the username from the current message
    
    // Check if the message content starts with a gateway format like "[irc] <username>"
    if let Some(captures) = GATEWAY_USERNAME_REGEX.captures(&msg.content) {
        if let Some(username) = captures.get(1) {
            let extracted = username.as_str().to_string();
            debug!("Extracted gateway username from content: {}", extracted);
            return Some(extracted);
        }
    }
    
    // Check if the author name is in gateway format like "<username>"
    let username = &msg.author.name;
    if let Some(captures) = AUTHOR_USERNAME_REGEX.captures(username) {
        if let Some(username) = captures.get(1) {
            let extracted = username.as_str().to_string();
            debug!("Extracted gateway username from author name: {}", extracted);
            return Some(extracted);
        }
    }
    
    // Check if the author name itself is in gateway format like "<username>"
    if username.starts_with('<') && username.ends_with('>') {
        let extracted = username[1..username.len()-1].to_string();
        debug!("Extracted gateway username from author name brackets: {}", extracted);
        return Some(extracted);
    }
    
    // Check if the message content contains the username in a format like "Ulm_Workin: message"
    // This is a fallback for when the gateway format isn't standard
    if let Some(colon_pos) = msg.content.find(':') {
        if colon_pos > 0 && colon_pos < 30 { // Reasonable username length
            let potential_username = msg.content[0..colon_pos].trim();
            
            // Additional checks to avoid false positives
            // Avoid matching URLs (http:, https:, etc.)
            if !potential_username.is_empty() && 
               !potential_username.contains(' ') && 
               !potential_username.eq_ignore_ascii_case("http") && 
               !potential_username.eq_ignore_ascii_case("https") && 
               !potential_username.eq_ignore_ascii_case("ftp") && 
               !potential_username.contains('/') {
                debug!("Extracted potential gateway username from message prefix: {}", potential_username);
                return Some(potential_username.to_string());
            }
        }
    }
    
    // If we get here, we couldn't extract a username
    None
}

// Helper function to get the best display name for a user
pub async fn get_best_display_name(ctx: &Context, msg: &Message) -> String {
    // Only try to extract gateway username if this is a bot message
    if msg.author.bot {
        if let Some(gateway_username) = extract_gateway_username(msg) {
            debug!("Found gateway username: {}", gateway_username);
            return gateway_username;
        }
    }
    
    let user_id = msg.author.id;
    
    // Prioritize server nickname over global name over username
    if let Some(guild_id) = msg.guild_id {
        // Get member data which includes the nickname
        match guild_id.member(&ctx.http, user_id).await {
            Ok(member) => {
                // Use nickname if available, otherwise fall back to global name or username
                if let Some(nick) = &member.nick {
                    debug!("Using server nickname for {}: {}", user_id, nick);
                    return nick.clone();
                }
            },
            Err(e) => {
                error!("Failed to get member data for {} in guild {}: {:?}", user_id, guild_id, e);
            }
        }
    }
    
    // Fall back to global name if available
    if let Some(global_name) = &msg.author.global_name {
        if !global_name.is_empty() {
            debug!("Using global name for {}: {}", user_id, global_name);
            return global_name.clone();
        }
    }
    
    // Last resort: use username
    debug!("Using username for {}: {}", user_id, msg.author.name);
    msg.author.name.clone()
}

// Get the best display name for a user with explicit guild ID
pub async fn get_best_display_name_with_guild(ctx: &Context, user_id: UserId, guild_id: GuildId) -> String {
    
    // Get member data which includes the nickname
    match guild_id.member(&ctx.http, user_id).await {
        Ok(member) => {
            // Use nickname if available
            if let Some(nick) = &member.nick {
                debug!("Using server nickname for {} in guild {}: {}", user_id, guild_id, nick);
                return nick.clone();
            }
            
            // Fall back to global name if available
            if let Some(global_name) = &member.user.global_name {
                if !global_name.is_empty() {
                    debug!("Using global name for {} in guild {}: {}", user_id, guild_id, global_name);
                    return global_name.clone();
                }
            }
            
            // Last resort: use username
            debug!("Using username for {} in guild {}: {}", user_id, guild_id, member.user.name);
            member.user.name
        },
        Err(e) => {
            error!("Failed to get member data for {} in guild {}: {:?}", user_id, guild_id, e);
            
            // Try to get user data directly
            match ctx.http.get_user(user_id).await {
                Ok(user) => {
                    // Try global name first
                    if let Some(global_name) = &user.global_name {
                        if !global_name.is_empty() {
                            debug!("Using global name for {}: {}", user_id, global_name);
                            return global_name.clone();
                        }
                    }
                    
                    // Fall back to username
                    debug!("Using username for {}: {}", user_id, user.name);
                    user.name
                },
                Err(e) => {
                    error!("Failed to get user data for {}: {:?}", user_id, e);
                    
                    // Instead of returning just the user ID, use a more user-friendly fallback
                    format!("User-{}", user_id.to_string().chars().take(4).collect::<String>())
                }
            }
        }
    }
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
