use crate::db_utils;
use crate::duckduckgo_search::DuckDuckGoSearchClient;
use crate::gemini_api::GeminiClient;
use crate::multi_response_generator::MultiResponseGenerator;
use crate::news_verification;
use anyhow::Result;
use serenity::http::Http;
use serenity::model::channel::Message;
use serenity::model::id::ChannelId;
use serenity::prelude::*;
use std::sync::Arc;
use tokio_rusqlite::Connection;
use tracing::{error, info};

/// Extract topic from response in "TOPIC: description" format
fn extract_topic_from_response(response: &str) -> Option<String> {
    if let Some(topic_start) = response.find("TOPIC:") {
        let after_topic = &response[topic_start + 6..];
        let topic = after_topic.lines().next()?.trim();
        if !topic.is_empty() {
            return Some(topic.to_string());
        }
    }
    None
}

/// Remove the TOPIC tag from the response text for display
fn strip_topic_from_response(response: &str) -> String {
    if let Some(topic_start) = response.find("TOPIC:") {
        let before = &response[..topic_start];
        let after_topic = &response[topic_start + 6..];
        // Find end of TOPIC line
        let rest = if let Some(newline_pos) = after_topic.find('\n') {
            &after_topic[newline_pos + 1..]
        } else {
            // TOPIC is inline - take everything after the topic description
            // Find where the topic ends (next sentence)
            let topic_line = after_topic.lines().next().unwrap_or("");
            &after_topic[topic_line.len()..]
        };
        let cleaned = format!("{}{}", before.trim_end(), rest.trim_start());
        // Clean up double spaces
        cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
    } else {
        response.to_string()
    }
}

/// Search for an article using DuckDuckGo
async fn try_search_for_article(query: &str) -> Option<String> {
    info!("Searching DuckDuckGo for fact source: {}", query);
    let client = DuckDuckGoSearchClient::new();
    match client.search(query).await {
        Ok(Some(result)) => {
            info!("Found search result: {} - {}", result.title, result.url);
            Some(result.url)
        }
        Ok(None) => {
            info!("No search results found for: {}", query);
            None
        }
        Err(e) => {
            error!("DuckDuckGo search failed: {:?}", e);
            None
        }
    }
}

