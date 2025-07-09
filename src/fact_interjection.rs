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
use crate::google_search::GoogleSearchClient;
use crate::url_validator;

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

// Function to find a better URL using search when the original URL fails validation
async fn find_better_url(fact: &str) -> Result<Option<String>> {
    info!("Attempting to find a better URL for fact: {}", fact);
    
    // Create a search client
    let search_client = GoogleSearchClient::new();
    
    // Extract the main fact without the citation
    let main_fact = if let Some(citation_index) = fact.find("Source:") {
        fact[..citation_index].trim()
    } else {
        fact.trim()
    };
    
    // Perform a search using the fact text
    match search_client.search(main_fact).await {
        Ok(Some(result)) => {
            info!("Found potential replacement URL: {} - {}", result.title, result.url);
            
            // Validate the new URL
            match news_interjection::validate_url_exists(&result.url).await {
                Ok((true, Some(final_url))) => {
                    info!("Replacement URL validation successful: {}", final_url);
                    Ok(Some(final_url))
                },
                _ => {
                    info!("Replacement URL validation failed: {}", result.url);
                    Ok(None)
                }
            }
        },
        Ok(None) => {
            info!("No search results found for fact");
            Ok(None)
        },
        Err(e) => {
            error!("Error searching for better URL: {:?}", e);
            Ok(None)
        }
    }
}

// Function to validate a citation URL with search fallback
async fn validate_citation_with_fallback(
    _gemini_client: &GeminiClient,
    fact: &str,
    citation: &str,
) -> Result<(bool, Option<String>)> {
    // First try the original URL
    match news_interjection::validate_url_exists(citation).await {
        Ok((true, final_url)) => {
            // URL exists and is valid
            info!("Citation URL validation successful: {}", citation);
            Ok((true, final_url))
        },
        Ok((false, _)) => {
            // URL doesn't exist or isn't HTML, try to find a better one
            info!("Citation URL validation failed: {}. Attempting to find a better URL...", citation);
            
            match find_better_url(fact).await {
                Ok(Some(better_url)) => {
                    info!("Found better URL: {}", better_url);
                    Ok((true, Some(better_url)))
                },
                _ => {
                    info!("Could not find a better URL for fact");
                    Ok((false, None))
                }
            }
        },
        Err(e) => {
            // Error validating URL
            error!("Error validating citation URL {}: {:?}", citation, e);
            // Try to find a better URL as fallback
            match find_better_url(fact).await {
                Ok(Some(better_url)) => {
                    info!("Found better URL after error: {}", better_url);
                    Ok((true, Some(better_url)))
                },
                _ => {
                    // Default to accepting the citation if validation fails due to technical issues
                    // and we couldn't find a better URL
                    info!("Technical error validating URL and could not find a better URL");
                    Ok((true, None))
                }
            }
        }
    }
}

// Function to validate a citation URL
async fn validate_citation_with_ai(
    gemini_client: &GeminiClient,
    fact: &str,
    citation: &str,
) -> bool {
    // Validate the citation with fallback to search
    match validate_citation_with_fallback(gemini_client, fact, citation).await {
        Ok((is_valid, _)) => is_valid,
        Err(e) => {
            error!("Error in citation validation with fallback: {:?}", e);
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
            
            // Check for self-reference issues
            if response.contains("I'm Crow") || 
               response.contains("As Crow") || 
               response.contains("handsome") && response.contains("modest") ||
               response.contains("Satellite of Love") {
                error!("Fact interjection error: Response contains self-reference: {}", response);
                return Ok(());
            }
            
            // Validate URL using our new validator
            if !url_validator::validate_url(&response) {
                error!("Fact interjection error: Invalid URL in response: {}", response);
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
                
                // If we found a better URL through validation, replace it in the response
                match validate_citation_with_fallback(gemini_client, &response, &citation).await {
                    Ok((true, Some(better_url))) if better_url != citation => {
                        info!("Replacing citation URL in response: {} -> {}", citation, better_url);
                        let response = response.replace(&citation, &better_url);
                        
                        // Start typing indicator
                        if let Err(e) = channel_id.broadcast_typing(http).await {
                            error!("Failed to send typing indicator for fact interjection: {:?}", e);
                        }
                        
                        // Apply realistic typing delay based on response length
                        let words = response.split_whitespace().count();
                        let delay_secs = (words as f32 * 0.2).max(2.0).min(5.0) as u64;
                        tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
                        
                        // Send the response with the updated URL
                        let response_text = response.clone(); // Clone for logging
                        if let Err(e) = channel_id.say(http, response).await {
                            error!("Error sending fact interjection: {:?}", e);
                        } else {
                            info!("Fact interjection evaluation: SENT response with updated URL - {}", response_text);
                        }
                        
                        return Ok(());
                    },
                    _ => {}
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
