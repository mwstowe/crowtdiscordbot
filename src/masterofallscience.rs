use anyhow::{Result, anyhow};
use serenity::model::channel::Message;
use serenity::all::Http;
use tracing::{error, info};
use reqwest::Client as HttpClient;
use serde::Deserialize;
use serde_json;
use std::time::Duration;
use rand::seq::SliceRandom;
use crate::google_search::GoogleSearchClient;
use crate::gemini_api::GeminiClient;
use crate::enhanced_masterofallscience_search::EnhancedMasterOfAllScienceSearch;
use crate::screenshot_search_utils;

// API endpoints
const MASTEROFALLSCIENCE_BASE_URL: &str = "https://masterofallscience.com/api/search";
const MASTEROFALLSCIENCE_CAPTION_URL: &str = "https://masterofallscience.com/api/caption";
const MASTEROFALLSCIENCE_IMAGE_URL: &str = "https://masterofallscience.com/img";
const MASTEROFALLSCIENCE_RANDOM_URL: &str = "https://masterofallscience.com/api/random";

// Common search terms for random screenshots when no query is provided
const RANDOM_SEARCH_TERMS: &[&str] = &[
    "rick", "morty", "summer", "beth", "jerry", "unity", "birdperson", "squanchy",
    "meeseeks", "portal", "pickle", "council", "citadel", "dimension", "schwifty",
    "gazorpazorp", "cronenberg", "purge", "microverse", "federation", "szechuan",
    "vindicators", "toxic", "froopyland", "mindblowers", "vat", "acid", "plumbus",
    "interdimensional", "cable", "eyeholes", "poopybutthole", "wubba", "lubba", "dub",
    "gromflomite", "smith", "sanchez", "evil", "morty", "president", "jessica", "principal",
    "tammy", "phoenix", "person", "scary", "terry", "noob", "noob", "snowball", "snuffles"
];

#[derive(Debug, Clone)]
pub struct MasterOfAllScienceClient {
    http_client: HttpClient,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct MasterOfAllScienceSearchResult {
    #[serde(rename = "Id")]
    id: Option<u64>,
    #[serde(rename = "Episode")]
    episode: String,
    #[serde(rename = "Timestamp")]
    timestamp: u64,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MasterOfAllScienceResult {
    pub episode: String,
    pub episode_title: String,
    pub season: u32,
    pub episode_number: u32,
    pub timestamp: String,
    pub image_url: String,
    pub caption: String,
}

impl MasterOfAllScienceClient {
    pub fn new() -> Self {
        info!("Creating MasterOfAllScience client");
        
        // Create HTTP client with reasonable timeouts
        let http_client = HttpClient::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client");
            
        Self { http_client }
    }

    // Get a random screenshot from MasterOfAllScience
    pub async fn random(&self) -> Result<Option<MasterOfAllScienceResult>> {
        info!("Getting random MasterOfAllScience screenshot");
        
        // Try the direct random API endpoint first
        match self.get_random_direct().await {
            Ok(Some(result)) => {
                info!("Successfully got random screenshot from direct API");
                return Ok(Some(result));
            },
            Ok(None) => {
                info!("No results from direct random API, trying fallback method");
            },
            Err(e) => {
                info!("Error from direct random API: {}, trying fallback method", e);
            }
        }
        
        // Fallback: Use a random search term from our list
        // Select a random term before the async operation to avoid Send issues
        let random_term = {
            let mut rng = rand::thread_rng();
            RANDOM_SEARCH_TERMS.choose(&mut rng)
                .ok_or_else(|| anyhow!("Failed to select random search term"))?
                .to_string() // Convert to owned String to avoid lifetime issues
        };
        
        info!("Using random search term: {}", random_term);
        self.search(&random_term).await
    }
    
    // Try to get a random screenshot using MasterOfAllScience's random API
    async fn get_random_direct(&self) -> Result<Option<MasterOfAllScienceResult>> {
        // Make the request to the random API
        let random_response = self.http_client.get(MASTEROFALLSCIENCE_RANDOM_URL)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to get random screenshot from MasterOfAllScience: {}", e))?;
            
        if !random_response.status().is_success() {
            return Err(anyhow!("MasterOfAllScience random API failed with status: {}", random_response.status()));
        }
        
        // Parse the response - the structure is different from what we expected
        // The random API returns a complex object with Episode, Frame, and Subtitles
        let random_result: serde_json::Value = random_response.json()
            .await
            .map_err(|e| anyhow!("Failed to parse MasterOfAllScience random result: {}", e))?;
            
        // Extract the frame information
        let frame = random_result.get("Frame")
            .ok_or_else(|| anyhow!("Missing Frame in random result"))?;
            
        let episode = frame.get("Episode")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing Episode in frame"))?;
            
        let timestamp = frame.get("Timestamp")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow!("Missing Timestamp in frame"))?;
            
        // Extract episode information
        let episode_info = random_result.get("Episode")
            .ok_or_else(|| anyhow!("Missing Episode info in random result"))?;
            
        let season = episode_info.get("Season")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
            
        let episode_number = episode_info.get("EpisodeNumber")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
            
        let episode_title = episode_info.get("Title")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();
            
        // Extract subtitles/caption
        let subtitles = random_result.get("Subtitles")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow!("Missing Subtitles in random result"))?;
            
        let caption = subtitles.iter()
            .filter_map(|s| s.get("Content").and_then(|c| c.as_str()))
            .collect::<Vec<&str>>()
            .join(" ");
            
        // Format the image URL
        let image_url = format!("{}/{}/{}.jpg", MASTEROFALLSCIENCE_IMAGE_URL, episode, timestamp);
        
        Ok(Some(MasterOfAllScienceResult {
            episode: episode.to_string(),
            episode_title,
            season,
            episode_number,
            timestamp: timestamp.to_string(),
            image_url,
            caption,
        }))
    }

