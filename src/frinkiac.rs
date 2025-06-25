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
use crate::enhanced_frinkiac_search::EnhancedFrinkiacSearch;
use crate::gemini_api::GeminiClient;
use crate::text_formatting;
use crate::screenshot_search_common;

// API endpoints
const FRINKIAC_BASE_URL: &str = "https://frinkiac.com/api/search";
const FRINKIAC_CAPTION_URL: &str = "https://frinkiac.com/api/caption";
const FRINKIAC_IMAGE_URL: &str = "https://frinkiac.com/img";
const FRINKIAC_MEME_URL: &str = "https://frinkiac.com/meme";
const FRINKIAC_RANDOM_URL: &str = "https://frinkiac.com/api/random";

// Common search terms for random screenshots when no query is provided
const RANDOM_SEARCH_TERMS: &[&str] = &[
    "homer", "bart", "lisa", "marge", "maggie", "burns", "smithers",
    "flanders", "moe", "apu", "krusty", "milhouse", "ralph", "nelson",
    "skinner", "chalmers", "wiggum", "quimby", "troy", "mcclure", "hutz",
    "hibbert", "frink", "comic book guy", "barney", "lenny", "carl",
    "patty", "selma", "edna", "otto", "groundskeeper", "willie", "martin",
    "duffman", "gil", "sideshow", "bob", "mel", "itchy", "scratchy"
];

pub struct FrinkiacClient {
    http_client: HttpClient,
}

