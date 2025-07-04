use anyhow::Result;
use serenity::model::channel::Message;
use serenity::prelude::*;
use tracing::{error, info};
use crate::db_utils;
use crate::gemini_api::GeminiClient;
use std::sync::Arc;
use tokio_rusqlite::Connection;
use serenity::model::id::ChannelId;
use serenity::http::Http;
use regex::Regex;
use crate::news_interjection;

// Handle fact interjection with Message object
pub async fn handle_fact_interjection(
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
                error!("Error retrieving recent messages for fact interjection: {:?}", e);
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    
    // Call the common implementation
    handle_fact_interjection_common(
        &ctx.http,
        msg.channel_id,
        gemini_client,
        &context_messages,
        bot_name,
    ).await
}

// Handle fact interjection for spontaneous interjections (without Message object)
pub async fn handle_spontaneous_fact_interjection(
    http: &Http,
    channel_id: ChannelId,
    gemini_client: &GeminiClient,
    message_db: &Option<Arc<tokio::sync::Mutex<Connection>>>,
    bot_name: &str,
    gemini_context_messages: usize,
) -> Result<()> {
    // Get recent messages for context
    let context_messages = if let Some(db) = message_db {
        match db_utils::get_recent_messages(db.clone(), gemini_context_messages, Some(&channel_id.to_string())).await {
            Ok(messages) => messages,
            Err(e) => {
                error!("Error retrieving recent messages for spontaneous fact interjection: {:?}", e);
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    
    // Call the common implementation
    handle_fact_interjection_common(
        http,
        channel_id,
        gemini_client,
        &context_messages,
        bot_name,
    ).await
}

// Function to validate if a fact has a proper citation with a URL
fn has_valid_citation(fact: &str) -> bool {
    // URL regex pattern
    let url_regex = Regex::new(r"https?://[^\s]+").unwrap();
    
    // Check if the fact contains a URL
    if url_regex.is_match(fact) {
        return true;
    }
    
    // No URL found
    false
}

// Function to extract the URL citation from a fact
fn extract_citation(fact: &str) -> Option<String> {
    // URL regex pattern
    let url_regex = Regex::new(r"https?://[^\s]+").unwrap();
    
    // Find the URL in the fact
    if let Some(url_match) = url_regex.find(fact) {
        return Some(url_match.as_str().trim().to_string());
    }
    
    None
}

// Function to validate a citation URL
async fn validate_citation_with_ai(
    _gemini_client: &GeminiClient,
    _fact: &str,
    citation: &str,
) -> bool {
    // Validate that the URL actually exists and is accessible
    match news_interjection::validate_url_exists(citation).await {
        Ok((true, _)) => {
            // URL exists and is valid
            info!("Citation URL validation successful: {}", citation);
            true
        },
        Ok((false, _)) => {
            // URL doesn't exist or isn't HTML
            info!("Citation URL validation failed: URL doesn't exist or isn't HTML: {}", citation);
            false
        },
        Err(e) => {
            // Error validating URL
            error!("Error validating citation URL {}: {:?}", citation, e);
            // Default to accepting the citation if validation fails due to technical issues
            true
        }
    }
}

// Common implementation for both regular and spontaneous fact interjections
async fn handle_fact_interjection_common(
    http: &Http,
    channel_id: ChannelId,
    gemini_client: &GeminiClient,
    context_messages: &Vec<(String, String, String)>,
    _bot_name: &str,
) -> Result<()> {
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
        info!("No context available for fact interjection in channel_id: {}", channel_id);
        // Use empty string instead of "No recent messages" to avoid showing this in logs
        "".to_string()
    };
    
    // Create the fact prompt using the prompt templates
    let fact_prompt = gemini_client.prompt_templates().format_fact_interjection(&context_text);
    
    // Call Gemini API with the fact prompt
    match gemini_client.generate_response_with_context(&fact_prompt, "", &context_messages, None).await {
        Ok(response) => {
            // Check if the response starts with "pass" (case-insensitive) - if so, don't send anything
            if response.trim().to_lowercase().starts_with("pass") {
                info!("Fact interjection evaluation: decided to PASS - no response sent");
                return Ok(());
            }
            
            // Check if the response looks like the prompt itself (API error)
            if response.contains("{bot_name}") || 
               response.contains("{context}") || 
               response.contains("Guidelines:") ||
               response.contains("Example good response:") {
                error!("Fact interjection error: API returned the prompt instead of a response");
                return Ok(());
            }
            
            // First check if the fact has a citation pattern
            if !has_valid_citation(&response) {
                info!("Fact interjection rejected: No valid citation found in: {}", response);
                return Ok(());
            }
            
            // Extract the citation for validation
            if let Some(citation) = extract_citation(&response) {
                // Validate the citation with a second API call
                if !validate_citation_with_ai(gemini_client, &response, &citation).await {
                    info!("Fact interjection rejected: Citation validation failed for: {}", citation);
                    return Ok(());
                }
            }
            
            // Start typing indicator
            if let Err(e) = channel_id.broadcast_typing(http).await {
                error!("Failed to send typing indicator for fact interjection: {:?}", e);
            }
            
            // Apply realistic typing delay based on response length
            let words = response.split_whitespace().count();
            let delay_secs = (words as f32 * 0.2).max(2.0).min(5.0) as u64;
            tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
            
            // Send the response
            let response_text = response.clone(); // Clone for logging
            if let Err(e) = channel_id.say(http, response).await {
                error!("Error sending fact interjection: {:?}", e);
            } else {
                info!("Fact interjection evaluation: SENT response - {}", response_text);
            }
        },
        Err(e) => {
            error!("Error generating fact interjection: {:?}", e);
        }
    }
    
    Ok(())
}