    pub async fn search(&self, query: &str) -> Result<Option<MasterOfAllScienceResult>> {
        info!("MasterOfAllScience search for: {}", query);
        
        // Try different search strategies in order of preference
        
        // 1. Try exact phrase search with quotes
        if let Some(result) = self.search_with_strategy(&format!("\"{}\"", query)).await? {
            info!("Found result with exact phrase search");
            return Ok(Some(result));
        }
        
        // 2. Try exact phrase search without quotes (in case API handles it differently)
        if let Some(result) = self.search_with_strategy(query).await? {
            info!("Found result with standard search");
            return Ok(Some(result));
        }
        
        // 3. Try with plus signs between words to force word boundaries
        let plus_query = query.split_whitespace().collect::<Vec<&str>>().join("+");
        if let Some(result) = self.search_with_strategy(&plus_query).await? {
            info!("Found result with plus-separated search");
            return Ok(Some(result));
        }
        
        // 4. Try with variations of "as" phrases (for cases like "as safe as they said")
        if query.contains(" as ") {
            let variations = crate::screenshot_search_utils::generate_as_phrase_variations(query);
            for variation in variations {
                info!("Trying 'as' phrase variation: {}", variation);
                if let Some(result) = self.search_with_strategy(&variation).await? {
                    info!("Found result with 'as' phrase variation");
                    return Ok(Some(result));
                }
            }
        }
        
        // 5. Try with variations for common speech patterns
        let speech_variations = crate::screenshot_search_utils::generate_speech_pattern_variations(query);
        for variation in speech_variations {
            info!("Trying speech pattern variation: {}", variation);
            if let Some(result) = self.search_with_strategy(&variation).await? {
                info!("Found result with speech pattern variation");
                return Ok(Some(result));
            }
        }
        
        // 6. If the query has multiple words, try searching for pairs of consecutive words
        let words: Vec<&str> = query.split_whitespace().collect();
        if words.len() > 1 {
            for i in 0..words.len() - 1 {
                let pair_query = format!("{} {}", words[i], words[i + 1]);
                info!("Trying pair search: {}", pair_query);
                if let Some(result) = self.search_with_strategy(&pair_query).await? {
                    info!("Found result with word pair search");
                    return Ok(Some(result));
                }
            }
        }
        
        // 7. Try searching for individual significant words
        if words.len() > 1 {
            // Skip common words and focus on significant ones
            let significant_words: Vec<&str> = words.iter()
                .filter(|&&word| {
                    let word_lower = word.to_lowercase();
                    word_lower.len() > 3 && !crate::screenshot_search_utils::is_common_word(&word_lower)
                })
                .copied()
                .collect();
                
            for word in significant_words {
                info!("Trying single word search: {}", word);
                if let Some(result) = self.search_with_strategy(word).await? {
                    info!("Found result with single word search");
                    return Ok(Some(result));
                }
            }
        }
        
        // 8. Try with fuzzy variations of the query
        let fuzzy_variations = crate::screenshot_search_utils::generate_fuzzy_variations(query);
        for variation in fuzzy_variations {
            info!("Trying fuzzy variation: {}", variation);
            if let Some(result) = self.search_with_strategy(&variation).await? {
                info!("Found result with fuzzy variation");
                return Ok(Some(result));
            }
        }
        
        // No results found with any strategy
        info!("No MasterOfAllScience results found for query: {}", query);
        Ok(None)
    }

