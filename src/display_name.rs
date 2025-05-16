use serenity::model::channel::Message;
use serenity::prelude::*;
use regex::Regex;

// Helper function to get the best display name for a user
pub async fn get_best_display_name(ctx: &Context, msg: &Message) -> String {
    // Prioritize server nickname over global name over username
    if let Some(guild_id) = msg.guild_id {
        // Get member data which includes the nickname
        if let Ok(member) = guild_id.member(&ctx.http, msg.author.id).await {
            // Use nickname if available, otherwise fall back to global name or username
            if let Some(nick) = member.nick {
                return nick;
            }
        }
    }
    
    // Fall back to global name or username if no nickname or couldn't get member data
    msg.author.global_name.clone().unwrap_or_else(|| msg.author.name.clone())
}

// Clean a display name by removing IRC formatting and brackets
pub fn clean_display_name(name: &str) -> String {
    let mut clean_name = name.to_string();
    clean_name = clean_name.replace("<", "").replace(">", "");
    clean_name = clean_name.replace("[irc]", "").trim().to_string();
    clean_name
}

// Extract pronouns from a display name
pub fn extract_pronouns(name: &str) -> Option<String> {
    // Check for pronouns in parentheses (they/them)
    let parentheses_regex = Regex::new(r"\(([^)]*)\)").ok()?;
    if let Some(captures) = parentheses_regex.captures(name) {
        let content = captures.get(1)?.as_str().to_lowercase();
        if content.contains("/") || content.contains("he") || content.contains("she") || 
           content.contains("they") || content.contains("xe") || content.contains("ze") ||
           content.contains("it") || content.contains("fae") {
            return Some(content);
        }
    }
    
    // Check for pronouns in brackets [she/her]
    let brackets_regex = Regex::new(r"\[([^\]]*)\]").ok()?;
    if let Some(captures) = brackets_regex.captures(name) {
        let content = captures.get(1)?.as_str().to_lowercase();
        if content.contains("/") || content.contains("he") || content.contains("she") || 
           content.contains("they") || content.contains("xe") || content.contains("ze") ||
           content.contains("it") || content.contains("fae") {
            return Some(content);
        }
    }
    
    // Check for pronouns after pipe character (Name | they/them)
    let parts: Vec<&str> = name.split('|').collect();
    if parts.len() > 1 {
        let potential_pronouns = parts[1].trim().to_lowercase();
        if potential_pronouns.contains("/") || potential_pronouns.contains("he") || 
           potential_pronouns.contains("she") || potential_pronouns.contains("they") ||
           potential_pronouns.contains("xe") || potential_pronouns.contains("ze") ||
           potential_pronouns.contains("it") || potential_pronouns.contains("fae") {
            return Some(potential_pronouns);
        }
    }
    
    None
}
