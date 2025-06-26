use anyhow::Result;
use tracing::{info, error};
use crate::gemini_api::GeminiClient;
use crate::morbotron::{MorbotronClient, MorbotronResult};
use crate::google_search::GoogleSearchClient;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use strsim::jaro_winkler;
use regex::Regex;

// A struct to hold the popularity data for quotes
#[derive(Debug, Deserialize, Serialize)]
struct QuotePopularity {
    quotes: HashMap<String, u32>, // Maps quote text to popularity score
}

// Constants for the Gemini prompt
const GEMINI_MORBOTRON_PROMPT: &str = r#"You are a Futurama quote expert tasked with finding the most relevant quote based on search terms.

Search terms: "{}"

Instructions:
1. Find a Futurama quote that EXPLICITLY contains the search terms when possible
2. If no exact match exists, find quotes that contain synonyms or related concepts
3. Prioritize quotes that include the EXACT search terms in them
4. Consider famous quotes that might relate to these concepts
5. Return your response in this exact JSON format:
   {{
     "quote": "The exact quote text",
     "episode": "Season X Episode Y: Episode Title",
     "character": "Character who said it"
   }}
6. If you can't find a relevant quote, respond with: {{"result": "pass"}}

Examples:
- Search: "bite metal"
- Response: {{"quote": "Bite my shiny metal ass!", "episode": "Season 1 Episode 1: Space Pilot 3000", "character": "Bender"}}

- Search: "good news"
- Response: {{"quote": "Good news, everyone!", "episode": "Various episodes", "character": "Professor Farnsworth"}}

- Search: "blackjack hookers"
- Response: {{"quote": "I'll make my own theme park with blackjack and hookers!", "episode": "Season 1 Episode 2: The Series Has Landed", "character": "Bender"}}

- Search: "death snu"
- Response: {{"quote": "Death by snu-snu!", "episode": "Season 3 Episode 1: Amazon Women in the Mood", "character": "Femputer"}}

- Search: "unknown phrase"
- Response: {{"result": "pass"}}

Remember to return ONLY the JSON with no additional text."#;

pub struct EnhancedMorbotronSearch {
    gemini_client: GeminiClient,
    morbotron_client: MorbotronClient,
    google_client: GoogleSearchClient,
}

impl EnhancedMorbotronSearch {
    pub fn new(gemini_client: GeminiClient, morbotron_client: MorbotronClient, google_client: GoogleSearchClient) -> Self {
        Self {
            gemini_client,
            morbotron_client,
            google_client,
        }
    }
    
