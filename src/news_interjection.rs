use crate::db_utils;
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

    // Call Gemini API with the news prompt
    match gemini_client
        .generate_response_with_context_and_pronouns(&news_prompt, "", &Vec::new(), None)
        .await
    {
        Ok(response) => {
            // Check if the response starts with "pass" (case-insensitive) - if so, don't send anything
            if response.trim().to_lowercase().starts_with("pass") {
                info!("News interjection evaluation: decided to PASS - no response sent");
                return Ok(());
            }

            // Check if the response looks like the prompt itself (API error)
            if response.contains("{bot_name}")
                || response.contains("{context}")
                || response.contains("Guidelines:")
                || response.contains("Example good response:")
            {
                error!("News interjection error: API returned the prompt instead of a response");
                return Ok(());
            }

            // Validate the news article and reference with a second API call
            let validation_prompt = format!(
                "Validate this news article reference:\n\nArticle: {}\n\nCheck if:\n1. Any URLs mentioned exist and are accessible\n2. The article summary matches the actual content\n3. The source appears to be a legitimate news/tech website\n\nRespond with only: VALID or INVALID",
                response.trim()
            );

            match gemini_client.generate_content(&validation_prompt).await {
                Ok(validation_response) => {
                    let validation = validation_response.trim().to_uppercase();
                    if !validation.starts_with("VALID") {
                        info!(
                            "News interjection validation failed: {} - skipping interjection",
                            validation
                        );
                        return Ok(());
                    }
                    info!("News interjection validation passed: {}", validation);
                }
                Err(e) => {
                    error!("News interjection validation API call failed: {:?} - skipping interjection", e);
                    return Ok(());
                }
            }

            // Check for self-reference issues
            if response.contains("I'm Crow")
                || response.contains("As Crow")
                || response.contains("handsome") && response.contains("modest")
                || response.contains("Satellite of Love")
            {
                error!(
                    "News interjection error: Response contains self-reference: {}",
                    response
                );
                return Ok(());
            }

            // Validate URL format using our new validator
            if !news_verification::verify_url_format(&response) {
                error!(
                    "News interjection error: Invalid URL format in response: {}",
                    response
                );
                return Ok(());
            }

            // Extract article title, URL, and summary
            if let Some((title, url)) = news_verification::extract_article_info(&response) {
                // Get the summary (everything after the URL)
                let url_pos = response.find(&url).unwrap_or(0);
                let summary = if url_pos + url.len() < response.len() {
                    response[(url_pos + url.len())..].trim().to_string()
                } else {
                    String::new()
                };

                // Validate that the URL actually exists and follow redirects
                match validate_url_exists(&url).await {
                    Ok((true, Some(final_url))) => {
                        // URL exists, now verify that the title and summary match the content
                        match news_verification::verify_news_article(
                            gemini_client,
                            &title,
                            &final_url,
                            &summary,
                        )
                        .await
                        {
                            Ok(true) => {
                                // Title and summary match the URL content
                                info!("News verification successful: Title and summary match URL content");

                                // Replace the original URL with the final URL if they're different
                                let final_response = if url != final_url {
                                    response.replace(&url, &final_url)
                                } else {
                                    response
                                };

                                // Start typing indicator now that we've decided to send a message
                                if let Err(e) = msg.channel_id.broadcast_typing(&ctx.http).await {
                                    error!("Failed to send typing indicator for news interjection: {:?}", e);
                                }

                                // Apply realistic typing delay
                                apply_realistic_delay(&final_response, ctx, msg.channel_id).await;

                                // Send the response
                                let response_text = final_response.clone(); // Clone for logging
                                if let Err(e) = msg.channel_id.say(&ctx.http, final_response).await
                                {
                                    error!("Error sending news interjection: {:?}", e);
                                } else {
                                    info!("News interjection sent: {}", response_text);
                                }
                            }
                            Ok(false) => {
                                // Title and summary don't match the URL content
                                info!("News interjection skipped: Title/summary mismatch with URL content");
                            }
                            Err(e) => {
                                // Error verifying title and summary
                                error!("Error verifying news article title/summary: {:?}", e);
                            }
                        }
                    }
                    Ok((true, None)) => {
                        // URL exists but we couldn't get the final URL
                        info!(
                            "News interjection skipped: URL exists but couldn't get final URL: {}",
                            url
                        );
                    }
                    Ok((false, _)) => {
                        // URL doesn't exist or isn't HTML
                        info!(
                            "News interjection skipped: URL doesn't exist or isn't HTML: {}",
                            url
                        );
                    }
                    Err(e) => {
                        // Error validating URL
                        error!("Error validating URL {}: {:?}", url, e);
                    }
                }
            } else {
                info!("News interjection skipped: Couldn't extract article title and URL");
            }
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
