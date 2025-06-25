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
use crate::text_formatting;
use crate::screenshot_search_common;

// API endpoints
const MASTEROFALLSCIENCE_BASE_URL: &str = "https://masterofallscience.com/api/search";
const MASTEROFALLSCIENCE_CAPTION_URL: &str = "https://masterofallscience.com/api/caption";
const MASTEROFALLSCIENCE_IMAGE_URL: &str = "https://masterofallscience.com/img";
const MASTEROFALLSCIENCE_RANDOM_URL: &str = "https://masterofallscience.com/api/random";

// Common search terms for random screenshots when no query is provided
const RANDOM_SEARCH_TERMS: &[&str] = &[
    "rick", "morty", "summer", "beth", "jerry", "portal gun", "wubba lubba dub dub",
    "get schwifty", "pickle rick", "tiny rick", "mr. meeseeks", "look at me",
    "plumbus", "interdimensional cable", "gazorpazorp", "unity", "bird person",
    "squanchy", "council of ricks", "citadel", "dimension c-137", "cronenberg",
    "purge", "szechuan sauce", "evil morty", "mr. poopybutthole", "scary terry",
    "butter robot", "pass the butter", "snuffles", "snowball", "where are my testicles",
    "keep summer safe", "show me what you got", "head bent over", "raised up posterior",
    "ants in my eyes johnson", "two brothers", "ball fondlers", "real fake doors",
    "personal space", "lil' bits", "eyeholes", "gazorpazorpfield", "turbulent juice",
    "microverse", "miniverse", "teenyverse", "roy", "morty's mind blowers",
    "vindicators", "noob noob", "got damn", "toxic rick", "toxic morty",
    "froopyland", "simple rick", "story train", "vat of acid", "snake jazz",
    "time travel", "space", "alien", "dimension", "universe", "multiverse",
    "science", "adventure", "family", "garage", "lab", "ship", "gun", "portal"
];

// MasterOfAllScience search result structure
#[derive(Deserialize, Debug)]
struct MasterOfAllScienceSearchResult {
    #[serde(rename = "Id")]
    #[serde(default)]
    id: String,
    #[serde(rename = "Episode")]
    episode: String,
    #[serde(rename = "Timestamp")]
    timestamp: u64,
}

// MasterOfAllScience caption result structure
#[derive(Deserialize, Debug)]
struct MasterOfAllScienceCaptionResult {
    Subtitles: Vec<MasterOfAllScienceSubtitle>,
    Episode: MasterOfAllScienceEpisode,
}

#[derive(Deserialize, Debug)]
struct MasterOfAllScienceSubtitle {
    Content: String,
    StartTimestamp: u64,
    EndTimestamp: u64,
}

#[derive(Deserialize, Debug)]
struct MasterOfAllScienceEpisode {
    Title: String,
    Season: u32,
    Episode: u32,
}

// MasterOfAllScience result structure for returning to the caller
#[derive(Debug, Clone)]
pub struct MasterOfAllScienceResult {
    pub episode: String,
    pub season: u32,
    pub episode_number: u32,
    pub episode_title: String,
    pub timestamp: String,
    pub image_url: String,
    pub caption: String,
}

// MasterOfAllScience client for searching and retrieving captions
#[derive(Clone)]
pub struct MasterOfAllScienceClient {
    http_client: HttpClient,
}

impl MasterOfAllScienceClient {
    // Create a new MasterOfAllScience client
    pub fn new() -> Self {
        let http_client = HttpClient::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
            
        MasterOfAllScienceClient {
            http_client,
        }
    }
    
    // Get a random screenshot
    pub async fn random(&self) -> Result<Option<MasterOfAllScienceResult>> {
        // Choose a random search term
        let random_term = RANDOM_SEARCH_TERMS.choose(&mut rand::thread_rng())
            .ok_or_else(|| anyhow!("Failed to choose random search term"))?;
            
        info!("Using random search term: {}", random_term);
        self.search(random_term).await
    }
    
    // Search for a screenshot matching the query
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
        
        info!("Sending request to MasterOfAllScience API: {}", search_url);
        
        // Make the search request
        let search_response = self.http_client.get(&search_url)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to search MasterOfAllScience: {}", e))?;
            
        let status = search_response.status();
        info!("MasterOfAllScience API response status: {}", status);
        
        if !status.is_success() {
            return Err(anyhow!("MasterOfAllScience search failed with status: {}", status));
        }
        
