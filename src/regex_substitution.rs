use crate::display_name::get_best_display_name;
use anyhow::Result;
use regex::Regex;
use serenity::model::channel::Message;
use serenity::prelude::*;
use tracing::{error, info, warn};

// URL pattern for detecting URLs in text
const URL_PATTERN: &str = r"https?://[^\s/$.?#].[^\s]*";

// Special regex characters that might need escaping
const REGEX_SPECIAL_CHARS: &[char] = &[
    '.', '+', '*', '?', '^', '$', '(', ')', '[', ']', '{', '}', '|', '\\',
];

// Function to handle potential regex special characters in user input
fn sanitize_regex_pattern(pattern: &str) -> String {
    // Note: Smart quote replacement was removed as it was redundant
    // If smart quote handling is needed in the future, implement with actual smart quote characters
    let pattern = pattern.to_string();

    // We don't want to escape everything automatically because users might intentionally
    // use regex special characters. Just log the presence of special characters.
    for &c in REGEX_SPECIAL_CHARS {
        if pattern.contains(c) {
            info!(
                "Pattern contains regex special character '{}' which may need escaping",
                c
            );
        }
    }

    pattern
}

// Handle regex substitution for messages starting with !s/, .s/, !/, or ./
pub async fn handle_regex_substitution(ctx: &Context, msg: &Message) -> Result<()> {
    // Log the guild ID for debugging
    if let Some(guild_id) = msg.guild_id {
        info!("Processing regex substitution in guild: {}", guild_id);
    } else {
        info!("Processing regex substitution in DM or group");
    }

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
            &replacement_part[0..replacement_part.len() - 1]
        } else {
            // No trailing slash
            replacement_part
        }
    };

    // Extract flags if present
    let flags = if parts.len() > 3 { parts[3] } else { "" };
    let case_insensitive = flags.contains('i');

    // Log the substitution attempt
    info!(
        "Regex substitution attempt: pattern='{}', replacement='{}', flags='{}'",
        pattern, replacement, flags
    );

    // Get the last four messages from the channel
    let builder = serenity::builder::GetMessages::new()
        .before(msg.id)
        .limit(4);
    let messages = msg.channel_id.messages(&ctx.http, builder).await?;

    // Get the bot's user ID
    let bot_id = ctx.http.get_current_user().await?.id;

    // Check if the most recent message is a bot regex response
    let is_bot_regex_response = messages
        .first()
        .map(|m| {
            m.author.id == bot_id
                && (m.content.contains(" meant: ") || m.content.contains(" *really* meant: "))
        })
        .unwrap_or(false);

    // Count how many "really" are in the message if it's a bot regex response
    let really_count = if is_bot_regex_response {
        if let Some(msg_content) = messages.first().map(|m| &m.content) {
            // Count occurrences of "*really*" in the message
            let re = Regex::new(r"\*really\*").unwrap_or_else(|_| Regex::new(r"").unwrap());
            re.find_iter(msg_content).count()
        } else {
            0
        }
    } else {
        0
    };

    // Extract the original author's name from the bot regex response if applicable
    let original_author = if is_bot_regex_response {
        if let Some(first_msg) = messages.first() {
            info!("Extracting original author from bot message: '{}'", first_msg.content);
            // Use regex to extract the original author's name
            let re = Regex::new(r"^(.*?) (?:\*really\* )*meant: ").unwrap_or_else(|_| {
                error!("Failed to compile regex for extracting author name");
                Regex::new(r".*").unwrap() // Fallback regex that matches everything
            });

            if let Some(captures) = re.captures(&first_msg.content) {
                let extracted_name = captures
                    .get(1)
                    .map(|name_match| name_match.as_str().to_string());
                info!("Extracted original author name: {:?}", extracted_name);
                extracted_name
            } else {
                info!("Failed to extract original author name from message");
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    // Filter out commands and bot messages (except regex responses if they're the most recent)
    let valid_messages: Vec<&Message> = messages
        .iter()
        .enumerate()
        .filter(|(i, m)| {
            // Allow regular messages
            (!m.content.starts_with('!') && !m.content.starts_with('.')) ||
            // Allow the most recent message if it's a bot regex response
            (*i == 0 && is_bot_regex_response)
        })
        .map(|(_, m)| m)
        .collect();

    // Sanitize the pattern to handle special characters
    let sanitized_pattern = sanitize_regex_pattern(pattern);

    // Try to build the regex
    let regex_result = if case_insensitive {
        regex::RegexBuilder::new(&sanitized_pattern)
            .case_insensitive(true)
            .build()
    } else {
        regex::RegexBuilder::new(&sanitized_pattern).build()
    };

    // Compile URL detection regex
    let url_regex = Regex::new(URL_PATTERN).expect("Invalid URL pattern regex");

    // Compile regexes used inside the loop
    let extract_content_regex =
        Regex::new(r".*? (?:\*really\* )*meant: (.*)").unwrap_or_else(|_| {
            error!("Failed to compile regex for extracting message content");
            Regex::new(r".*").unwrap() // Fallback regex that matches everything
        });

    let extract_author_regex = Regex::new(r"^(.*?) (?:\*really\* )*meant: ").unwrap_or_else(|_| {
        error!("Failed to compile regex for extracting author name");
        Regex::new(r".*").unwrap() // Fallback regex that matches everything
    });

    match regex_result {
        Ok(re) => {
            // Try each message in order from most recent to least recent
            for (i, prev_msg) in valid_messages.iter().enumerate() {
                // Extract the content to modify
                let content_to_modify = if i == 0 && is_bot_regex_response {
                    // If this is a bot regex response, extract just the message content without the prefix
                    // Use regex to handle any number of "really" occurrences
                    if let Some(captures) = extract_content_regex.captures(&prev_msg.content) {
                        if let Some(content_match) = captures.get(1) {
                            content_match.as_str().to_string()
                        } else {
                            prev_msg.content.clone()
                        }
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
                    let original_urls: Vec<&str> = url_regex
                        .find_iter(&content_to_modify)
                        .map(|m| m.as_str())
                        .collect();

                    // Get all URLs from new message
                    let new_urls: Vec<&str> = url_regex
                        .find_iter(&new_content)
                        .map(|m| m.as_str())
                        .collect();

                    // Check if any URLs were modified
                    if original_urls != new_urls {
                        warn!("Regex substitution would modify URLs - skipping");
                        continue; // Try next message
                    }

                    // Get the display name of the original message author
                    let display_name = if i == 0 && is_bot_regex_response {
                        // If this is a bot regex response, use the extracted original author's name
                        info!("Using bot regex response path, original_author: {:?}", original_author);
                        if let Some(ref author_name) = original_author {
                            info!("Using extracted original author name: {}", author_name);
                            author_name.clone()
                        } else {
                            info!("No original author extracted, falling back to regex extraction");
                            // Fallback to extracting from the message content
                            if let Some(captures) = extract_author_regex.captures(&prev_msg.content)
                            {
                                if let Some(name_match) = captures.get(1) {
                                    let extracted = name_match.as_str().to_string();
                                    info!("Fallback extraction successful: {}", extracted);
                                    extracted
                                } else {
                                    info!("Fallback extraction failed, using best display name");
                                    get_best_display_name(ctx, prev_msg).await
                                }
                            } else {
                                info!("Fallback regex failed, using best display name");
                                get_best_display_name(ctx, prev_msg).await
                            }
                        }
                    } else if prev_msg.author.bot {
                        // Check if this is the bot's own message
                        if prev_msg.author.id == bot_id {
                            // Use "I" for the bot's own messages to make it more natural
                            "I".to_string()
                        } else if let Some(gateway_username) =
                            crate::display_name::extract_gateway_username(prev_msg)
                        {
                            // Check if this is a gateway bot message and extract the gateway username
                            gateway_username
                        } else {
                            // For other bot messages, get the display name of the original author
                            // Use the guild ID from the current message since it's more reliable
                            if let Some(guild_id) = msg.guild_id {
                                // Try to get the display name with guild context first
                                crate::display_name::get_best_display_name_with_guild(
                                    ctx,
                                    prev_msg.author.id,
                                    guild_id,
                                )
                                .await
                            } else {
                                get_best_display_name(ctx, prev_msg).await
                            }
                        }
                    } else {
                        // For regular messages, get the display name of the original author
                        // Use the guild ID from the current message since it's more reliable
                        if let Some(guild_id) = msg.guild_id {
                            // Try to get the display name with guild context first
                            let name = crate::display_name::get_best_display_name_with_guild(
                                ctx,
                                prev_msg.author.id,
                                guild_id,
                            )
                            .await;

                            // If the name looks like a user ID (all digits), try to get a better name
                            if crate::display_name::is_user_id(&name) {
                                // Fall back to the username from the message if available
                                prev_msg
                                    .author
                                    .global_name
                                    .clone()
                                    .unwrap_or_else(|| prev_msg.author.name.clone())
                            } else {
                                name
                            }
                        } else {
                            get_best_display_name(ctx, prev_msg).await
                        }
                    };

                    // Clean the display name
                    let clean_display_name = crate::display_name::clean_display_name(&display_name)
                        .trim()
                        .to_string();

                    // Format and send the response
                    let response = if i == 0 && is_bot_regex_response {
                        // For a bot regex response, we need to keep the original author's name
                        // and add one more "really" to indicate another substitution
                        // The clean_display_name here should be the original author, not "Crow"
                        let really_part = "*really* ".repeat(really_count + 1);
                        format!("{clean_display_name} {really_part}meant: {new_content}")
                    } else {
                        format!("{clean_display_name} meant: {new_content}")
                    };

                    if let Err(e) = msg.channel_id.say(&ctx.http, response).await {
                        error!("Error sending regex substitution response: {:?}", e);
                    }

                    // Stop after first successful substitution
                    return Ok(());
                }
            }
            // If we get here, no substitutions worked - silently give up
        }
        Err(e) => {
            error!("Invalid regex pattern '{}': {:?}", pattern, e);

            // Check if the error is likely due to an apostrophe
            if pattern.contains("'") {
                info!("Pattern contains apostrophes which may cause regex parsing issues");
            }

            // Silently fail - don't notify the user of regex errors
        }
    }

    Ok(())
}
