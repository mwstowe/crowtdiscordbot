use crate::db_utils;
use crate::gemini_api::GeminiClient;
use crate::news_feed::{Headline, HeadlineCache};
use crate::response_timing::apply_realistic_delay;
use anyhow::Result;
use serenity::model::channel::Message;
use serenity::prelude::*;
use std::sync::Arc;
use tokio_rusqlite::Connection;
use tracing::{error, info};

// Handle news interjection using real headlines from RSS feeds
pub async fn handle_news_interjection(
    ctx: &Context,
    msg: &Message,
    gemini_client: &GeminiClient,
    message_db: &Option<Arc<tokio::sync::Mutex<Connection>>>,
    _bot_name: &str,
    gemini_context_messages: usize,
    headline_cache: &HeadlineCache,
) -> Result<()> {
    // Get cached headlines
    let headlines = headline_cache.read().await;
    if headlines.is_empty() {
        info!("News interjection: no headlines cached yet");
        return Ok(());
    }

    // Get recent conversation context
    let context_text = if let Some(db) = message_db {
        match db_utils::get_recent_messages_with_reply_context(
            db.clone(),
            gemini_context_messages,
            Some(msg.channel_id.to_string().as_str()),
        )
        .await
        {
            Ok(messages) => {
                let mut chronological = messages;
                chronological.reverse();
                chronological
                    .iter()
                    .map(|(_author, display_name, _pronouns, content, _reply)| {
                        format!("{}: {}", display_name, content)
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            Err(_) => String::new(),
        }
    } else {
        String::new()
    };

    // Format headlines for Gemini to pick from
    let headline_list: String = headlines
        .iter()
        .enumerate()
        .take(30)
        .map(|(i, h)| format!("{}. [{}] {} - {}", i + 1, h.source, h.title, h.url))
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = format!(
        "You are a witty Discord bot. Below are real headlines from news feeds, and the recent conversation.\n\n\
        HEADLINES:\n{headline_list}\n\n\
        RECENT CONVERSATION:\n{context_text}\n\n\
        Pick ONE headline that is either:\n\
        1. Breaking/interesting enough to share on its own (celebrity death, major event, weird story)\n\
        2. Relevant to what's being discussed\n\n\
        If nothing is interesting or relevant, respond with ONLY the word \"pass\".\n\n\
        Otherwise, respond with EXACTLY this format:\n\
        NUMBER: [the headline number you picked]\n\
        COMMENT: [1-2 sentence witty comment about why this is interesting or how it relates to the conversation]\n\n\
        Rules:\n\
        - Do NOT put anything in quotation marks\n\
        - Do NOT include the URL in your comment (we add it automatically)\n\
        - Do NOT explain what the article is about if the title already says it\n\
        - Be brief and natural, like sharing a link with friends\n\
        - If nothing genuinely stands out, just pass"
    );

    match gemini_client.generate_content(&prompt).await {
        Ok(response) => {
            let trimmed = response.trim();

            if trimmed.to_lowercase().starts_with("pass") {
                info!("News interjection: AI decided to pass");
                return Ok(());
            }

            // Parse the response
            if let Some((headline, comment)) = parse_selection(trimmed, &headlines) {
                let final_message = format!("{} {}", comment, headline.url);

                if let Err(e) = msg.channel_id.broadcast_typing(&ctx.http).await {
                    error!("Failed to send typing indicator: {:?}", e);
                }

                apply_realistic_delay(&final_message, ctx, msg.channel_id).await;

                if let Err(e) = msg.channel_id.say(&ctx.http, &final_message).await {
                    error!("Error sending news interjection: {:?}", e);
                } else {
                    info!("News interjection sent: {}", final_message);
                }
            } else {
                info!(
                    "News interjection: could not parse AI selection: {}",
                    trimmed
                );
            }
        }
        Err(e) => {
            error!("News interjection API error: {:?}", e);
        }
    }

    Ok(())
}

/// Parse the AI's selection response to extract the chosen headline and comment
fn parse_selection(response: &str, headlines: &[Headline]) -> Option<(Headline, String)> {
    let mut number: Option<usize> = None;
    let mut comment: Option<String> = None;

    for line in response.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("NUMBER:") {
            number = rest.trim().parse::<usize>().ok();
        } else if let Some(rest) = line.strip_prefix("COMMENT:") {
            comment = Some(rest.trim().to_string());
        }
    }

    let idx = number?.checked_sub(1)?;
    let headline = headlines.get(idx)?.clone();
    let comment = comment?;

    if comment.is_empty() {
        return None;
    }

    Some((headline, comment))
}