    pub async fn search(&self, query: &str) -> Result<Option<MorbotronResult>> {
        info!("Enhanced Morbotron search for: {}", query);
        
        // First, try to get a quote from Gemini API
        info!("Trying Gemini API for quote generation");
        let gemini_prompt = GEMINI_MORBOTRON_PROMPT.replace("{}", query);
        
        match self.gemini_client.generate_content(&gemini_prompt).await {
            Ok(response) => {
                info!("Received Gemini API response: {}", response);
                
                // Skip if Gemini returned "pass"
                if response.trim().to_lowercase().contains("pass") {
                    info!("Gemini API returned 'pass', falling back to direct search");
                    return self.morbotron_client.search(query).await;
                } else {
                    // Clean up the response - remove markdown code blocks and any other formatting
                    let cleaned_response = response
                        .replace("```json", "")
                        .replace("```", "")
                        .trim()
                        .to_string();
                    
                    info!("Cleaned Gemini response: {}", cleaned_response);
                    
                    // Try to parse the response as JSON
                    let json_result = serde_json::from_str::<serde_json::Value>(&cleaned_response);
                    
                    match json_result {
                        Ok(json) => {
                            info!("Successfully parsed Gemini response as JSON");
                            
                            // Extract quote, episode, and character information
                            let quote = json.get("quote").and_then(|q| q.as_str()).unwrap_or("");
                            let episode = json.get("episode").and_then(|e| e.as_str()).unwrap_or("");
                            let character = json.get("character").and_then(|c| c.as_str()).unwrap_or("");
                            
                            info!("Extracted quote: {}", quote);
                            info!("Extracted episode: {}", episode);
                            info!("Extracted character: {}", character);
                            
                            if !quote.is_empty() {
                                // We have a quote from Gemini, now try to find a matching image
                                
                                // First, try a direct site search with the quote
                                if let Some(result) = self.find_image_with_site_search(quote, character, episode).await? {
                                    // Create a custom result with the Gemini quote but the found image
                                    let custom_result = MorbotronResult {
                                        episode: result.episode,
                                        episode_title: result.episode_title,
                                        episode_number: result.episode_number,
                                        season: result.season,
                                        timestamp: result.timestamp,
                                        image_url: result.image_url,
                                        caption: quote.to_string(), // Use the Gemini quote
                                    };
                                    
                                    info!("Created custom result with Gemini quote and found image");
                                    return Ok(Some(custom_result));
                                }
                                
                                // If site search didn't work, try the Morbotron API
                                if let Some(result) = self.find_image_with_morbotron_api(quote, episode).await? {
                                    // Create a custom result with the Gemini quote but the found image
                                    let custom_result = MorbotronResult {
                                        episode: result.episode,
                                        episode_title: result.episode_title,
                                        episode_number: result.episode_number,
                                        season: result.season,
                                        timestamp: result.timestamp,
                                        image_url: result.image_url,
                                        caption: quote.to_string(), // Use the Gemini quote
                                    };
                                    
                                    info!("Created custom result with Gemini quote and found image");
                                    return Ok(Some(custom_result));
                                }
                                
                                // If Morbotron API didn't work, try a general search engine search
                                info!("Trying general search engine for Futurama images");
                                let search_query = format!("futurama {} screenshot", quote);
                                match self.google_client.search(&search_query).await {
                                    Ok(Some(search_result)) => {
                                        info!("Found general search result: {} - {}", search_result.title, search_result.url);
                                        
                                        // Try to extract a Morbotron URL from the search results
                                        if search_result.url.contains("morbotron.com") {
                                            if let Some(frame_id) = extract_frame_id_from_url(&search_result.url) {
                                                info!("Extracted frame ID from URL: {}", frame_id);
                                                
                                                // Construct a direct URL to the frame
                                                let frame_parts: Vec<&str> = frame_id.split('_').collect();
                                                if frame_parts.len() >= 2 {
                                                    let episode_code = frame_parts[0];
                                                    let timestamp = frame_parts[1];
                                                    
                                                    // Create a MorbotronResult directly
                                                    let result = MorbotronResult {
                                                        episode: episode_code.to_string(),
                                                        episode_title: search_result.title.clone(),
                                                        episode_number: 0, // We don't have this info
                                                        season: 0,         // We don't have this info
                                                        timestamp: timestamp.to_string(),
                                                        image_url: format!("https://morbotron.com/img/{}/{}", episode_code, timestamp),
                                                        caption: quote.to_string(),
                                                    };
                                                    
                                                    info!("Created direct result from frame ID via general search");
                                                    return Ok(Some(result));
                                                }
                                            }
                                        }
                                    },
                                    _ => {
                                        info!("No general search results found or error occurred");
                                    }
                                }
                                
                                // If we still don't have an image, create a result with just the quote
                                // and a default image (use a random image from Morbotron)
                                info!("No image found, using random image with Gemini quote");
                                if let Ok(Some(random_result)) = self.morbotron_client.random().await {
                                    let custom_result = MorbotronResult {
                                        episode: random_result.episode,
                                        episode_title: if !episode.is_empty() { episode.to_string() } else { random_result.episode_title },
                                        episode_number: random_result.episode_number,
                                        season: random_result.season,
                                        timestamp: random_result.timestamp,
                                        image_url: random_result.image_url,
                                        caption: quote.to_string(), // Use the Gemini quote
                                    };
                                    
                                    info!("Created custom result with Gemini quote and random image");
                                    return Ok(Some(custom_result));
                                }
                                
                                // If all else fails, try to find an image from the episode using image search
                                if !episode.is_empty() {
                                    info!("Trying image search for episode: {}", episode);
                                    let image_search_query = format!("futurama {} screenshot", episode);
                                    match self.google_client.search(&image_search_query).await {
                                        Ok(Some(search_result)) => {
                                            info!("Found image search result: {} - {}", search_result.title, search_result.url);
                                            
                                            // Try to extract a Morbotron URL from the search results
                                            if search_result.url.contains("morbotron.com") {
                                                if let Some(frame_id) = extract_frame_id_from_url(&search_result.url) {
                                                    info!("Extracted frame ID from URL: {}", frame_id);
                                                    
                                                    // Construct a direct URL to the frame
                                                    let frame_parts: Vec<&str> = frame_id.split('_').collect();
                                                    if frame_parts.len() >= 2 {
                                                        let episode_code = frame_parts[0];
                                                        let timestamp = frame_parts[1];
                                                        
                                                        // Create a MorbotronResult directly
                                                        let result = MorbotronResult {
                                                            episode: episode_code.to_string(),
                                                            episode_title: episode.to_string(),
                                                            episode_number: 0, // We don't have this info
                                                            season: 0,         // We don't have this info
                                                            timestamp: timestamp.to_string(),
                                                            image_url: format!("https://morbotron.com/img/{}/{}", episode_code, timestamp),
                                                            caption: quote.to_string(),
                                                        };
                                                        
                                                        info!("Created result from image search");
                                                        return Ok(Some(result));
                                                    }
                                                }
                                            }
                                        },
                                        _ => {
                                            info!("No image search results found or error occurred");
                                        }
                                    }
                                }
                                
                                // If we still don't have an image, use a random image from Morbotron
                                info!("No image found, using random image with Gemini quote");
                                if let Ok(Some(random_result)) = self.morbotron_client.random().await {
                                    let custom_result = MorbotronResult {
                                        episode: random_result.episode,
                                        episode_title: if !episode.is_empty() { episode.to_string() } else { random_result.episode_title },
                                        episode_number: random_result.episode_number,
                                        season: random_result.season,
                                        timestamp: random_result.timestamp,
                                        image_url: random_result.image_url,
                                        caption: quote.to_string(), // Use the Gemini quote
                                    };
                                    
                                    info!("Created custom result with Gemini quote and random image");
                                    return Ok(Some(custom_result));
                                }
                                
                                // If even that fails, return None
                                info!("Failed to create any result");
                                return Ok(None);
                            }
                        },
                        Err(e) => {
                            // If JSON parsing fails, try to extract the quote directly
                            info!("Failed to parse Gemini response as JSON: {}", e);
                            
                            // Try to extract a quote from the response
                            if let Some(quote) = self.extract_quote_from_text(&response) {
                                info!("Extracted quote from non-JSON response: {}", quote);
                                
                                // Try to find an image for this quote
                                if let Some(result) = self.find_image_with_site_search(&quote, "", "").await? {
                                    // Create a custom result with the extracted quote but the found image
                                    let custom_result = MorbotronResult {
                                        episode: result.episode,
                                        episode_title: result.episode_title,
                                        episode_number: result.episode_number,
                                        season: result.season,
                                        timestamp: result.timestamp,
                                        image_url: result.image_url,
                                        caption: quote.to_string(), // Use the extracted quote
                                    };
                                    
                                    info!("Created custom result with extracted quote and found image");
                                    return Ok(Some(custom_result));
                                }
                                
                                // If site search didn't work, try a general search engine search
                                info!("Trying general search engine for Futurama images");
                                let search_query = format!("futurama {} screenshot", quote);
                                match self.google_client.search(&search_query).await {
                                    Ok(Some(search_result)) => {
                                        info!("Found general search result: {} - {}", search_result.title, search_result.url);
                                        
                                        // Try to extract a Morbotron URL from the search results
                                        if search_result.url.contains("morbotron.com") {
                                            if let Some(frame_id) = extract_frame_id_from_url(&search_result.url) {
                                                info!("Extracted frame ID from URL: {}", frame_id);
                                                
                                                // Construct a direct URL to the frame
                                                let frame_parts: Vec<&str> = frame_id.split('_').collect();
                                                if frame_parts.len() >= 2 {
                                                    let episode_code = frame_parts[0];
                                                    let timestamp = frame_parts[1];
                                                    
                                                    // Create a MorbotronResult directly
                                                    let result = MorbotronResult {
                                                        episode: episode_code.to_string(),
                                                        episode_title: search_result.title.clone(),
                                                        episode_number: 0, // We don't have this info
                                                        season: 0,         // We don't have this info
                                                        timestamp: timestamp.to_string(),
                                                        image_url: format!("https://morbotron.com/img/{}/{}", episode_code, timestamp),
                                                        caption: quote.to_string(),
                                                    };
                                                    
                                                    info!("Created direct result from frame ID via general search");
                                                    return Ok(Some(result));
                                                }
                                            }
                                        }
                                    },
                                    _ => {
                                        info!("No general search results found or error occurred");
                                    }
                                }
                                
                                // If we still don't have an image, try to find an image using general image search
                                info!("Trying general image search for Futurama screenshot");
                                let image_search_query = format!("futurama screenshot");
                                match self.google_client.search(&image_search_query).await {
                                    Ok(Some(search_result)) => {
                                        info!("Found image search result: {} - {}", search_result.title, search_result.url);
                                        
                                        // Try to extract a Morbotron URL from the search results
                                        if search_result.url.contains("morbotron.com") {
                                            if let Some(frame_id) = extract_frame_id_from_url(&search_result.url) {
                                                info!("Extracted frame ID from URL: {}", frame_id);
                                                
                                                // Construct a direct URL to the frame
                                                let frame_parts: Vec<&str> = frame_id.split('_').collect();
                                                if frame_parts.len() >= 2 {
                                                    let episode_code = frame_parts[0];
                                                    let timestamp = frame_parts[1];
                                                    
                                                    // Create a MorbotronResult directly
                                                    let result = MorbotronResult {
                                                        episode: episode_code.to_string(),
                                                        episode_title: search_result.title.clone(),
                                                        episode_number: 0, // We don't have this info
                                                        season: 0,         // We don't have this info
                                                        timestamp: timestamp.to_string(),
                                                        image_url: format!("https://morbotron.com/img/{}/{}", episode_code, timestamp),
                                                        caption: quote.to_string(),
                                                    };
                                                    
                                                    info!("Created result from general image search");
                                                    return Ok(Some(result));
                                                }
                                            }
                                        }
                                    },
                                    _ => {
                                        info!("No general image search results found or error occurred");
                                    }
                                }
                                
                                // If we still don't have an image, use a random image from Morbotron
                                info!("No image found, using random image with extracted quote");
                                if let Ok(Some(random_result)) = self.morbotron_client.random().await {
                                    let custom_result = MorbotronResult {
                                        episode: random_result.episode,
                                        episode_title: random_result.episode_title,
                                        episode_number: random_result.episode_number,
                                        season: random_result.season,
                                        timestamp: random_result.timestamp,
                                        image_url: random_result.image_url,
                                        caption: quote.to_string(), // Use the extracted quote
                                    };
                                    
                                    info!("Created custom result with extracted quote and random image");
                                    return Ok(Some(custom_result));
                                }
                                
                                // If even that fails, return None
                                info!("Failed to create any result");
                                return Ok(None);
                            }
                        }
                    }
                }
            },
            Err(e) => {
                error!("Error with Gemini API: {}", e);
            }
        }
        
        // If all else fails, fall back to the regular search
        info!("No results from enhanced search, falling back to regular search");
        self.morbotron_client.search(query).await
    }
    