        // Get the response body as text first
        let response_body = search_response.text().await
            .map_err(|e| anyhow!("Failed to get MasterOfAllScience response body: {}", e))?;
        
        info!("MasterOfAllScience API response body: {}", response_body);
        
        // Parse the search results
        let search_results: Vec<MasterOfAllScienceSearchResult> = match serde_json::from_str::<Vec<MasterOfAllScienceSearchResult>>(&response_body) {
            Ok(results) => {
                info!("Successfully parsed MasterOfAllScience search results: {} results", results.len());
                results
            },
            Err(e) => {
                error!("Failed to parse MasterOfAllScience search results: {}. Response body: {}", e, response_body);
                return Err(anyhow!("Failed to parse MasterOfAllScience search results: {}", e));
            }
        };
            
        // If no results, return None
        if search_results.is_empty() {
            return Ok(None);
        }
        
        // Process all results and find the best match
        let mut best_result: Option<MasterOfAllScienceResult> = None;
        let mut best_score = 0.0;
        
        // Get the original query words for validation
        let query_lower = query.to_lowercase();
        let query_words: Vec<&str> = query_lower.split_whitespace().collect();
        
        // Process up to 5 results to find the best match
        for (i, result) in search_results.iter().take(5).enumerate() {
            let episode = &result.episode;
            let timestamp = result.timestamp;
            
            // Get the caption for this frame
            match self.get_caption_for_frame(episode, timestamp).await {
                Ok(Some(masterofallscience_result)) => {
                    // Calculate relevance score using our common utility
                    let score = screenshot_search_common::calculate_result_relevance(
                        &masterofallscience_result.caption,
                        &masterofallscience_result.episode_title,
                        query,
                        &query_words
                    );
                    
                    info!("Result #{} score: {:.2} for query: {}", i+1, score, query);
                    
                    // If this is the best result so far, keep it
                    if score > best_score {
                        best_score = score;
                        best_result = Some(masterofallscience_result);
                    }
                },
                _ => continue
            }
        }
        
        // Return the best result, or None if no good matches were found
        if best_score > 0.3 {  // Minimum threshold for relevance
            Ok(best_result)
        } else {
            info!("No relevant results found for query: {}", query);
            Ok(None)
        }
    }
    
    // Get the caption for a specific frame
    async fn get_caption_for_frame(&self, episode: &str, timestamp: u64) -> Result<Option<MasterOfAllScienceResult>> {
        // Build the caption URL
        let caption_url = format!("{}/{}/{}", MASTEROFALLSCIENCE_CAPTION_URL, episode, timestamp);
        
        // Make the caption request
        let caption_response = self.http_client.get(&caption_url)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to get MasterOfAllScience caption: {}", e))?;
            
        if !caption_response.status().is_success() {
            return Err(anyhow!("MasterOfAllScience caption request failed with status: {}", caption_response.status()));
        }
        
        // Parse the caption result
        let caption_result: MasterOfAllScienceCaptionResult = caption_response.json()
            .await
            .map_err(|e| anyhow!("Failed to parse MasterOfAllScience caption: {}", e))?;
            
        // If no subtitles, return None
        if caption_result.Subtitles.is_empty() {
            return Ok(None);
        }
        
        // Extract the caption text
        let caption = caption_result.Subtitles.iter()
            .map(|s| s.Content.clone())
            .collect::<Vec<String>>()
            .join("\n");
            
        // Build the image URL
        let image_url = format!("{}/{}/{}.jpg", MASTEROFALLSCIENCE_IMAGE_URL, episode, timestamp);
        
        // Extract episode information
        let episode_title = caption_result.Episode.Title.clone();
        let season = caption_result.Episode.Season;
        let episode_number = caption_result.Episode.Episode;
        
        // Return the result
        Ok(Some(MasterOfAllScienceResult {
            episode: episode.to_string(),
            season,
            episode_number,
            episode_title,
            timestamp: timestamp.to_string(),
            image_url,
            caption,
        }))
    }
}

