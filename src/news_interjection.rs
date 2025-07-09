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
use std::collections::HashSet;
use crate::url_validator;

// List of trusted news domains
fn get_trusted_domains() -> HashSet<&'static str> {
    let domains = [
        // Tech news
        "techcrunch.com", "arstechnica.com", "wired.com", "theverge.com", "engadget.com",
        "cnet.com", "zdnet.com", "venturebeat.com", "thenextweb.com", "gizmodo.com",
        "mashable.com", "slashdot.org", "tomshardware.com", "anandtech.com", "macrumors.com",
        "9to5mac.com", "9to5google.com", "androidpolice.com", "xda-developers.com",
        
        // Science news
        "scientificamerican.com", "sciencedaily.com", "livescience.com", "popsci.com",
        "newscientist.com", "sciencemag.org", "nature.com", "space.com", "phys.org",
        
        // General news
        "reuters.com", "apnews.com", "bbc.com", "bbc.co.uk", "npr.org", "washingtonpost.com",
        "nytimes.com", "theguardian.com", "economist.com", "bloomberg.com", "cnbc.com",
        "wsj.com", "ft.com", "time.com", "theatlantic.com", "vox.com", "slate.com",
        
        // Weird news
        "boingboing.net", "digg.com", "mentalfloss.com", "atlasobscura.com", "odditycentral.com",
        "neatorama.com", "unusualplaces.org", "oddee.com", "weirdnews.com", "theawesomer.com",
        
        // Government and educational
        "nasa.gov", "nih.gov", "cdc.gov", "noaa.gov", "epa.gov", "energy.gov", "nsf.gov",
        "edu", "ac.uk", "mit.edu", "harvard.edu", "stanford.edu", "berkeley.edu", "caltech.edu",
        
        // Major platforms with news content
        "medium.com", "substack.com", "github.blog", "stackoverflow.blog", "youtube.com",
        "reddit.com", "wikipedia.org", "wikimedia.org"
    ];
    
    domains.iter().copied().collect()
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
    
    // Create the news prompt using the prompt templates
    let news_prompt = gemini_client.prompt_templates().format_news_interjection(&context_text);
    
    // Call Gemini API with the news prompt
    match gemini_client.generate_response_with_context(&news_prompt, "", &Vec::new(), None).await {
        Ok(response) => {
            // Check if the response starts with "pass" (case-insensitive) - if so, don't send anything
            if response.trim().to_lowercase().starts_with("pass") {
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
            
            // Check for self-reference issues
            if response.contains("I'm Crow") || 
               response.contains("As Crow") || 
               response.contains("handsome") && response.contains("modest") ||
               response.contains("Satellite of Love") {
                error!("News interjection error: Response contains self-reference: {}", response);
                return Ok(());
            }
            
            // Validate URL using our new validator
            if !url_validator::validate_url(&response) {
                error!("News interjection error: Invalid URL in response: {}", response);
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
                
                // Validate that the URL actually exists and follow redirects
                match validate_url_exists(url_str).await {
                    Ok((true, Some(final_url))) => {
                        // URL exists, proceed with sending the message
                        info!("URL validation successful: {} exists", final_url);
                        
                        // Replace the original URL with the final URL if they're different
                        let final_response = if url_str != final_url {
                            cleaned_response.replace(url_str, &final_url)
                        } else {
                            cleaned_response
                        };
                        
                        // Start typing indicator now that we've decided to send a message
                        if let Err(e) = msg.channel_id.broadcast_typing(&ctx.http).await {
                            error!("Failed to send typing indicator for news interjection: {:?}", e);
                        }
                        
                        // Apply realistic typing delay
                        apply_realistic_delay(&final_response, ctx, msg.channel_id).await;
                        
                        // Send the response
                        let response_text = final_response.clone(); // Clone for logging
                        if let Err(e) = msg.channel_id.say(&ctx.http, final_response).await {
                            error!("Error sending news interjection: {:?}", e);
                        } else {
                            info!("News interjection evaluation: SENT response - {}", response_text);
                        }
                    },
                    Ok((true, None)) => {
                        // URL exists but we couldn't get the final URL
                        info!("News interjection skipped: URL exists but couldn't get final URL: {}", url_str);
                    },
                    Ok((false, _)) => {
                        // URL doesn't exist or isn't HTML
                        info!("News interjection skipped: URL doesn't exist or isn't HTML: {}", url_str);
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
            
            // Check if the domain is in our trusted list
            let host = url.host_str().unwrap_or("");
            let trusted_domains = get_trusted_domains();
            let domain_parts: Vec<&str> = host.split('.').collect();
            
            // Check if the domain or any parent domain is trusted
            let mut is_trusted = false;
            if domain_parts.len() >= 2 {
                let base_domain = format!("{}.{}", domain_parts[domain_parts.len() - 2], domain_parts[domain_parts.len() - 1]);
                is_trusted = trusted_domains.contains(base_domain.as_str());
                
                // Also check for subdomains of trusted domains
                for trusted in trusted_domains.iter() {
                    if host.ends_with(trusted) {
                        is_trusted = true;
                        break;
                    }
                }
            }
            
            if !is_trusted {
                info!("News interjection URL validation failed: Domain not in trusted list: {}", host);
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
            let content_type = response.headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            
            let is_html = content_type.contains("text/html") || 
                          content_type.contains("application/xhtml+xml");
            
            if !is_html {
                info!("URL validation failed: Content type is not HTML: {}", content_type);
                return Ok((false, None));
            }
            
            if status.is_success() {
                info!("URL validation successful: {} - Status: {}", final_url, status);
                return Ok((true, Some(final_url)));
            } else {
                info!("URL validation failed: {} - Status: {}", final_url, status);
                return Ok((false, None));
            }
        },
        Err(e) => {
            info!("URL validation failed: {} - Error: {}", url, e);
            return Ok((false, None));
        }
    }
}
