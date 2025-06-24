use anyhow::Result;
use serenity::model::channel::Message;
use serenity::prelude::*;
use tracing::{error, info};
use crate::db_utils;
use crate::response_timing::apply_realistic_delay;
use crate::gemini_api::GeminiClient;
use std::sync::Arc;
use tokio_rusqlite::Connection;
use regex::Regex;
use url::Url;
use reqwest;
use std::time::Duration;

// Handle news interjection
pub async fn handle_news_interjection(
    ctx: &Context,
    msg: &Message,
    gemini_client: &GeminiClient,
    message_db: &Option<Arc<tokio::sync::Mutex<Connection>>>,
    bot_name: &str,
    gemini_context_messages: usize,
) -> Result<()> {
    // Get recent messages for context
    let context_messages = if let Some(db) = message_db {
        match db_utils::get_recent_messages(db.clone(), gemini_context_messages, Some(msg.channel_id.to_string().as_str())).await {
            Ok(messages) => messages,
            Err(e) => {
                error!("Error retrieving recent messages for news interjection: {:?}", e);
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
        
        let formatted_messages: Vec<String> = chronological_messages.iter()
            .map(|(_author, display_name, content)| format!("{}: {}", display_name, content))
            .collect();
        formatted_messages.join("\n")
    } else {
        info!("No context available for news interjection in channel_id: {}", msg.channel_id);
        // Use empty string instead of "No recent messages" to avoid showing this in logs
        "".to_string()
    };
    
    // Create the news prompt
    let news_prompt = String::from(r#"You are {bot_name}, a Discord bot. Share an interesting technology or weird news article link with a brief comment about why it's interesting.

{context}

Guidelines:
1. Create a fictional but plausible news article link about technology or weird news (NO sports)
2. Format as: "Article title: https://example.com/article-path"
3. The URL must be specific and detailed (e.g., https://arstechnica.com/tech-policy/2025/06/new-ai-regulations-impact-open-source/)
4. Never use generic URLs like https://arstechnica.com/ or https://techcrunch.com/
5. Always include year, month, and a descriptive path in the URL
6. Then add a brief comment (1-2 sentences) on why it's interesting or relevant to the conversation
7. If possible, relate it to the conversation, but don't force it
8. Don't use phrases like "Check out this article" or "You might find this interesting"
9. NEVER include tags like "(via search)", "(via Google)", or any other source attribution
10. If you can't think of a relevant article, respond with "pass"

Example good response: "AI Creates Perfect Pizza Recipe Through Taste Simulation: https://techcrunch.com/2025/06/ai-taste-simulation-pizza This shows how AI sensory processing is advancing beyond visual and audio into taste simulation."

Example bad response: "Check out this interesting article about AI and food: https://techcrunch.com/ai-food-article (via search) I thought you might find this interesting given our conversation about technology."

Be creative but realistic with your article title and URL."#)
        .replace("{bot_name}", bot_name)
        .replace("{context}", &context_text);
    
    // Call Gemini API with the news prompt
    match gemini_client.generate_response_with_context(&news_prompt, "", &Vec::new(), None).await {
        Ok(response) => {
            // Check if the response is "pass" - if so, don't send anything
            if response.trim().to_lowercase() == "pass" {
                info!("News interjection evaluation: decided to PASS - no response sent");
                return Ok(());
            }
            
            // Check if the response looks like the prompt itself (API error)
            if response.contains("{bot_name}") || 
               response.contains("{context}") || 
               response.contains("Guidelines:") ||
               response.contains("Example good response:") {
                error!("News interjection error: API returned the prompt instead of a response");
                return Ok(());
            }
            
            // Validate and clean up the response
            let cleaned_response = clean_news_response(&response);
            
            // If the cleaning process resulted in an empty response, don't send anything
            if cleaned_response.is_empty() {
                info!("News interjection skipped: URL validation failed");
                return Ok(());
            }
            
            // Extract the URL for validation
            let url_regex = Regex::new(r"https?://[^\s]+").unwrap();
            if let Some(url_match) = url_regex.find(&cleaned_response) {
                let url_str = url_match.as_str();
                
                // Validate that the URL actually exists
                match validate_url_exists(url_str).await {
                    Ok(true) => {
                        // URL exists, proceed with sending the message
                        info!("URL validation successful: {} exists", url_str);
                        
                        // Start typing indicator now that we've decided to send a message
                        if let Err(e) = msg.channel_id.broadcast_typing(&ctx.http).await {
                            error!("Failed to send typing indicator for news interjection: {:?}", e);
                        }
                        
                        // Apply realistic typing delay
                        apply_realistic_delay(&cleaned_response, ctx, msg.channel_id).await;
                        
                        // Send the response
                        let response_text = cleaned_response.clone(); // Clone for logging
                        if let Err(e) = msg.channel_id.say(&ctx.http, cleaned_response).await {
                            error!("Error sending news interjection: {:?}", e);
                        } else {
                            info!("News interjection evaluation: SENT response - {}", response_text);
                        }
                    },
                    Ok(false) => {
                        // URL doesn't exist
                        info!("News interjection skipped: URL doesn't exist: {}", url_str);
                    },
                    Err(e) => {
                        // Error validating URL
                        error!("Error validating URL {}: {:?}", url_str, e);
                    }
                }
            } else {
                info!("News interjection skipped: No URL found in cleaned response");
            }
        },
        Err(e) => {
            error!("Error generating news interjection: {:?}", e);
        }
    }
    
    Ok(())
}

// Function to validate and clean up news responses
fn clean_news_response(response: &str) -> String {
    // Extract the URL from the response
    let url_regex = Regex::new(r"https?://[^\s]+").unwrap();
    
    if let Some(url_match) = url_regex.find(response) {
        let url_str = url_match.as_str();
        
        // Try to parse the URL
        if let Ok(url) = Url::parse(url_str) {
            // Check if the URL has a proper path (not just "/")
            let path = url.path();
            if path.len() <= 1 {
                // URL doesn't have a proper path
                info!("News interjection URL validation failed: URL has no proper path: {}", url_str);
                return String::new();
            }
            
            // Check if the URL contains a year in the path (common for news articles)
            let has_year = path.contains("/20");
            let has_month = path.contains("/01/") || path.contains("/02/") || path.contains("/03/") ||
                           path.contains("/04/") || path.contains("/05/") || path.contains("/06/") ||
                           path.contains("/07/") || path.contains("/08/") || path.contains("/09/") ||
                           path.contains("/10/") || path.contains("/11/") || path.contains("/12/");
            
            if !has_year && !has_month {
                // URL doesn't look like a news article
                info!("News interjection URL validation failed: URL doesn't look like a news article: {}", url_str);
                return String::new();
            }
            
            // Remove any "(via search)" or similar tags using regex for more flexibility
            let via_regex = Regex::new(r"\s*\(via\s+[^)]+\)\s*").unwrap();
            let cleaned_response = via_regex.replace_all(response, "").to_string();
            
            return cleaned_response.trim().to_string();
        } else {
            // Invalid URL
            info!("News interjection URL validation failed: Invalid URL: {}", url_str);
            return String::new();
        }
    } else {
        // No URL found
        info!("News interjection URL validation failed: No URL found in response");
        return String::new();
    }
}

// Function to validate if a URL actually exists by making a HEAD request
pub async fn validate_url_exists(url: &str) -> Result<bool> {
    info!("Validating URL exists: {}", url);
    
    // Create a client with a short timeout
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36")
        .build()?;
    
    // Try a HEAD request first (faster)
    match client.head(url).send().await {
        Ok(response) => {
            let status = response.status();
            if status.is_success() || status.is_redirection() {
                info!("URL validation successful (HEAD): {} - Status: {}", url, status);
                return Ok(true);
            } else if status.as_u16() == 405 {
                // Some servers don't support HEAD, try GET
                match client.get(url).send().await {
                    Ok(get_response) => {
                        let get_status = get_response.status();
                        info!("URL validation with GET: {} - Status: {}", url, get_status);
                        return Ok(get_status.is_success() || get_status.is_redirection());
                    },
                    Err(e) => {
                        info!("URL validation failed (GET): {} - Error: {}", url, e);
                        return Ok(false);
                    }
                }
            } else {
                info!("URL validation failed (HEAD): {} - Status: {}", url, status);
                return Ok(false);
            }
        },
        Err(e) => {
            // If there's a timeout or connection error, the URL likely doesn't exist
            info!("URL validation failed (HEAD): {} - Error: {}", url, e);
            
            // Try GET as a fallback
            match client.get(url).send().await {
                Ok(get_response) => {
                    let get_status = get_response.status();
                    info!("URL validation with GET: {} - Status: {}", url, get_status);
                    return Ok(get_status.is_success() || get_status.is_redirection());
                },
                Err(e) => {
                    info!("URL validation failed (GET): {} - Error: {}", url, e);
                    return Ok(false);
                }
            }
        }
    }
}