    // Internal method to perform the actual API call with a specific search strategy
    async fn search_with_strategy(&self, query: &str) -> Result<Option<MasterOfAllScienceResult>> {
        // URL encode the query
        let encoded_query = urlencoding::encode(query);
        let search_url = format!("{}?q={}", MASTEROFALLSCIENCE_BASE_URL, encoded_query);
        
        // Make the search request
        let search_response = self.http_client.get(&search_url)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to search MasterOfAllScience: {}", e))?;
            
        if !search_response.status().is_success() {
            return Err(anyhow!("MasterOfAllScience search failed with status: {}", search_response.status()));
        }
        
        // Parse the search results
        let search_results: Vec<MasterOfAllScienceSearchResult> = search_response.json()
            .await
            .map_err(|e| anyhow!("Failed to parse MasterOfAllScience search results: {}", e))?;
            
        // If no results, return None
        if search_results.is_empty() {
            return Ok(None);
        }
        
        // Take the first result
        let first_result = &search_results[0];
        let episode = &first_result.episode;
        let timestamp = first_result.timestamp;
        
        // Get the caption for this frame
        self.get_caption_for_frame(episode, timestamp).await
    }
    
    // Get caption and details for a specific frame
    async fn get_caption_for_frame(&self, episode: &str, timestamp: u64) -> Result<Option<MasterOfAllScienceResult>> {
        // Get the caption for this frame
        let caption_url = format!("{}?e={}&t={}", MASTEROFALLSCIENCE_CAPTION_URL, episode, timestamp);
        
        let caption_response = self.http_client.get(&caption_url)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to get caption from MasterOfAllScience: {}", e))?;
            
        if !caption_response.status().is_success() {
            return Err(anyhow!("MasterOfAllScience caption request failed with status: {}", caption_response.status()));
        }
        
        // Parse the caption result as a generic JSON Value first
        let caption_result: serde_json::Value = caption_response.json()
            .await
            .map_err(|e| anyhow!("Failed to parse MasterOfAllScience caption result: {}", e))?;
            
        // Extract episode information
        let episode_info = caption_result.get("Episode")
            .ok_or_else(|| anyhow!("Missing Episode info in caption result"))?;
            
        let episode_title = episode_info.get("Title")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();
            
        let season = episode_info.get("Season")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
            
        let episode_number = episode_info.get("EpisodeNumber")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
            
        // Extract subtitles/caption
        let subtitles = caption_result.get("Subtitles")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow!("Missing Subtitles in caption result"))?;
            
        let caption = subtitles.iter()
            .filter_map(|s| s.get("Content").and_then(|c| c.as_str()))
            .collect::<Vec<&str>>()
            .join(" ");
            
        // Format the image URL
        let image_url = format!("{}/{}/{}.jpg", MASTEROFALLSCIENCE_IMAGE_URL, episode, timestamp);
        
        Ok(Some(MasterOfAllScienceResult {
            episode: episode.to_string(),
            episode_title,
            season,
            episode_number,
            timestamp: timestamp.to_string(),
            image_url,
            caption,
        }))
    }
}