    // Helper method to find an image using site search
    async fn find_image_with_site_search(&self, quote: &str, character: &str, episode: &str) -> Result<Option<MorbotronResult>> {
        // First, try a direct site search with the quote
        let site_search_query = format!("site:morbotron.com \"{}\"", quote);
        info!("Trying direct site search with quote: {}", site_search_query);
        
        match self.google_client.search(&site_search_query).await {
            Ok(Some(search_result)) => {
                info!("Found direct site search result: {} - {}", search_result.title, search_result.url);
                
                // Try to extract the frame ID from the URL
                if let Some(frame_id) = extract_frame_id_from_url(&search_result.url) {
                    info!("Extracted frame ID from URL: {}", frame_id);
                    
                    // Construct a direct URL to the frame
                    let frame_parts: Vec<&str> = frame_id.split('_').collect();
                    if frame_parts.len() >= 2 {
                        let episode_code = frame_parts[0];
                        let timestamp = frame_parts[1];
                        
                        // Create a MorbotronResult directly
                        let result = MorbotronResult {
                            episode: episode_code.to_string(),
                            episode_title: search_result.title.clone(),
                            episode_number: 0, // We don't have this info
                            season: 0,         // We don't have this info
                            timestamp: timestamp.to_string(),
                            image_url: format!("https://morbotron.com/img/{}/{}", episode_code, timestamp),
                            caption: quote.to_string(),
                        };
                        
                        info!("Created direct result from frame ID");
                        return Ok(Some(result));
                    }
                }
                
                // If we couldn't extract a frame ID, try to use the URL directly
                if search_result.url.contains("morbotron.com/meme/") {
                    // Extract the episode and timestamp from the URL
                    let url_parts: Vec<&str> = search_result.url.split('/').collect();
                    if url_parts.len() >= 5 {
                        let episode_code = url_parts[4];
                        let timestamp = if url_parts.len() >= 6 { url_parts[5] } else { "0" };
                        
                        // Create a MorbotronResult directly
                        let result = MorbotronResult {
                            episode: episode_code.to_string(),
                            episode_title: search_result.title.clone(),
                            episode_number: 0, // We don't have this info
                            season: 0,         // We don't have this info
                            timestamp: timestamp.to_string(),
                            image_url: format!("https://morbotron.com/img/{}/{}", episode_code, timestamp),
                            caption: quote.to_string(),
                        };
                        
                        info!("Created direct result from URL");
                        return Ok(Some(result));
                    }
                }
            },
            Ok(None) => {
                info!("No direct site search results found");
            },
            Err(e) => {
                error!("Error with direct site search: {}", e);
            }
        }
        
        // Try with character name + quote if character is provided
        if !character.is_empty() {
            let character_quote_search = format!("site:morbotron.com \"{}\" \"{}\"", character, quote);
            info!("Trying site search with character and quote: {}", character_quote_search);
            
            match self.google_client.search(&character_quote_search).await {
                Ok(Some(search_result)) => {
                    info!("Found character+quote site search result: {} - {}", search_result.title, search_result.url);
                    
                    // Try to extract the frame ID from the URL
                    if let Some(frame_id) = extract_frame_id_from_url(&search_result.url) {
                        info!("Extracted frame ID from URL: {}", frame_id);
                        
                        // Construct a direct URL to the frame
                        let frame_parts: Vec<&str> = frame_id.split('_').collect();
                        if frame_parts.len() >= 2 {
                            let episode_code = frame_parts[0];
                            let timestamp = frame_parts[1];
                            
                            // Create a MorbotronResult directly
                            let result = MorbotronResult {
                                episode: episode_code.to_string(),
                                episode_title: search_result.title.clone(),
                                episode_number: 0, // We don't have this info
                                season: 0,         // We don't have this info
                                timestamp: timestamp.to_string(),
                                image_url: format!("https://morbotron.com/img/{}/{}", episode_code, timestamp),
                                caption: quote.to_string(),
                            };
                            
                            info!("Created direct result from frame ID");
                            return Ok(Some(result));
                        }
                    }
                    
                    // If we couldn't extract a frame ID, try to use the URL directly
                    if search_result.url.contains("morbotron.com/meme/") {
                        // Extract the episode and timestamp from the URL
                        let url_parts: Vec<&str> = search_result.url.split('/').collect();
                        if url_parts.len() >= 5 {
                            let episode_code = url_parts[4];
                            let timestamp = if url_parts.len() >= 6 { url_parts[5] } else { "0" };
                            
                            // Create a MorbotronResult directly
                            let result = MorbotronResult {
                                episode: episode_code.to_string(),
                                episode_title: search_result.title.clone(),
                                episode_number: 0, // We don't have this info
                                season: 0,         // We don't have this info
                                timestamp: timestamp.to_string(),
                                image_url: format!("https://morbotron.com/img/{}/{}", episode_code, timestamp),
                                caption: quote.to_string(),
                            };
                            
                            info!("Created direct result from URL");
                            return Ok(Some(result));
                        }
                    }
                },
                _ => {
                    info!("No character+quote site search results found");
                }
            }
        }
        
        // Try with episode information if provided
        if !episode.is_empty() {
            let episode_quote_search = format!("site:morbotron.com \"{}\" \"{}\"", episode, quote);
            info!("Trying site search with episode and quote: {}", episode_quote_search);
            
            match self.google_client.search(&episode_quote_search).await {
                Ok(Some(search_result)) => {
                    info!("Found episode+quote site search result: {} - {}", search_result.title, search_result.url);
                    
                    // Try to extract the frame ID from the URL
                    if let Some(frame_id) = extract_frame_id_from_url(&search_result.url) {
                        info!("Extracted frame ID from URL: {}", frame_id);
                        
                        // Construct a direct URL to the frame
                        let frame_parts: Vec<&str> = frame_id.split('_').collect();
                        if frame_parts.len() >= 2 {
                            let episode_code = frame_parts[0];
                            let timestamp = frame_parts[1];
                            
                            // Create a MorbotronResult directly
                            let result = MorbotronResult {
                                episode: episode_code.to_string(),
                                episode_title: search_result.title.clone(),
                                episode_number: 0, // We don't have this info
                                season: 0,         // We don't have this info
                                timestamp: timestamp.to_string(),
                                image_url: format!("https://morbotron.com/img/{}/{}", episode_code, timestamp),
                                caption: quote.to_string(),
                            };
                            
                            info!("Created direct result from frame ID");
                            return Ok(Some(result));
                        }
                    }
                },
                _ => {
                    info!("No episode+quote site search results found");
                }
            }
        }
        
        // Try with just a few words from the quote
        let words: Vec<&str> = quote.split_whitespace().collect();
        if words.len() >= 3 {
            // Try with the first 3 words
            let first_words = format!("{} {} {}", words[0], words[1], words[2]);
            let first_words_search = format!("site:morbotron.com \"{}\"", first_words);
            info!("Trying site search with first words: {}", first_words_search);
            
            match self.google_client.search(&first_words_search).await {
                Ok(Some(search_result)) => {
                    info!("Found first words site search result: {} - {}", search_result.title, search_result.url);
                    
                    // Try to extract the frame ID from the URL
                    if let Some(frame_id) = extract_frame_id_from_url(&search_result.url) {
                        info!("Extracted frame ID from URL: {}", frame_id);
                        
                        // Construct a direct URL to the frame
                        let frame_parts: Vec<&str> = frame_id.split('_').collect();
                        if frame_parts.len() >= 2 {
                            let episode_code = frame_parts[0];
                            let timestamp = frame_parts[1];
                            
                            // Create a MorbotronResult directly
                            let result = MorbotronResult {
                                episode: episode_code.to_string(),
                                episode_title: search_result.title.clone(),
                                episode_number: 0, // We don't have this info
                                season: 0,         // We don't have this info
                                timestamp: timestamp.to_string(),
                                image_url: format!("https://morbotron.com/img/{}/{}", episode_code, timestamp),
                                caption: quote.to_string(),
                            };
                            
                            info!("Created direct result from frame ID");
                            return Ok(Some(result));
                        }
                    }
                },
                _ => {
                    info!("No first words site search results found");
                }
            }
        }
        
        Ok(None)
    }
    
