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

// Function to validate if a fact has a proper citation
fn has_valid_citation(fact: &str) -> bool {
    // List of known reputable sources
    let reputable_sources = [
        // Scientific organizations
        "nasa", "who", "cdc", "nih", "noaa", "epa", "usgs", "fda", "nsf", "doe", "esa", "cern",
        // Academic institutions
        "university", "harvard", "stanford", "mit", "oxford", "cambridge", "yale", "princeton", "caltech", "berkeley",
        // Scientific journals
        "nature", "science", "cell", "lancet", "nejm", "pnas", "jama", "bmj", "plos", "journal of",
        // News organizations
        "bbc", "reuters", "associated press", "ap ", "npr", "pbs", "smithsonian", "national geographic",
        // Technology publications
        "wired", "ars technica", "ieee", "acm", "mit technology review",
        // Museums and educational institutions
        "museum", "institute", "foundation", "society", "association",
        // Government agencies
        "gov", "department of", "bureau of", "administration", "agency",
    ];
    
    // Check for citation patterns
    let citation_patterns = [
        // Source attribution with reputable source
        Regex::new(r"(?i)\b(?:according to|source:?|from|cited (?:in|by)|as reported (?:in|by))\s+([^\.]+)").unwrap(),
        // Year in parentheses with author, typical of academic citations
        Regex::new(r"(?i)(?:[A-Z][a-z]+ (?:et al\.|and [A-Z][a-z]+))? \([12][0-9]{3}\)").unwrap(),
        // URL reference to reputable domain
        Regex::new(r"(?i)(?:https?://)?(?:www\.)?([a-z0-9][-a-z0-9]*\.[a-z]{2,})(?:/[^\s]*)?").unwrap(),
        // Publication with date
        Regex::new(r"(?i)(?:published|reported|released) (?:in|by) ([^,\.]+) in [12][0-9]{3}").unwrap(),
        // Research study reference
        Regex::new(r"(?i)(?:a|the) (?:study|research|survey|analysis) (?:by|from|conducted by) ([^\.]+)").unwrap(),
    ];
    
    // Check if any citation pattern matches and contains a reputable source
    for pattern in &citation_patterns {
        if let Some(captures) = pattern.captures(fact) {
            if captures.len() > 1 {
                if let Some(source_match) = captures.get(1) {
                    let source_text = source_match.as_str().to_lowercase();
                    // Check if the source contains any reputable source keywords
                    for reputable in &reputable_sources {
                        if source_text.contains(reputable) {
                            return true;
                        }
                    }
                }
            }
        }
    }
    
    // Special case for direct mentions of reputable sources without formal citation structure
    for source in &reputable_sources {
        let source_pattern = format!(r"(?i)\b{}\b", source);
        if let Ok(regex) = Regex::new(&source_pattern) {
            if regex.is_match(fact) {
                // Check if it's likely a citation and not just a mention
                // Look for patterns that suggest it's being used as a source
                let citation_indicators = [
                    "found", "discovered", "reported", "stated", "says", "said", "published", 
                    "research", "study", "survey", "analysis", "data", "statistics", "according",
                    "suggests", "indicates", "shows", "reveals", "confirms", "estimates"
                ];
                
                for indicator in &citation_indicators {
                    let indicator_pattern = format!(r"(?i)\b{}\b", indicator);
                    if let Ok(indicator_regex) = Regex::new(&indicator_pattern) {
                        if indicator_regex.is_match(fact) {
                            return true;
                        }
                    }
                }
            }
        }
    }
    
    false
}