// Format a caption to proper sentence case and separate different speakers
fn format_caption(caption: &str) -> String {
    // Split by newlines to get potential different speakers
    let lines: Vec<&str> = caption.split('\n')
        .filter(|line| !line.trim().is_empty())
        .collect();
    
    // Process each line
    let mut formatted_lines: Vec<String> = Vec::new();
    let mut current_speaker_lines: Vec<String> = Vec::new();
    
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        
        // Convert to sentence case (first letter capitalized, rest lowercase)
        let sentence_case = if !trimmed.is_empty() {
            let mut chars = trimmed.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + &chars.collect::<String>().to_lowercase(),
            }
        } else {
            String::new()
        };
        
        // Check if this is likely a new speaker (empty line before or all caps line)
        let is_new_speaker = current_speaker_lines.is_empty() || 
                            trimmed == trimmed.to_uppercase() && 
                            trimmed.chars().any(|c| c.is_alphabetic());
        
        if is_new_speaker && !current_speaker_lines.is_empty() {
            // Join previous speaker's lines and add to formatted lines
            formatted_lines.push(format!("\"{}\"", current_speaker_lines.join(" ")));
            current_speaker_lines.clear();
        }
        
        // Add this line to current speaker
        current_speaker_lines.push(sentence_case);
    }
    
    // Add the last speaker's lines
    if !current_speaker_lines.is_empty() {
        formatted_lines.push(format!("\"{}\"", current_speaker_lines.join(" ")));
    }
    
    // Join all formatted parts
    formatted_lines.join(" ")
}

// Format a MasterOfAllScience result for display
fn format_masterofallscience_result(result: &MasterOfAllScienceResult) -> String {
    format!(
        "**S{:02}E{:02} - {}**\n{}\n\n{}",
        result.season, 
        result.episode_number, 
        result.episode_title,
        result.image_url,
        format_caption(&result.caption)
    )
}