    // Helper method to find an image using the Morbotron API
    async fn find_image_with_morbotron_api(&self, quote: &str, episode: &str) -> Result<Option<MorbotronResult>> {
        // Try with the full quote - just one API call
        info!("Trying search with full quote: {}", quote);
        match self.morbotron_client.search(quote).await {
            Ok(Some(result)) => {
                info!("Found result with full quote search");
                return Ok(Some(result));
            },
            _ => {
                info!("No results found for full quote search");
            }
        }
        
        // If episode information is available, try that as a fallback
        if !episode.is_empty() {
            // Try to extract season and episode numbers from the episode string
            if let Some(season_episode) = extract_season_episode_from_text(episode) {
                info!("Extracted season/episode: {}", season_episode);
                
                // Try searching by season and episode
                info!("Trying search by season/episode: {}", season_episode);
                match self.morbotron_client.search(&season_episode).await {
                    Ok(Some(result)) => {
                        info!("Found result by season/episode");
                        return Ok(Some(result));
                    },
                    _ => {
                        info!("No results found for season/episode search");
                    }
                }
            }
        }
        
        Ok(None)
    }
    
    // Helper method to extract quotes from text (looks for text in quotes)
    fn extract_quote_from_text(&self, text: &str) -> Option<String> {
        let re = Regex::new(r#""([^"]+)""#).ok()?;
        re.captures(text).and_then(|caps| caps.get(1).map(|m| m.as_str().to_string()))
    }
    
    // Helper method to extract episode information from text
    fn extract_episode_info(&self, text: &str) -> Option<String> {
        // Look for season/episode patterns like S01E01 or Season 1 Episode 1
        let re1 = Regex::new(r"S(\d+)E(\d+)").ok()?;
        if let Some(caps) = re1.captures(text) {
            if let (Some(season), Some(episode)) = (caps.get(1), caps.get(2)) {
                return Some(format!("S{}E{}", season.as_str(), episode.as_str()));
            }
        }
        
        let re2 = Regex::new(r"Season\s+(\d+)\s+Episode\s+(\d+)").ok()?;
        if let Some(caps) = re2.captures(text) {
            if let (Some(season), Some(episode)) = (caps.get(1), caps.get(2)) {
                return Some(format!("S{:02}E{:02}", 
                    season.as_str().parse::<u32>().unwrap_or(0),
                    episode.as_str().parse::<u32>().unwrap_or(0)));
            }
        }
        
        None
    }
    
    // Helper method to extract potential quotes from text
    fn extract_potential_quotes(&self, title: &str, snippet: &str) -> Vec<String> {
        let mut quotes = Vec::new();
        
        // Look for quoted text
        let re = Regex::new(r#""([^"]+)""#).ok().unwrap_or_else(|| Regex::new(r".*").unwrap());
        
        for text in &[title, snippet] {
            for cap in re.captures_iter(text) {
                if let Some(quote) = cap.get(1) {
                    quotes.push(quote.as_str().to_string());
                }
            }
        }
        
        quotes
    }
    
    // Helper method to check if a result contains the search terms
    fn result_contains_search_terms(&self, result: &MorbotronResult, query: &str) -> bool {
        let query_lower = query.to_lowercase();
        let caption_lower = result.caption.to_lowercase();
        let episode_title_lower = result.episode_title.to_lowercase();
        
        // Split query into words
        let query_words: Vec<&str> = query_lower.split_whitespace().collect();
        
        // Check if all query words are in the caption or episode title
        for word in &query_words {
            if !caption_lower.contains(word) && !episode_title_lower.contains(word) {
                return false;
            }
        }
        
        true
    }
    
    // Calculate a total relevance score for a result
    fn calculate_total_score(&self, result: &MorbotronResult, query: &str, search_term: &str) -> f32 {
        let query_lower = query.to_lowercase();
        let caption_lower = result.caption.to_lowercase();
        let episode_title_lower = result.episode_title.to_lowercase();
        
        // Split query into words
        let query_words: Vec<&str> = query_lower.split_whitespace().collect();
        
        // Count how many query words are in the caption and title
        let caption_matching_words = query_words.iter()
            .filter(|&word| caption_lower.contains(word))
            .count();
        
        let title_matching_words = query_words.iter()
            .filter(|&word| episode_title_lower.contains(word))
            .count();
        
        // Calculate score based on proportion of matching words in caption
        let caption_proportion = caption_matching_words as f32 / query_words.len() as f32;
        
        // Calculate score based on proportion of matching words in title
        let title_proportion = title_matching_words as f32 / query_words.len() as f32;
        
        // Give a bonus if all words match in either caption or title
        let all_words_bonus = if caption_matching_words == query_words.len() || title_matching_words == query_words.len() {
            0.5
        } else {
            0.0
        };
        
        // Combine scores with weights
        let combined_score = (caption_proportion * 0.7) + (title_proportion * 0.3) + all_words_bonus;
        
        // Cap at 1.0
        combined_score.min(1.0)
    }
}

// Helper function to extract frame ID from a Morbotron URL
fn extract_frame_id_from_url(url: &str) -> Option<String> {
    // Look for patterns like /meme/S05E08/1211643 or /caption/S05E08_1211643
    let meme_re = regex::Regex::new(r"/meme/([^/]+)/(\d+)").ok()?;
    if let Some(caps) = meme_re.captures(url) {
        if let (Some(episode), Some(timestamp)) = (caps.get(1), caps.get(2)) {
            return Some(format!("{}_{}", episode.as_str(), timestamp.as_str()));
        }
    }
    
    // Look for patterns like /caption/S05E08_1211643
    let caption_re = regex::Regex::new(r"/caption/([^/]+)").ok()?;
    if let Some(caps) = caption_re.captures(url) {
        if let Some(frame_id) = caps.get(1) {
            return Some(frame_id.as_str().to_string());
        }
    }
    
    None
}

// Extract phrases from a quote (split by punctuation)
fn extract_phrases(text: &str) -> Vec<String> {
    let mut phrases = Vec::new();
    
    // Split by common punctuation that separates phrases
    let separators = ['.', '!', '?', ';', ':', ','];
    
    let mut start = 0;
    for (i, c) in text.char_indices() {
        if separators.contains(&c) {
            if i > start {
                let phrase = text[start..i].trim().to_string();
                if !phrase.is_empty() {
                    phrases.push(phrase);
                }
            }
            start = i + 1;
        }
    }
    
    // Add the last phrase if there is one
    if start < text.len() {
        let phrase = text[start..].trim().to_string();
        if !phrase.is_empty() {
            phrases.push(phrase);
        }
    }
    
    // If no phrases were found (no punctuation), add the whole text as one phrase
    if phrases.is_empty() && !text.trim().is_empty() {
        phrases.push(text.trim().to_string());
    }
    
    phrases
}

// Helper function to extract season and episode information from text
fn extract_season_episode_from_text(text: &str) -> Option<String> {
    // Look for "Season X Episode Y" format
    let season_episode_re = regex::Regex::new(r"Season\s+(\d+)\s+Episode\s+(\d+)").ok()?;
    if let Some(caps) = season_episode_re.captures(text) {
        if let (Some(season), Some(episode)) = (caps.get(1), caps.get(2)) {
            return Some(format!("S{:02}E{:02}", 
                season.as_str().parse::<u32>().unwrap_or(0),
                episode.as_str().parse::<u32>().unwrap_or(0)));
        }
    }
    
    // Look for S##E## format
    let se_format_re = regex::Regex::new(r"S(\d+)E(\d+)").ok()?;
    if let Some(caps) = se_format_re.captures(text) {
        if let (Some(season), Some(episode)) = (caps.get(1), caps.get(2)) {
            return Some(format!("S{:02}E{:02}", 
                season.as_str().parse::<u32>().unwrap_or(0),
                episode.as_str().parse::<u32>().unwrap_or(0)));
        }
    }
    
    None
}

// Helper function to extract episode code from episode title
fn extract_episode_code_from_title(episode_title: &str) -> Option<String> {
    // Try to extract season and episode numbers from the title
    if let Some(season_episode) = extract_season_episode_from_text(episode_title) {
        return Some(season_episode);
    }
    
    // Look for "Season X Episode Y" format
    let season_episode_re = regex::Regex::new(r"Season\s+(\d+)\s+Episode\s+(\d+)").ok();
    if let Some(re) = season_episode_re {
        if let Some(caps) = re.captures(episode_title) {
            if let (Some(season), Some(episode)) = (caps.get(1), caps.get(2)) {
                return Some(format!("S{:02}E{:02}", 
                    season.as_str().parse::<u32>().unwrap_or(0),
                    episode.as_str().parse::<u32>().unwrap_or(0)));
            }
        }
    }
    
    // No episode code found
    None
}

// Helper function to check if a word is a common word that should be ignored in some contexts
fn is_common_word(word: &str) -> bool {
    const COMMON_WORDS: &[&str] = &[
        "the", "and", "that", "have", "for", "not", "with", "you", "this", "but",
        "his", "from", "they", "she", "will", "say", "would", "can", "been", "one",
        "all", "were", "when", "there", "what", "them", "some", "her", "who", "could",
        "make", "like", "time", "just", "him", "know", "take", "into", "year", "your",
        "good", "more", "than", "then", "look", "only", "come", "its", "over", "think",
        "also", "back", "after", "use", "two", "how", "our", "work", "first", "well",
        "way", "even", "new", "want", "because", "any", "these", "give", "day", "most",
    ];
    
    COMMON_WORDS.contains(&word.to_lowercase().as_str())
}
