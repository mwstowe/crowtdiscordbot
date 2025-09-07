use crate::db_utils;
use crate::duckduckgo_search::DuckDuckGoSearchClient;
use crate::gemini_api::GeminiClient;
use crate::news_interjection;
use crate::url_validator;
use anyhow::Result;
use regex::Regex;
use serenity::http::Http;
use serenity::model::channel::Message;
use serenity::model::id::ChannelId;
use serenity::prelude::*;
use std::sync::Arc;
use tokio_rusqlite::Connection;
use tracing::{error, info};

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
        match db_utils::get_recent_messages_with_pronouns(
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

    // Call the common implementation
    handle_fact_interjection_common(
        &ctx.http,
        msg.channel_id,
        gemini_client,
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
    message_db: &Option<Arc<tokio::sync::Mutex<Connection>>>,
    bot_name: &str,
    gemini_context_messages: usize,
) -> Result<()> {
    // Get recent messages for context
    let context_messages = if let Some(db) = message_db {
        match db_utils::get_recent_messages_with_pronouns(
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

    // Call the common implementation
    handle_fact_interjection_common(http, channel_id, gemini_client, &context_messages, bot_name)
        .await
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
    let search_client = DuckDuckGoSearchClient::new();

    // Extract the main fact without the citation
    let main_fact = if let Some(citation_index) = fact.find("Source:") {
        fact[..citation_index].trim()
    } else {
        fact.trim()
    };

    // Perform a search using the fact text
    match search_client.search(main_fact).await {
        Ok(Some(result)) => {
            info!(
                "Found potential replacement URL: {} - {}",
                result.title, result.url
            );

            // Validate the new URL
            match news_interjection::validate_url_exists(&result.url).await {
                Ok((true, Some(final_url))) => {
                    info!("Replacement URL validation successful: {}", final_url);
                    Ok(Some(final_url))
                }
                _ => {
                    info!("Replacement URL validation failed: {}", result.url);
                    Ok(None)
                }
            }
        }
        Ok(None) => {
            info!("No search results found for fact");
            Ok(None)
        }
        Err(e) => {
            error!("Error searching for better URL: {:?}", e);
            Ok(None)
        }
    }
}

// Function to validate that a fact and its citation actually match using Gemini API
async fn validate_fact_matches_citation(
    gemini_client: &GeminiClient,
    fact: &str,
    citation: &str,
) -> Result<bool> {
    info!(
        "Validating that fact matches citation: {} - {}",
        fact, citation
    );

    // Extract the main fact without the citation
    let main_fact = if let Some(citation_index) = fact.find("Source:") {
        fact[..citation_index].trim()
    } else {
        fact.trim()
    };

    // Create a prompt to check if the fact and citation match
    let validation_prompt = format!(
        "You are a fact-checking assistant. Your task is to determine if a given fact is supported by the provided URL citation.\n\n\
        Fact: \"{main_fact}\"\n\
        Citation URL: {citation}\n\n\
        Please verify if this fact is likely to be found at or supported by the content at this URL.\n\
        Consider the domain expertise of the website, the specificity of the fact, and whether the URL path suggests relevant content.\n\n\
        Respond with ONLY ONE of these exact options:\n\
        1. \"MATCH\" - if the fact is likely supported by the URL\n\
        2. \"MISMATCH\" - if the fact is clearly not related to the URL or contradicts what would be found there\n\
        3. \"UNCERTAIN\" - if you cannot determine with confidence\n\n\
        Respond with ONLY one of these three words and nothing else."
    );

    // Call Gemini API to validate
    match gemini_client.generate_content(&validation_prompt).await {
        Ok(response) => {
            let response = response.trim().to_uppercase();
            info!("Fact-citation validation result: {}", response);

            if response == "MATCH" {
                Ok(true)
            } else if response == "MISMATCH" {
                Ok(false)
            } else {
                // For UNCERTAIN or any other response, we'll be conservative and reject
                info!("Uncertain fact-citation match, rejecting to be safe");
                Ok(false)
            }
        }
        Err(e) => {
            error!("Error validating fact-citation match: {:?}", e);
            // Default to rejecting if we can't validate
            Ok(false)
        }
    }
}

// Function to validate a citation URL
async fn validate_citation_with_ai(
    gemini_client: &GeminiClient,
    fact: &str,
    citation: &str,
) -> bool {
    // First check if the URL exists
    match news_interjection::validate_url_exists(citation).await {
        Ok((true, _)) => {
            // URL exists, now check if the fact and citation match
            match validate_fact_matches_citation(gemini_client, fact, citation).await {
                Ok(matches) => {
                    if matches {
                        info!("Fact validation successful: fact matches citation content");
                        true
                    } else {
                        info!("Fact validation failed: fact does NOT match citation content");
                        false
                    }
                }
                Err(e) => {
                    error!("Error validating fact-citation match: {:?}", e);
                    // Be conservative and reject if we can't validate
                    false
                }
            }
        }
        Ok((false, _)) => {
            info!("Citation URL validation failed: URL doesn't exist or isn't HTML");
            false
        }
        Err(e) => {
            error!("Error validating citation URL: {:?}", e);
            // Be conservative and reject if we can't validate
            false
        }
    }
}

// Common implementation for both regular and spontaneous fact interjections
async fn handle_fact_interjection_common(
    http: &Http,
    channel_id: ChannelId,
    gemini_client: &GeminiClient,
    context_messages: &[(String, String, Option<String>, String)],
    _bot_name: &str,
) -> Result<()> {
    // Format context for the prompt
    let context_text = if !context_messages.is_empty() {
        // Reverse the messages to get chronological order (oldest first)
        let mut chronological_messages = context_messages.to_owned();
        chronological_messages.reverse();

        let formatted_messages: Vec<String> = chronological_messages
            .iter()
            .map(|(_author, display_name, _pronouns, content)| format!("{display_name}: {content}"))
            .collect();
        formatted_messages.join("\n")
    } else {
        info!(
            "No context available for fact interjection in channel_id: {}",
            channel_id
        );
        // Use empty string instead of "No recent messages" to avoid showing this in logs
        "".to_string()
    };

    // Create the fact prompt using the prompt templates
    let fact_prompt = gemini_client
        .prompt_templates()
        .format_fact_interjection(&context_text);

    // Call Gemini API with the fact prompt
    match gemini_client
        .generate_response_with_context_and_pronouns(&fact_prompt, "", context_messages, None)
        .await
    {
        Ok(response) => {
            // Check if the response starts with "pass" (case-insensitive) - if so, don't send anything
            if response.trim().to_lowercase().starts_with("pass") {
                info!("Fact interjection evaluation: decided to PASS - no response sent");
                return Ok(());
            }

            // Check if the response looks like the prompt itself (API error)
            if response.contains("{bot_name}")
                || response.contains("{context}")
                || response.contains("Guidelines:")
                || response.contains("Example good response:")
            {
                error!("Fact interjection: API returned prompt template instead of response");
                return Ok(());
            }

            // Validate the fact and reference with a second API call
            let validation_prompt = format!(
                "Validate this fact and reference:\n\nFact: {}\n\nCheck if:\n1. Any URLs mentioned exist and are accessible\n2. The content supports the stated fact\n3. The source appears credible\n\nRespond with only: VALID or INVALID",
                response.trim()
            );

            match gemini_client.generate_content(&validation_prompt).await {
                Ok(validation_response) => {
                    let validation = validation_response.trim().to_uppercase();
                    if !validation.starts_with("VALID") {
                        info!(
                            "Fact interjection validation failed: {} - skipping interjection",
                            validation
                        );
                        return Ok(());
                    }
                    info!("Fact interjection validation passed: {}", validation);
                }
                Err(e) => {
                    error!("Fact interjection validation API call failed: {:?} - skipping interjection", e);
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
                    "Fact interjection error: Response contains self-reference: {}",
                    response
                );
                return Ok(());
            }

            // Validate URL using our new validator
            if !url_validator::validate_url(&response) {
                error!(
                    "Fact interjection error: Invalid URL in response: {}",
                    response
                );
                return Ok(());
            }

            // First check if the fact has a citation pattern
            if !has_valid_citation(&response) {
                info!(
                    "Fact interjection rejected: No valid citation found in: {}",
                    response
                );
                return Ok(());
            }

            // Extract the citation for validation
            if let Some(citation) = extract_citation(&response) {
                // Validate the citation with a second API call
                if !validate_citation_with_ai(gemini_client, &response, &citation).await {
                    info!(
                        "Fact interjection rejected: Citation validation failed for: {}",
                        citation
                    );
                    return Ok(());
                }

                // If we found a better URL through validation, replace it in the response
                match validate_citation_with_fallback(gemini_client, &response, &citation).await {
                    Ok((true, Some(better_url))) if better_url != citation => {
                        info!(
                            "Replacing citation URL in response: {} -> {}",
                            citation, better_url
                        );
                        let response = response.replace(&citation, &better_url);

                        // Start typing indicator
                        if let Err(e) = channel_id.broadcast_typing(http).await {
                            error!(
                                "Failed to send typing indicator for fact interjection: {:?}",
                                e
                            );
                        }

                        // Apply realistic typing delay based on response length
                        let words = response.split_whitespace().count();
                        let delay_secs = (words as f32 * 0.2).clamp(2.0, 5.0) as u64;
                        tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;

                        // Send the response with the updated URL
                        let response_text = response.clone(); // Clone for logging
                        if let Err(e) = channel_id.say(http, response).await {
                            error!("Error sending fact interjection: {:?}", e);
                        } else {
                            info!(
                                "Fact interjection evaluation: SENT response with updated URL - {}",
                                response_text
                            );
                        }

                        return Ok(());
                    }
                    _ => {}
                }
            }

            // Start typing indicator
            if let Err(e) = channel_id.broadcast_typing(http).await {
                error!(
                    "Failed to send typing indicator for fact interjection: {:?}",
                    e
                );
            }

            // Apply realistic typing delay based on response length
            let words = response.split_whitespace().count();
            let delay_secs = (words as f32 * 0.2).clamp(2.0, 5.0) as u64;
            tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;

            // Send the response
            let response_text = response.clone(); // Clone for logging
            if let Err(e) = channel_id.say(http, response).await {
                error!("Error sending fact interjection: {:?}", e);
            } else {
                info!("Fact interjection sent: {}", response_text);
            }
        }
        Err(e) => {
            error!("Error generating fact interjection: {:?}", e);
        }
    }

    Ok(())
}
// Function to validate a citation URL with search fallback
async fn validate_citation_with_fallback(
    gemini_client: &GeminiClient,
    fact: &str,
    citation: &str,
) -> Result<(bool, Option<String>)> {
    // First try the original URL
    match news_interjection::validate_url_exists(citation).await {
        Ok((true, final_url)) => {
            // URL exists and is valid, but now we need to check if the fact matches the citation
            info!("Citation URL exists: {}", citation);

            // Validate that the fact and citation actually match
            match validate_fact_matches_citation(gemini_client, fact, citation).await {
                Ok(true) => {
                    info!("Fact matches citation content: {}", citation);
                    Ok((true, final_url))
                }
                Ok(false) => {
                    info!("Fact does NOT match citation content: {}. Attempting to find a better URL...", citation);
                    // Try to find a better URL
                    match find_better_url(fact).await {
                        Ok(Some(better_url)) => {
                            // Validate the new URL matches the fact
                            match validate_fact_matches_citation(gemini_client, fact, &better_url)
                                .await
                            {
                                Ok(true) => {
                                    info!("Found better matching URL: {}", better_url);
                                    Ok((true, Some(better_url)))
                                }
                                _ => {
                                    info!("Better URL also doesn't match fact content");
                                    Ok((false, None))
                                }
                            }
                        }
                        _ => {
                            info!("Could not find a better URL for fact");
                            Ok((false, None))
                        }
                    }
                }
                Err(e) => {
                    error!("Error validating fact-citation match: {:?}", e);
                    // Be conservative and reject if we can't validate
                    Ok((false, None))
                }
            }
        }
        Ok((false, _)) => {
            // URL doesn't exist or isn't HTML, try to find a better one
            info!(
                "Citation URL validation failed: {}. Attempting to find a better URL...",
                citation
            );

            match find_better_url(fact).await {
                Ok(Some(better_url)) => {
                    // Validate the new URL matches the fact
                    match validate_fact_matches_citation(gemini_client, fact, &better_url).await {
                        Ok(true) => {
                            info!("Found better matching URL: {}", better_url);
                            Ok((true, Some(better_url)))
                        }
                        _ => {
                            info!("Better URL doesn't match fact content");
                            Ok((false, None))
                        }
                    }
                }
                _ => {
                    info!("Could not find a better URL for fact");
                    Ok((false, None))
                }
            }
        }
        Err(e) => {
            // Error validating URL
            error!("Error validating citation URL {}: {:?}", citation, e);
            // Try to find a better URL as fallback
            match find_better_url(fact).await {
                Ok(Some(better_url)) => {
                    // Validate the new URL matches the fact
                    match validate_fact_matches_citation(gemini_client, fact, &better_url).await {
                        Ok(true) => {
                            info!("Found better matching URL after error: {}", better_url);
                            Ok((true, Some(better_url)))
                        }
                        _ => {
                            info!("Better URL doesn't match fact content");
                            Ok((false, None))
                        }
                    }
                }
                _ => {
                    // Be conservative and reject if we can't validate
                    info!("Technical error validating URL and could not find a better URL");
                    Ok((false, None))
                }
            }
        }
    }
}
