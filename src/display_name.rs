use lazy_static::lazy_static;
use regex::Regex;
use serenity::model::channel::Message;
use serenity::model::id::{GuildId, UserId};
use serenity::prelude::*;
use tracing::{debug, error};

// Regular expression for extracting gateway usernames from bot messages
lazy_static! {
    // Match patterns like "[irc] <username>" in the message content
    static ref GATEWAY_USERNAME_REGEX: Regex = Regex::new(r"\[(?:irc|matrix|slack|discord)\] <([^>]+)>").unwrap();

    // Match patterns like "<username>" in the author name
    static ref AUTHOR_USERNAME_REGEX: Regex = Regex::new(r"<([^>]+)>").unwrap();

    // IRC formatting codes (bold, italic, underline, color)
    static ref IRC_FORMATTING_REGEX: Regex = Regex::new(r"[\x02\x1D\x1F\x03\x0F](?:\d{1,2}(?:,\d{1,2})?)?").unwrap();

    // Pronouns in parentheses at the end of a name
    static ref PRONOUNS_STRIP_REGEX: Regex = Regex::new(r"\s*\([^)]+\)\s*$").unwrap();

    // Pronouns capture in parentheses at the end of a name
    static ref PRONOUNS_CAPTURE_REGEX: Regex = Regex::new(r"\s*\(([^)]+)\)\s*$").unwrap();
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
        let extracted = username[1..username.len() - 1].to_string();
        debug!(
            "Extracted gateway username from author name brackets: {}",
            extracted
        );
        return Some(extracted);
    }

    // Check if the message content contains the username in a format like "Ulm_Workin: message"
    // This is a fallback for when the gateway format isn't standard
    if let Some(colon_pos) = msg.content.find(':') {
        if colon_pos > 0 && colon_pos < 30 {
            // Reasonable username length
            let potential_username = msg.content[0..colon_pos].trim();

            // Additional checks to avoid false positives
            // Avoid matching URLs (http:, https:, etc.)
            if !potential_username.is_empty()
                && !potential_username.contains(' ')
                && !potential_username.eq_ignore_ascii_case("http")
                && !potential_username.eq_ignore_ascii_case("https")
                && !potential_username.eq_ignore_ascii_case("ftp")
                && !potential_username.contains('/')
            {
                debug!(
                    "Extracted potential gateway username from message prefix: {}",
                    potential_username
                );
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
            }
            Err(e) => {
                error!(
                    "Failed to get member data for {} in guild {}: {:?}",
                    user_id, guild_id, e
                );
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
pub async fn get_best_display_name_with_guild(
    ctx: &Context,
    user_id: UserId,
    guild_id: GuildId,
) -> String {
    // Get member data which includes the nickname
    match guild_id.member(&ctx.http, user_id).await {
        Ok(member) => {
            // Use nickname if available
            if let Some(nick) = &member.nick {
                debug!(
                    "Using server nickname for {} in guild {}: {}",
                    user_id, guild_id, nick
                );
                return nick.clone();
            }

            // Fall back to global name if available
            if let Some(global_name) = &member.user.global_name {
                if !global_name.is_empty() {
                    debug!(
                        "Using global name for {} in guild {}: {}",
                        user_id, guild_id, global_name
                    );
                    return global_name.clone();
                }
            }

            // Last resort: use username
            debug!(
                "Using username for {} in guild {}: {}",
                user_id, guild_id, member.user.name
            );
            member.user.name
        }
        Err(e) => {
            error!(
                "Failed to get member data for {} in guild {}: {:?}",
                user_id, guild_id, e
            );

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
                }
                Err(e) => {
                    error!("Failed to get user data for {}: {:?}", user_id, e);

                    // Instead of returning just the user ID, use a more user-friendly fallback
                    let user_id_short = user_id.to_string().chars().take(4).collect::<String>();
                    format!("User-{user_id_short}")
                }
            }
        }
    }
}

// Clean a display name by removing IRC formatting, brackets, and pronouns
pub fn clean_display_name(name: &str) -> String {
    // If the name is already in gateway format (within <> brackets), strip the brackets
    if name.starts_with('<') && name.ends_with('>') {
        return name[1..name.len() - 1].to_string();
    }

    // First remove IRC formatting
    let mut clean_name = name.to_string();

    // Remove IRC formatting codes (bold, italic, underline, color)
    clean_name = IRC_FORMATTING_REGEX
        .replace_all(&clean_name, "")
        .to_string();

    // Remove pronouns in parentheses at the end of the name
    clean_name = PRONOUNS_STRIP_REGEX.replace(&clean_name, "").to_string();

    clean_name
}

// Escape Discord markdown characters in a name so it renders literally.
// Without this, names like "shining_teh_" get interpreted as italics by Discord.
pub fn escape_markdown(name: &str) -> String {
    let mut escaped = String::with_capacity(name.len());
    for c in name.chars() {
        // These characters are meaningful in Discord markdown
        if matches!(c, '_' | '*' | '~' | '`' | '|' | '\\' | '>') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

// Returns true if the name contains any Discord markdown-significant characters
// that would need escaping when rendered.
pub fn needs_markdown_escape(name: &str) -> bool {
    name.chars()
        .any(|c| matches!(c, '_' | '*' | '~' | '`' | '|' | '\\' | '>'))
}

// Escape any of the given names where they appear verbatim in `text`.
//
// Algorithm:
//   1. Skip names that don't need escaping.
//   2. For names that do, replace each exact occurrence of the raw name
//      with its markdown-escaped form.
//
// This is intended for post-processing Gemini responses: the model sees the
// natural (unescaped) name, and only the rendered output is escaped. Names are
// processed longest-first so that a longer name is escaped before a shorter
// name that might be a substring of it.
pub fn escape_names_in_text(text: &str, names: &[String]) -> String {
    // Deduplicate and keep only names that actually need escaping.
    let mut candidates: Vec<&String> = names
        .iter()
        .filter(|n| !n.is_empty() && needs_markdown_escape(n))
        .collect();
    candidates.sort_by_key(|n| std::cmp::Reverse(n.len()));
    candidates.dedup();

    let mut result = text.to_string();
    for name in candidates {
        let escaped = escape_markdown(name);
        // Only replace if the escaped form isn't already present, to avoid
        // double-escaping if the model happened to echo an escaped name.
        if result.contains(name.as_str()) {
            result = result.replace(name.as_str(), &escaped);
        }
    }
    result
}

// Extract pronouns from a display name
pub fn extract_pronouns(name: &str) -> Option<String> {
    // Look for pronouns in parentheses at the end of the name
    if let Some(captures) = PRONOUNS_CAPTURE_REGEX.captures(name) {
        if let Some(pronouns) = captures.get(1) {
            return Some(pronouns.as_str().to_string());
        }
    }
    None
}

// Check if a string looks like a user ID (all digits)
pub fn is_user_id(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_digit())
}