// Function to extract the citation from a fact
fn extract_citation(fact: &str) -> Option<String> {
    // Common patterns for citation extraction
    let citation_patterns = [
        // Source attribution at the end
        Regex::new(r"(?i)(?:according to|source:?|from|cited (?:in|by)|as reported (?:in|by))\s+([^\.\?!]+)").unwrap(),
        // Parenthetical citation
        Regex::new(r"\(([^)]+)\)").unwrap(),
        // URL citation
        Regex::new(r"(?i)(?:https?://)?(?:www\.)?[a-z0-9][-a-z0-9]*\.[a-z0-9][-a-z0-9]*\.[a-z]{2,}(?:/[^\s]*)?").unwrap(),
        Regex::new(r"(?i)(?:https?://)?(?:www\.)?[a-z0-9][-a-z0-9]*\.[a-z]{2,}(?:/[^\s]*)?").unwrap(),
    ];
    
    // Try to extract citation using patterns
    for pattern in &citation_patterns {
        if let Some(captures) = pattern.captures(fact) {
            if captures.len() > 0 {
                if let Some(citation_match) = captures.get(1).or_else(|| captures.get(0)) {
                    return Some(citation_match.as_str().trim().to_string());
                }
            }
        }
    }
    
    None
}

// Function to validate a citation with a second API call
async fn validate_citation_with_ai(
    gemini_client: &GeminiClient,
    fact: &str,
    citation: &str,
) -> bool {
    // Create a prompt to validate the citation
    let validation_prompt = format!(
        r#"You are a fact-checking assistant. Please evaluate if the following citation appears to be legitimate and appropriate for the fact it supports.

Fact with citation: "{}"

Extracted citation: "{}"

Please analyze:
1. Is this a real, legitimate source (e.g., reputable publication, academic institution, government agency)?
2. Is the citation specific enough to be credible (not just a vague reference)?
3. Is the citation appropriate and relevant for the stated fact?

Respond with ONLY the single word "VALID" (no punctuation) if the citation meets all criteria, or "INVALID" if it fails any criterion. Do not include any explanation or additional text."#,
        fact, citation
    );
    
    // Call Gemini API to validate the citation
    match gemini_client.generate_response_with_context(&validation_prompt, "", &Vec::new(), None).await {
        Ok(response) => {
            let trimmed_response = response.trim().to_uppercase();
            // Remove any punctuation and check if it starts with VALID
            let cleaned_response = trimmed_response.trim_end_matches(|c: char| !c.is_alphanumeric());
            if cleaned_response == "VALID" || cleaned_response.starts_with("VALID ") {
                info!("Citation validation: VALID - {}", citation);
                true
            } else {
                info!("Citation validation: INVALID - {}", citation);
                false
            }
        },
        Err(e) => {
            error!("Error validating citation: {:?}", e);
            // Default to accepting the citation if validation fails
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
    bot_name: &str,
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
    
    // Create the fact prompt
    let fact_prompt = String::from(r#"You are {bot_name}, a Discord bot. Share an interesting and factually accurate fact related to the conversation.

{context}

Guidelines:
1. Share a single, concise, factually accurate fact that is relevant to the recent conversation
2. The fact MUST be true and verifiable - this is extremely important
3. Start with "Fun fact:" or "Did you know?"
4. Keep it brief (1-2 sentences)
5. Make it interesting and educational
6. If possible, relate it to the conversation topic, but don't force it
7. If you can't find a relevant fact based on the conversation, share a general interesting fact about technology, science, history, or nature
8. ALWAYS include a specific, verifiable citation or source for your fact (e.g., "According to NASA's 2023 report...", "Source: National Geographic (2022)", etc.)
9. The citation MUST be from a reputable source (scientific organization, academic institution, respected publication)
10. If you can't provide a verifiable citation from a reputable source for your fact, respond with ONLY the word "pass" - nothing else
11. If you include a reference to MST3K, it should be a direct quote that fits naturally in context (like "Watch out for snakes!"), not a forced reference (like "Even Tom Servo would find that interesting!")

Example good response: "Fun fact: The average cloud weighs around 1.1 million pounds due to the weight of water droplets. (Source: USGS Water Science School, 2019)"

Example bad response: "I noticed you were talking about weather. Here's an interesting fact: clouds are actually quite heavy! The average cloud weighs around 1.1 million pounds due to the weight of water droplets. Isn't that fascinating?"

Be concise and factual, and always include a specific, verifiable citation from a reputable source."#)
        .replace("{bot_name}", bot_name)
        .replace("{context}", &context_text);
    
    // Call Gemini API with the fact prompt
    match gemini_client.generate_response_with_context(&fact_prompt, "", &Vec::new(), None).await {
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