// Format a caption to proper sentence case and separate different speakers
fn format_caption(caption: &str) -> String {
    text_formatting::format_caption(caption, text_formatting::RICK_AND_MORTY_PROPER_NOUNS)
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
    args: Option<String>,
    masterofallscience_client: &MasterOfAllScienceClient,
    gemini_client: Option<&GeminiClient>
) -> Result<()> {
    // Parse arguments to support filtering by season/episode
    let (search_term, season_filter, episode_filter) = if let Some(args_str) = args {
        parse_masterofallscience_args(&args_str)
    } else {
        (None, None, None)
    };
    
    // Show a "searching" message that we'll edit later with the result
    let searching_msg = if let Ok(sent_msg) = msg.channel_id.say(http, "🔍 Searching Rick and Morty quotes...").await {
        Some(sent_msg)
    } else {
        None
    };
    
    // Determine whether to use enhanced search or regular search
    let mut search_result = if let Some(term) = &search_term {
        if let Some(gemini) = gemini_client {
            info!("Using enhanced search with Gemini API and Google Search");
            let google_client = GoogleSearchClient::new();
            let enhanced_search = EnhancedMasterOfAllScienceSearch::new(gemini.clone(), masterofallscience_client.clone(), google_client);
            enhanced_search.search(term).await
        } else {
            info!("Using regular search (Gemini API not available)");
            masterofallscience_client.search(term).await
        }
    } else {
        // If no search term but we have filters, get a random screenshot
        masterofallscience_client.random().await
    };
    
    // Apply filters if needed
    if let Ok(Some(ref mut result)) = search_result {
        let mut filtered_out = false;
        
        // Filter by season if specified
        if let Some(season) = season_filter {
            if result.season != season {
                filtered_out = true;
                info!("Result filtered out: season {} doesn't match filter {}", result.season, season);
            }
        }
        
        // Filter by episode if specified
        if let Some(episode) = episode_filter {
            if result.episode_number != episode {
                filtered_out = true;
                info!("Result filtered out: episode {} doesn't match filter {}", result.episode_number, episode);
            }
        }
        
        // If filtered out, return appropriate message
        if filtered_out {
            search_result = Ok(None);
        }
    }
    
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
            let error_msg = "Couldn't find any Rick and Morty screenshots. Wubba lubba dub dub!";
            
            // Edit the searching message if we have one, otherwise send a new message
            if let Some(mut search_msg) = searching_msg {
                if let Err(e) = search_msg.edit(http, serenity::builder::EditMessage::new().content(error_msg)).await {
                    error!("Error editing searching message: {:?}", e);
                    // Try sending a new message if editing fails
                    if let Err(e) = msg.channel_id.say(http, error_msg).await {
                        error!("Error sending MasterOfAllScience error message: {:?}", e);
                    }
                }
            } else {
                if let Err(e) = msg.channel_id.say(http, error_msg).await {
                    error!("Error sending MasterOfAllScience error message: {:?}", e);
                }
            }
        },
        Err(e) => {
            // Create a user-friendly error message
            let user_error_msg = "Couldn't find any Rick and Morty screenshots. Wubba lubba dub dub!";
            
            // Log the detailed error for debugging
            error!("Error searching MasterOfAllScience: {}", e);
            
            // Edit the searching message if we have one, otherwise send a new message
            if let Some(mut search_msg) = searching_msg {
                if let Err(e) = search_msg.edit(http, serenity::builder::EditMessage::new().content(user_error_msg)).await {
                    error!("Error editing searching message: {:?}", e);
                    // Try sending a new message if editing fails
                    if let Err(e) = msg.channel_id.say(http, user_error_msg).await {
                        error!("Error sending MasterOfAllScience error message: {:?}", e);
                    }
                }
            } else {
                if let Err(e) = msg.channel_id.say(http, user_error_msg).await {
                    error!("Error sending MasterOfAllScience error message: {:?}", e);
                }
            }
        }
    }
    
    Ok(())
}

// Parse arguments for the !masterofallscience command
// Format: !masterofallscience [search term] [-s season] [-e episode]
fn parse_masterofallscience_args(args: &str) -> (Option<String>, Option<u32>, Option<u32>) {
    let mut search_term = String::new();
    let mut season: Option<u32> = None;
    let mut episode: Option<u32> = None;
    
    let mut parts = args.split_whitespace().peekable();
    
    while let Some(part) = parts.next() {
        match part {
            "-s" | "-season" => {
                if let Some(season_str) = parts.next() {
                    season = season_str.parse::<u32>().ok();
                }
            },
            "-e" | "-episode" => {
                if let Some(episode_str) = parts.next() {
                    episode = episode_str.parse::<u32>().ok();
                }
            },
            _ => {
                // If we already have some search term, add a space
                if !search_term.is_empty() {
                    search_term.push(' ');
                }
                search_term.push_str(part);
            }
        }
    }
    
    // If search term is empty, return None
    let search_term = if search_term.is_empty() {
        None
    } else {
        Some(search_term)
    };
    
    (search_term, season, episode)
}