// Handle fact interjection with Message object
pub async fn handle_fact_interjection(
    ctx: &Context,
    msg: &Message,
    gemini_client: &GeminiClient,
    multi_response_generator: &Option<MultiResponseGenerator>,
    message_db: &Option<Arc<tokio::sync::Mutex<Connection>>>,
    bot_name: &str,
    gemini_context_messages: usize,
) -> Result<()> {
    let context_messages = if let Some(db) = message_db {
        match db_utils::get_recent_messages_with_reply_context(
            db.clone(),
            gemini_context_messages,
            Some(msg.channel_id.to_string().as_str()),
        )
        .await
        {
            Ok(messages) => messages,
            Err(e) => {
                error!(
                    "Error retrieving recent messages for fact interjection: {:?}",
                    e
                );
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    handle_fact_interjection_common(
        &ctx.http,
        msg.channel_id,
        gemini_client,
        multi_response_generator,
        &context_messages,
        bot_name,
    )
    .await
}

// Handle fact interjection for spontaneous interjections (without Message object)
pub async fn handle_spontaneous_fact_interjection(
    http: &Http,
    channel_id: ChannelId,
    gemini_client: &GeminiClient,
    multi_response_generator: &Option<MultiResponseGenerator>,
    message_db: &Option<Arc<tokio::sync::Mutex<Connection>>>,
    bot_name: &str,
    gemini_context_messages: usize,
) -> Result<()> {
    let context_messages = if let Some(db) = message_db {
        match db_utils::get_recent_messages_with_reply_context(
            db.clone(),
            gemini_context_messages,
            Some(&channel_id.to_string()),
        )
        .await
        {
            Ok(messages) => messages,
            Err(e) => {
                error!(
                    "Error retrieving recent messages for spontaneous fact interjection: {:?}",
                    e
                );
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    handle_fact_interjection_common(
        http,
        channel_id,
        gemini_client,
        multi_response_generator,
        &context_messages,
        bot_name,
    )
    .await
}

/// Send a fact response with typing delay
async fn send_fact_response(http: &Http, channel_id: ChannelId, response: &str) {
    if let Err(e) = channel_id.broadcast_typing(http).await {
        error!(
            "Failed to send typing indicator for fact interjection: {:?}",
            e
        );
    }

    let words = response.split_whitespace().count();
    let delay_secs = (words as f32 * 0.2).clamp(2.0, 5.0) as u64;
    tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;

    if let Err(e) = channel_id.say(http, response).await {
        error!("Error sending fact interjection: {:?}", e);
    } else {
        info!("Fact interjection sent: {}", response);
    }
}

// Common implementation for both regular and spontaneous fact interjections
async fn handle_fact_interjection_common(
    http: &Http,
    channel_id: ChannelId,
    gemini_client: &GeminiClient,
    multi_response_generator: &Option<MultiResponseGenerator>,
    #[allow(clippy::type_complexity)] context_messages: &[(
        String,
        String,
        Option<String>,
        String,
        Option<String>,
    )],
    _bot_name: &str,
) -> Result<()> {
    // Format context for the prompt
    let context_text = if !context_messages.is_empty() {
        let mut chronological_messages = context_messages.to_owned();
        chronological_messages.reverse();

        let formatted_messages: Vec<String> = chronological_messages
            .iter()
            .map(
                |(_author, display_name, _pronouns, content, reply_context)| {
                    if let Some(reply) = reply_context {
                        format!("{}: {} (in reply to: {})", display_name, content, reply)
                    } else {
                        format!("{}: {}", display_name, content)
                    }
                },
            )
            .collect();
        formatted_messages.join("\n")
    } else {
        info!(
            "No context available for fact interjection in channel_id: {}",
            channel_id
        );
        "".to_string()
    };

    let fact_prompt = gemini_client
        .prompt_templates()
        .format_fact_interjection(&context_text);

    let context_for_api: Vec<(String, String, Option<String>, String)> = context_messages
        .iter()
        .map(
            |(author, display_name, pronouns, content, _reply_context)| {
                (
                    author.clone(),
                    display_name.clone(),
                    pronouns.clone(),
                    content.clone(),
                )
            },
        )
        .collect();

    let response_result = if let Some(multi_gen) = multi_response_generator {
        multi_gen
            .generate_best_response_with_context(&fact_prompt, &context_for_api)
            .await
    } else {
        gemini_client
            .generate_best_response_with_context_and_pronouns(
                &fact_prompt,
                "",
                &context_for_api,
                None,
                false,
            )
            .await
    };

    match response_result {
        Ok(Some(response)) => {
            // Check if the response looks like the prompt itself
            if response.contains("{bot_name}")
                || response.contains("{context}")
                || response.contains("Guidelines:")
            {
                error!("Fact interjection: API returned prompt template instead of response");
                return Ok(());
            }

            // Extract topic and use search-first approach
            if let Some(topic) = extract_topic_from_response(&response) {
                info!("Extracted fact topic for search: {}", topic);
                let display_response = strip_topic_from_response(&response);

                if let Some(url) = try_search_for_article(&topic).await {
                    // Validate the search result
                    match news_verification::verify_news_article(
                        gemini_client,
                        &topic,
                        &url,
                        &display_response,
                    )
                    .await
                    {
                        Ok(true) => {
                            info!("Fact search result validated: {}", url);
                            let final_response = format!("{} Source: {}", display_response, url);
                            send_fact_response(http, channel_id, &final_response).await;
                        }
                        _ => {
                            info!("Fact search result failed validation - sending without URL");
                            send_fact_response(http, channel_id, &display_response).await;
                        }
                    }
                } else {
                    info!("No search results for fact topic - sending without URL");
                    send_fact_response(http, channel_id, &display_response).await;
                }
            } else {
                info!("No TOPIC found in fact response - sending as-is");
                send_fact_response(http, channel_id, &response).await;
            }
        }
        Ok(None) => {
            info!("Fact interjection evaluation: decided to PASS - no response sent");
        }
        Err(e) => {
            error!("Error generating fact interjection: {:?}", e);
        }
    }

    Ok(())
}
