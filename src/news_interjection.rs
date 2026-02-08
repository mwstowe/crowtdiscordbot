use crate::db_utils;
use crate::duckduckgo_search::DuckDuckGoSearchClient;
use crate::gemini_api::GeminiClient;
use crate::news_verification;
use crate::response_timing::apply_realistic_delay;
use anyhow::Result;
use serenity::model::channel::Message;
use serenity::prelude::*;
use std::sync::Arc;
use std::time::Duration;
use tokio_rusqlite::Connection;
use tracing::{error, info};

// Extract topic from response in "TOPIC: description" format
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

// Handle news interjection
pub async fn handle_news_interjection(
    ctx: &Context,
    msg: &Message,
    gemini_client: &GeminiClient,
    message_db: &Option<Arc<tokio::sync::Mutex<Connection>>>,
    _bot_name: &str,
    gemini_context_messages: usize,
) -> Result<()> {
    // Get recent messages for context
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
                    "Error retrieving recent messages for news interjection: {:?}",
                    e
                );
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    // Format context for the prompt
    let context_text = if !context_messages.is_empty() {
        // Reverse the messages to get chronological order (oldest first)
        let mut chronological_messages = context_messages.clone();
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
            "No context available for news interjection in channel_id: {}",
            msg.channel_id
        );
        // Use empty string instead of "No recent messages" to avoid showing this in logs
        "".to_string()
    };

    // Create the news prompt using the prompt templates
    let news_prompt = gemini_client
        .prompt_templates()
        .format_news_interjection(&context_text);

    // Call Gemini API with the news prompt using multi-response generation
    match gemini_client
        .generate_best_response_with_context_and_pronouns(
            &news_prompt,
            "",
            &Vec::new(),
            None,
            false, // Let it decide whether to respond for news interjections
        )
        .await
    {
        Ok(Some(response)) => {
            // Check if the response looks like the prompt itself (API error)
            if response.contains("{bot_name}")
                || response.contains("{context}")
                || response.contains("Guidelines:")
            {
                error!("News interjection error: API returned the prompt instead of a response");
                return Ok(());
            }

            // Extract the topic from the response
            if let Some(topic) = extract_topic_from_response(&response) {
                info!("Extracted topic for search: {}", topic);

                // Search for an article about this topic
                if let Some(search_result) = try_search_for_article(&topic).await {
                    // Validate the search result
                    match news_verification::verify_news_article(
                        gemini_client,
                        &topic,
                        &search_result.url,
                        &response,
                    )
                    .await
                    {
                        Ok(true) => {
                            info!(
                                "Search result validated successfully: {}",
                                search_result.url
                            );

                            // Append the validated URL to the response
                            let final_response =
                                format!("{} Source: {}", response, search_result.url);

                            if let Err(e) = msg.channel_id.broadcast_typing(&ctx.http).await {
                                error!("Failed to send typing indicator: {:?}", e);
                            }

                            apply_realistic_delay(&final_response, ctx, msg.channel_id).await;

                            if let Err(e) =
                                msg.channel_id.say(&ctx.http, final_response.clone()).await
                            {
                                error!("Error sending news interjection: {:?}", e);
                            } else {
                                info!(
                                    "News interjection sent with validated URL: {}",
                                    final_response
                                );
                            }
                        }
                        Ok(false) => {
                            info!("Search result failed validation - sending response without URL");

                            // Send the response without a URL rather than skipping entirely
                            if let Err(e) = msg.channel_id.broadcast_typing(&ctx.http).await {
                                error!("Failed to send typing indicator: {:?}", e);
                            }

                            apply_realistic_delay(&response, ctx, msg.channel_id).await;

                            if let Err(e) = msg.channel_id.say(&ctx.http, response.clone()).await {
                                error!("Error sending news interjection: {:?}", e);
                            } else {
                                info!("News interjection sent without URL: {}", response);
                            }
                        }
                        Err(e) => {
                            error!("Error verifying search result: {:?}", e);
                        }
                    }
                } else {
                    info!("No search results found - sending response without URL");

                    // Send the response without a URL
                    if let Err(e) = msg.channel_id.broadcast_typing(&ctx.http).await {
                        error!("Failed to send typing indicator: {:?}", e);
                    }

                    apply_realistic_delay(&response, ctx, msg.channel_id).await;

                    if let Err(e) = msg.channel_id.say(&ctx.http, response.clone()).await {
                        error!("Error sending news interjection: {:?}", e);
                    } else {
                        info!("News interjection sent without URL: {}", response);
                    }
                }
            } else {
                info!("Could not extract topic from response - skipping interjection");
            }
        }
        Ok(None) => {
            info!("News interjection evaluation: decided to PASS - no response sent");
        }
        Err(e) => {
            error!("Error generating news interjection: {:?}", e);
        }
    }

    Ok(())
}

// Function to validate if a URL actually exists, follow redirects, and check content type
pub async fn validate_url_exists(url: &str) -> Result<(bool, Option<String>)> {
    info!("Validating URL exists: {}", url);

    // Create a client with a short timeout that follows redirects
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36")
        .build()?;

    // Try a GET request to follow redirects and check content type
    match client.get(url).send().await {
        Ok(response) => {
            let status = response.status();
            let final_url = response.url().to_string();

            // Check if we were redirected
            let was_redirected = url != final_url;
            if was_redirected {
                info!("URL was redirected: {} -> {}", url, final_url);
            }

            // Check content type
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");

            let is_html = content_type.contains("text/html")
                || content_type.contains("application/xhtml+xml");

            if !is_html {
                info!(
                    "URL validation failed: Content type is not HTML: {}",
                    content_type
                );
                return Ok((false, None));
            }

            if status.is_success() {
                info!(
                    "URL validation successful: {} - Status: {}",
                    final_url, status
                );
                Ok((true, Some(final_url)))
            } else {
                info!("URL validation failed: {} - Status: {}", final_url, status);
                Ok((false, None))
            }
        }
        Err(e) => {
            info!("URL validation failed: {} - Error: {}", url, e);
            Ok((false, None))
        }
    }
}

/// Try to search for a valid article using DuckDuckGo
async fn try_search_for_article(query: &str) -> Option<SearchResult> {
    info!("Searching DuckDuckGo for: {}", query);

    let search_client = DuckDuckGoSearchClient::new();

    match search_client.search(query).await {
        Ok(Some(result)) => {
            info!("Found search result: {} - {}", result.title, result.url);
            Some(SearchResult { url: result.url })
        }
        Ok(None) => {
            info!("No search results found for: {}", query);
            None
        }
        Err(e) => {
            error!("Error searching for article: {:?}", e);
            None
        }
    }
}

/// Simple struct to hold search results
struct SearchResult {
    url: String,
}
