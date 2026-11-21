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

    // A web link; names inside one must never be altered or the link breaks
    static ref URL_REGEX: Regex = Regex::new(r"https?://[^\s<>]+").unwrap();

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

    // Check if the message content contains the username in a format like "Jane_Doe: message"
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

// The ways a stored display name may be written by someone (or the model)
// referring to that person. The model reads "<IRC_Pat>: hi" or
// "[irc] <IRC_Pat>: hi" in its context but will naturally write "IRC_Pat",
// and a name stored as "shining_teh_ (they/them)" is written "shining_teh_".
fn name_variants(name: &str) -> Vec<String> {
    let mut variants: Vec<String> = Vec::new();
    let mut add = |candidate: &str| {
        let candidate = candidate.trim();
        if !candidate.is_empty() && !variants.iter().any(|v| v == candidate) {
            variants.push(candidate.to_string());
        }
    };

    add(name);
    // Gateway forms: "[irc] <user>" or "<user>" -> "user"
    let inner = GATEWAY_USERNAME_REGEX
        .captures(name)
        .or_else(|| AUTHOR_USERNAME_REGEX.captures(name))
        .map(|c| c[1].to_string());
    for base in std::iter::once(name).chain(inner.as_deref()) {
        add(&clean_display_name(base));
    }
    variants
}

// Escape the given names wherever they appear in `text`, so Discord shows them
// literally instead of treating underscores, asterisks etc. as formatting.
//
// This is for post-processing model output: the model sees (and writes) the
// natural name, and only the rendered text is escaped. It
//   * recognises each name in the forms people write it (see `name_variants`)
//     and regardless of capitalisation, preferring the longest name where
//     names overlap, in a single pass so one replacement can't affect another;
//   * never touches text inside a URL, which would break the link;
//   * leaves text that is already escaped alone, so applying it twice, or to
//     text that has been through another escaping step, changes nothing.
pub fn escape_names_in_text(text: &str, names: &[String]) -> String {
    let mut variants: Vec<String> = names
        .iter()
        .flat_map(|n| name_variants(n))
        .filter(|v| needs_markdown_escape(v))
        .collect();
    if variants.is_empty() || text.is_empty() {
        return text.to_string();
    }
    // Longest first: at one position the regex prefers the earlier alternative.
    variants.sort_by(|a, b| {
        b.chars()
            .count()
            .cmp(&a.chars().count())
            .then_with(|| a.cmp(b))
    });
    variants.dedup();

    let pattern = format!(
        "(?i){}",
        variants
            .iter()
            .map(|v| regex::escape(v))
            .collect::<Vec<_>>()
            .join("|")
    );
    let Ok(name_regex) = Regex::new(&pattern) else {
        error!("Could not build the name-escaping pattern; leaving text unescaped");
        return text.to_string();
    };

    let url_spans: Vec<(usize, usize)> = URL_REGEX
        .find_iter(text)
        .map(|m| (m.start(), m.end()))
        .collect();

    let mut out = String::with_capacity(text.len() + 8);
    let mut copied_to = 0;
    for m in name_regex.find_iter(text) {
        let (start, end) = (m.start(), m.end());
        let inside_url = url_spans.iter().any(|&(us, ue)| start < ue && end > us);
        let already_escaped = text[..start].ends_with('\\');
        if inside_url || already_escaped {
            continue;
        }
        out.push_str(&text[copied_to..start]);
        out.push_str(&escape_markdown(m.as_str()));
        copied_to = end;
    }
    out.push_str(&text[copied_to..]);
    out
}

// The names to escape for a set of conversation rows given as
// (author, display_name): the display name, or the author when it is empty.
pub fn names_of<'a, I>(rows: I) -> Vec<String>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    rows.into_iter()
        .map(|(author, display)| if display.is_empty() { author } else { display })
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn escapes_an_exact_name() {
        assert_eq!(
            escape_names_in_text("shining_teh_, hi", &names(&["shining_teh_"])),
            "shining\\_teh\\_, hi"
        );
    }

    /// The model reads "<IRC_Pat>: ..." in its context but writes "IRC_Pat".
    #[test]
    fn escapes_gateway_names_echoed_without_brackets() {
        let stored = names(&["<Jane_Doe>", "[irc] <IRC_Pat>", "<pat_the_cat>"]);
        assert_eq!(
            escape_names_in_text("Jane_Doe and IRC_Pat meet pat_the_cat", &stored),
            "Jane\\_Doe and IRC\\_Pat meet pat\\_the\\_cat"
        );
        // Echoed with the brackets is escaped too, as a whole.
        assert_eq!(
            escape_names_in_text("hello <Jane_Doe>", &stored),
            "hello <Jane\\_Doe\\>"
        );
    }

    #[test]
    fn escapes_names_stored_with_pronouns_or_irc_formatting() {
        assert_eq!(
            escape_names_in_text("shining_teh_ rules", &names(&["shining_teh_ (they/them)"])),
            "shining\\_teh\\_ rules"
        );
        assert_eq!(
            escape_names_in_text("a_b_ rules", &names(&["\u{2}a_b_\u{f}"])),
            "a\\_b\\_ rules"
        );
    }

    #[test]
    fn matches_the_name_whatever_case_the_model_uses() {
        assert_eq!(
            escape_names_in_text("Shining_Teh_ wins", &names(&["shining_teh_"])),
            "Shining\\_Teh\\_ wins"
        );
    }

    #[test]
    fn running_it_twice_changes_nothing() {
        let stored = names(&["_dave", "shining_teh_", "<Jane_Doe>", "*star*"]);
        let once = escape_names_in_text("_dave, shining_teh_, Jane_Doe and *star*", &stored);
        assert_eq!(once, "\\_dave, shining\\_teh\\_, Jane\\_Doe and \\*star\\*");
        assert_eq!(escape_names_in_text(&once, &stored), once);
    }

    #[test]
    fn already_escaped_text_is_left_alone() {
        let text = "shining\\_teh\\_ is here";
        assert_eq!(escape_names_in_text(text, &names(&["shining_teh_"])), text);
    }

    #[test]
    fn urls_are_never_altered() {
        let text = "see https://en.wikipedia.org/wiki/Pat_The_Cat_ for Pat_The_Cat_";
        assert_eq!(
            escape_names_in_text(text, &names(&["Pat_The_Cat_"])),
            "see https://en.wikipedia.org/wiki/Pat_The_Cat_ for Pat\\_The\\_Cat\\_"
        );
    }

    #[test]
    fn overlapping_names_prefer_the_longer_one() {
        assert_eq!(
            escape_names_in_text("bob_smith_ and bob_", &names(&["bob_", "bob_smith_"])),
            "bob\\_smith\\_ and bob\\_"
        );
    }

    #[test]
    fn ordinary_names_and_empty_input_are_untouched() {
        assert_eq!(
            escape_names_in_text("hi Bob", &names(&["Bob", ""])),
            "hi Bob"
        );
        assert_eq!(escape_names_in_text("", &names(&["a_b_"])), "");
        assert_eq!(escape_names_in_text("a_b_", &[]), "a_b_");
    }

    #[test]
    fn names_of_prefers_display_name_and_falls_back_to_author() {
        let rows = [("author_a", "Display_A"), ("author_b", ""), ("", "")];
        assert_eq!(names_of(rows), vec!["Display_A", "author_b"]);
    }

    #[test]
    fn names_that_need_regex_escaping_do_not_break_matching() {
        assert_eq!(
            escape_names_in_text("hey (x)_y_ and a.b*", &names(&["(x)_y_", "a.b*"])),
            "hey (x)\\_y\\_ and a.b\\*"
        );
    }
}