impl Clone for FrinkiacClient {
    fn clone(&self) -> Self {
        // Create a new HTTP client when cloning
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct FrinkiacSearchResult {
    #[serde(rename = "Id")]
    id: Option<u64>,
    #[serde(rename = "Episode")]
    episode: String,
    #[serde(rename = "Timestamp")]
    timestamp: u64,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct FrinkiacCaptionResult {
    #[serde(rename = "Episode")]
    episode: Option<FrinkiacEpisode>,
    #[serde(rename = "Subtitles")]
    subtitles: Vec<FrinkiacSubtitle>,
    #[serde(rename = "Framerate")]
    framerate: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct FrinkiacEpisode {
    #[serde(rename = "Key")]
    key: String,
    #[serde(rename = "Season")]
    season: u32,
    #[serde(rename = "EpisodeNumber")]
    episode_number: u32,
    #[serde(rename = "Title")]
    title: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct FrinkiacSubtitle {
    #[serde(rename = "Id")]
    id: Option<u64>,
    #[serde(rename = "RepresentativeTimestamp")]
    timestamp: u64,
    #[serde(rename = "Episode")]
    episode: String,
    #[serde(rename = "StartTimestamp")]
    start_timestamp: u64,
    #[serde(rename = "EndTimestamp")]
    end_timestamp: u64,
    #[serde(rename = "Content")]
    content: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct FrinkiacResult {
    pub episode: String,
    pub episode_title: String,
    pub season: u32,
    pub episode_number: u32,
    pub timestamp: String,
    pub image_url: String,
    pub meme_url: String,
    pub caption: String,
}

impl FrinkiacClient {
    pub fn new() -> Self {
        info!("Creating Frinkiac client");
        
        // Create HTTP client with reasonable timeouts
        let http_client = HttpClient::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client");
            
        Self { http_client }
    }

    // Get a random screenshot from Frinkiac
    pub async fn random(&self) -> Result<Option<FrinkiacResult>> {
        info!("Getting random Frinkiac screenshot");
        
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
    
    // Try to get a random screenshot using Frinkiac's random API
    async fn get_random_direct(&self) -> Result<Option<FrinkiacResult>> {
        // Make the request to the random API
        let random_response = self.http_client.get(FRINKIAC_RANDOM_URL)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to get random screenshot from Frinkiac: {}", e))?;
            
        if !random_response.status().is_success() {
            return Err(anyhow!("Frinkiac random API failed with status: {}", random_response.status()));
        }
        
        // Parse the response - the structure is different from what we expected
        // The random API returns a complex object with Episode, Frame, and Subtitles
        let random_result: serde_json::Value = random_response.json()
            .await
            .map_err(|e| anyhow!("Failed to parse Frinkiac random result: {}", e))?;
            
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
        let image_url = format!("{}/{}/{}.jpg", FRINKIAC_IMAGE_URL, episode, timestamp);
        
        // Format the meme URL (for sharing)
        let meme_url = format!("{}/{}/{}.jpg", FRINKIAC_MEME_URL, episode, timestamp);
        
        Ok(Some(FrinkiacResult {
            episode: episode.to_string(),
            episode_title,
            season,
            episode_number,
            timestamp: timestamp.to_string(),
            image_url,
            meme_url,
            caption,
        }))
    }

    // Get caption and details for a specific frame
    pub async fn get_caption_for_frame(&self, episode: &str, timestamp: u64) -> Result<Option<FrinkiacResult>> {
        // Get the caption for this frame
        let caption_url = format!("{}?e={}&t={}", FRINKIAC_CAPTION_URL, episode, timestamp);
        
        let caption_response = self.http_client.get(&caption_url)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to get caption from Frinkiac: {}", e))?;
            
        if !caption_response.status().is_success() {
            return Err(anyhow!("Frinkiac caption request failed with status: {}", caption_response.status()));
        }
        
        // Parse the caption result as a generic JSON Value first
        let caption_result: serde_json::Value = caption_response.json()
            .await
            .map_err(|e| anyhow!("Failed to parse Frinkiac caption result: {}", e))?;
            
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
        let image_url = format!("{}/{}/{}.jpg", FRINKIAC_IMAGE_URL, episode, timestamp);
        
        // Format the meme URL (for sharing)
        let meme_url = format!("{}/{}/{}.jpg", FRINKIAC_MEME_URL, episode, timestamp);
        
        Ok(Some(FrinkiacResult {
            episode: episode.to_string(),
            episode_title,
            season,
            episode_number,
            timestamp: timestamp.to_string(),
            image_url,
            meme_url,
            caption,
        }))
    }

    pub async fn search(&self, query: &str) -> Result<Option<FrinkiacResult>> {
        info!("Frinkiac search for: {}", query);
        
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
        info!("No Frinkiac results found for query: {}", query);
        Ok(None)
    }

    // Internal method to perform the actual API call with a specific search strategy
    async fn search_with_strategy(&self, query: &str) -> Result<Option<FrinkiacResult>> {
        // URL encode the query
        let encoded_query = urlencoding::encode(query);
        let search_url = format!("{}?q={}", FRINKIAC_BASE_URL, encoded_query);
        
        // Make the search request
        let search_response = self.http_client.get(&search_url)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to search Frinkiac: {}", e))?;
            
        if !search_response.status().is_success() {
            return Err(anyhow!("Frinkiac search failed with status: {}", search_response.status()));
        }
        
        // Parse the search results
        let search_results: Vec<FrinkiacSearchResult> = search_response.json()
            .await
            .map_err(|e| anyhow!("Failed to parse Frinkiac search results: {}", e))?;
            
        // If no results, return None
        if search_results.is_empty() {
            return Ok(None);
        }
        
        // Process all results and find the best match
        let mut best_result: Option<FrinkiacResult> = None;
        let mut best_score = 0.0;
        
        // Get the original query words for validation
        let query_lower = query.to_lowercase();
        let query_words: Vec<&str> = query_lower.split_whitespace().collect();
        
        // Process up to 5 results to find the best match
        for (i, result) in search_results.iter().take(5).enumerate() {
            let episode = &result.episode;
            let timestamp = result.timestamp;
            
            // Get the caption for this frame
            if let Ok(Some(frinkiac_result)) = self.get_caption_for_frame(episode, timestamp).await {
                // Calculate relevance score using our common utility
                let score = screenshot_search_common::calculate_result_relevance(
                    &frinkiac_result.caption,
                    &frinkiac_result.episode_title,
                    query,
                    &query_words
                );
                
                info!("Result #{} score: {:.2} for query: {}", i+1, score, query);
                
                // If this is the best result so far, keep it
                if score > best_score {
                    best_score = score;
                    best_result = Some(frinkiac_result);
                }
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
}

// Format a caption to proper sentence case and separate different speakers
fn format_caption(caption: &str) -> String {
    text_formatting::format_caption(caption, text_formatting::SIMPSONS_PROPER_NOUNS)
}

// Format a Frinkiac result for display
fn format_frinkiac_result(result: &FrinkiacResult) -> String {
    format!(
        "**S{:02}E{:02} - {}**\n{}\n\n{}",
        result.season, 
        result.episode_number, 
        result.episode_title,
        result.image_url,
        format_caption(&result.caption)
    )
}

// This function will be called from main.rs to handle the !frinkiac command
pub async fn handle_frinkiac_command(
    http: &Http, 
    msg: &Message, 
    args: Option<String>,
    frinkiac_client: &FrinkiacClient,
    gemini_client: Option<&GeminiClient>
) -> Result<()> {
    // Parse arguments to support filtering by season/episode
    let (search_term, season_filter, episode_filter) = if let Some(args_str) = args {
        parse_frinkiac_args(&args_str)
    } else {
        (None, None, None)
    };
    
    // If no search term is provided, get a random screenshot
    if search_term.is_none() && season_filter.is_none() && episode_filter.is_none() {
        info!("Frinkiac request for random screenshot");
        
        // Send a "searching" message
        let searching_msg = match msg.channel_id.say(http, "Finding a random Simpsons moment...").await {
            Ok(msg) => Some(msg),
            Err(e) => {
                error!("Error sending searching message: {:?}", e);
                None
            }
        };
        
        // Get a random screenshot
        match frinkiac_client.random().await {
            Ok(Some(result)) => {
                // Format the response
                let response = format_frinkiac_result(&result);
                
                // Edit the searching message if we have one, otherwise send a new message
                if let Some(mut search_msg) = searching_msg {
                    if let Err(e) = search_msg.edit(http, serenity::builder::EditMessage::new().content(&response)).await {
                        error!("Error editing searching message: {:?}", e);
                        // Try sending a new message if editing fails
                        if let Err(e) = msg.channel_id.say(http, &response).await {
                            error!("Error sending Frinkiac result: {:?}", e);
                        }
                    }
                } else {
                    if let Err(e) = msg.channel_id.say(http, &response).await {
                        error!("Error sending Frinkiac result: {:?}", e);
                    }
                }
            },
            Ok(None) => {
                let error_msg = "Couldn't find any Simpsons screenshots. D'oh!";
                
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
                let error_msg = format!("Error finding a random Simpsons screenshot: {}", e);
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
    
    // Construct a search message based on filters and terms
    let search_message = match (&search_term, &season_filter, &episode_filter) {
        (Some(term), None, None) => format!("Searching for Simpsons scene: \"{}\"...", term),
        (Some(term), Some(s), None) => format!("Searching for \"{}\" in season {}...", term, s),
        (Some(term), None, Some(e)) => format!("Searching for \"{}\" in episode {}...", term, e),
        (Some(term), Some(s), Some(e)) => format!("Searching for \"{}\" in S{}E{}...", term, s, e),
        (None, Some(s), None) => format!("Finding a random scene from season {}...", s),
        (None, None, Some(e)) => format!("Finding a random scene from episode {}...", e),
        (None, Some(s), Some(e)) => format!("Finding a random scene from S{}E{}...", s, e),
        (None, None, None) => "Finding a random Simpsons moment...".to_string(),
    };
    
    // Send a "searching" message
    let searching_msg = match msg.channel_id.say(http, &search_message).await {
        Ok(msg) => Some(msg),
        Err(e) => {
            error!("Error sending searching message: {:?}", e);
            None
        }
    };
    
    // Determine whether to use enhanced search or regular search
    let mut search_result = if let Some(term) = &search_term {
        if let Some(gemini) = gemini_client {
            info!("Using enhanced search with Gemini API and Google Search");
            let google_client = GoogleSearchClient::new();
            let enhanced_search = EnhancedFrinkiacSearch::new(gemini.clone(), frinkiac_client.clone(), google_client);
            enhanced_search.search(term).await
        } else {
            info!("Using regular search (Gemini API not available)");
            frinkiac_client.search(term).await
        }
    } else {
        // If no search term but we have filters, get a random screenshot
        frinkiac_client.random().await
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
            let response = format_frinkiac_result(&result);
            
            // Edit the searching message if we have one, otherwise send a new message
            if let Some(mut search_msg) = searching_msg {
                if let Err(e) = search_msg.edit(http, serenity::builder::EditMessage::new().content(&response)).await {
                    error!("Error editing searching message: {:?}", e);
                    // Try sending a new message if editing fails
                    if let Err(e) = msg.channel_id.say(http, &response).await {
                        error!("Error sending Frinkiac result: {:?}", e);
                    }
                }
            } else {
                if let Err(e) = msg.channel_id.say(http, &response).await {
                    error!("Error sending Frinkiac result: {:?}", e);
                }
            }
        },
        Ok(None) => {
            let error_msg = match (&search_term, &season_filter, &episode_filter) {
                (Some(term), None, None) => format!("No Simpsons scenes found for '{}'. Try a different phrase or wording.", term),
                (Some(term), Some(s), None) => format!("No Simpsons scenes found for '{}' in season {}.", term, s),
                (Some(term), None, Some(e)) => format!("No Simpsons scenes found for '{}' in episode {}.", term, e),
                (Some(term), Some(s), Some(e)) => format!("No Simpsons scenes found for '{}' in S{}E{}.", term, s, e),
                (None, Some(s), None) => format!("No Simpsons scenes found for season {}.", s),
                (None, None, Some(e)) => format!("No Simpsons scenes found for episode {}.", e),
                (None, Some(s), Some(e)) => format!("No Simpsons scenes found for S{}E{}.", s, e),
                (None, None, None) => "Couldn't find any Simpsons screenshots. D'oh!".to_string(),
            };
            
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
            let error_msg = format!("Error searching for Simpsons scene: {}", e);
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

// Parse arguments for the frinkiac command
// Format: !frinkiac [search term] [-s season] [-e episode]
fn parse_frinkiac_args(args: &str) -> (Option<String>, Option<u32>, Option<u32>) {
    let mut search_term = None;
    let mut season = None;
    let mut episode = None;
    
    // Split the args by spaces
    let parts: Vec<&str> = args.split_whitespace().collect();
    
    // Process the parts
    let mut i = 0;
    while i < parts.len() {
        match parts[i] {
            "-s" | "--season" => {
                // Next part should be the season number
                if i + 1 < parts.len() {
                    if let Ok(s) = parts[i + 1].parse::<u32>() {
                        season = Some(s);
                    }
                    i += 2;
                } else {
                    i += 1;
                }
            },
            "-e" | "--episode" => {
                // Next part should be the episode number
                if i + 1 < parts.len() {
                    if let Ok(e) = parts[i + 1].parse::<u32>() {
                        episode = Some(e);
                    }
                    i += 2;
                } else {
                    i += 1;
                }
            },
            // If it's not a flag, it's part of the search term
            _ => {
                // If we haven't set the search term yet, start collecting it
                if search_term.is_none() {
                    let mut term = String::new();
                    
                    // Collect all parts until we hit a flag or the end
                    let mut j = i;
                    while j < parts.len() && !parts[j].starts_with('-') {
                        if !term.is_empty() {
                            term.push(' ');
                        }
                        term.push_str(parts[j]);
                        j += 1;
                    }
                    
                    search_term = Some(term);
                    i = j;
                } else {
                    i += 1;
                }
            }
        }
    }
    
    (search_term, season, episode)
}