// This function will be called from main.rs to handle the !masterofallscience command
pub async fn handle_masterofallscience_command(
    http: &Http, 
    msg: &Message, 
    search_term: Option<String>,
    masterofallscience_client: &MasterOfAllScienceClient,
    gemini_client: Option<&GeminiClient>
) -> Result<()> {
    // If no search term is provided, get a random screenshot
    if search_term.is_none() {
        info!("MasterOfAllScience request for random screenshot");
        
        // Send a "searching" message
        let searching_msg = match msg.channel_id.say(http, "Finding a random Rick and Morty moment...").await {
            Ok(msg) => Some(msg),
            Err(e) => {
                error!("Error sending searching message: {:?}", e);
                None
            }
        };
        
        // Get a random screenshot
        match masterofallscience_client.random().await {
            Ok(Some(result)) => {
                // Format the response
                let response = format_masterofallscience_result(&result);
                
                // Edit the searching message if we have one, otherwise send a new message
                if let Some(mut search_msg) = searching_msg {
                    if let Err(e) = search_msg.edit(http, serenity::builder::EditMessage::new().content(&response)).await {
                        error!("Error editing searching message: {:?}", e);
                        // Try sending a new message if editing fails
                        if let Err(e) = msg.channel_id.say(http, &response).await {
                            error!("Error sending MasterOfAllScience result: {:?}", e);
                        }
                    }
                } else {
                    if let Err(e) = msg.channel_id.say(http, &response).await {
                        error!("Error sending MasterOfAllScience result: {:?}", e);
                    }
                }
            },
            Ok(None) => {
                let error_msg = "Couldn't find any Rick and Morty screenshots. Wubba lubba dub dub!";
                
                // Edit the searching message if we have one, otherwise send a new message
                if let Some(mut search_msg) = searching_msg {
                    if let Err(e) = search_msg.edit(http, serenity::builder::EditMessage::new().content(error_msg)).await {
                        error!("Error editing searching message: {:?}", e);
                        // Try sending a new message if editing fails
                        if let Err(e) = msg.channel_id.say(http, error_msg).await {
                            error!("Error sending no results message: {:?}", e);
                        }
                    }
                } else {
                    if let Err(e) = msg.channel_id.say(http, error_msg).await {
                        error!("Error sending no results message: {:?}", e);
                    }
                }
            },
            Err(e) => {
                let error_msg = format!("Error finding a random Rick and Morty screenshot: {}", e);
                error!("{}", &error_msg);
                
                // Edit the searching message if we have one, otherwise send a new message
                if let Some(mut search_msg) = searching_msg {
                    if let Err(e) = search_msg.edit(http, serenity::builder::EditMessage::new().content(&error_msg)).await {
                        error!("Error editing searching message: {:?}", e);
                        // Try sending a new message if editing fails
                        if let Err(e) = msg.channel_id.say(http, &error_msg).await {
                            error!("Error sending error message: {:?}", e);
                        }
                    }
                } else {
                    if let Err(e) = msg.channel_id.say(http, &error_msg).await {
                        error!("Error sending error message: {:?}", e);
                    }
                }
            }
        }
        
        return Ok(());
    }
    
    // If a search term is provided, search for it
    let term = search_term.unwrap();
    info!("MasterOfAllScience request with search term: {}", term);
    
    // Send a "searching" message
    let searching_msg = match msg.channel_id.say(http, format!("Searching for Rick and Morty scene: \"{}\"...", term)).await {
        Ok(msg) => Some(msg),
        Err(e) => {
            error!("Error sending searching message: {:?}", e);
            None
        }
    };
    
    // Determine whether to use enhanced search or regular search
    let search_result = if let Some(gemini) = gemini_client {
        info!("Using enhanced search with Gemini API and Google Search");
        let google_client = GoogleSearchClient::new();
        let enhanced_search = EnhancedMasterOfAllScienceSearch::new(gemini.clone(), masterofallscience_client.clone(), google_client);
        enhanced_search.search(&term).await
    } else {
        info!("Using regular search (Gemini API not available)");
        masterofallscience_client.search(&term).await
    };
    
    // Process the search result
    match search_result {
        Ok(Some(result)) => {
            // Format the response
            let response = format_masterofallscience_result(&result);
            
            // Edit the searching message if we have one, otherwise send a new message
            if let Some(mut search_msg) = searching_msg {
                if let Err(e) = search_msg.edit(http, serenity::builder::EditMessage::new().content(&response)).await {
                    error!("Error editing searching message: {:?}", e);
                    // Try sending a new message if editing fails
                    if let Err(e) = msg.channel_id.say(http, &response).await {
                        error!("Error sending MasterOfAllScience result: {:?}", e);
                    }
                }
            } else {
                if let Err(e) = msg.channel_id.say(http, &response).await {
                    error!("Error sending MasterOfAllScience result: {:?}", e);
                }
            }
        },
        Ok(None) => {
            let error_msg = format!("No Rick and Morty scenes found for '{}'. Try a different phrase or wording.", term);
            
            // Edit the searching message if we have one, otherwise send a new message
            if let Some(mut search_msg) = searching_msg {
                if let Err(e) = search_msg.edit(http, serenity::builder::EditMessage::new().content(&error_msg)).await {
                    error!("Error editing searching message: {:?}", e);
                    // Try sending a new message if editing fails
                    if let Err(e) = msg.channel_id.say(http, &error_msg).await {
                        error!("Error sending no results message: {:?}", e);
                    }
                }
            } else {
                if let Err(e) = msg.channel_id.say(http, &error_msg).await {
                    error!("Error sending no results message: {:?}", e);
                }
            }
        },
        Err(e) => {
            let error_msg = format!("Error searching for Rick and Morty scene: {}", e);
            error!("{}", &error_msg);
            
            // Edit the searching message if we have one, otherwise send a new message
            if let Some(mut search_msg) = searching_msg {
                if let Err(e) = search_msg.edit(http, serenity::builder::EditMessage::new().content(&error_msg)).await {
                    error!("Error editing searching message: {:?}", e);
                    // Try sending a new message if editing fails
                    if let Err(e) = msg.channel_id.say(http, &error_msg).await {
                        error!("Error sending error message: {:?}", e);
                    }
                }
            } else {
                if let Err(e) = msg.channel_id.say(http, &error_msg).await {
                    error!("Error sending error message: {:?}", e);
                }
            }
        }
    }
    
    Ok(())
}
