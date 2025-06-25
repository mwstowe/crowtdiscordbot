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
use crate::enhanced_morbotron_search::EnhancedMorbotronSearch;

// API endpoints
const MORBOTRON_BASE_URL: &str = "https://morbotron.com/api/search";
const MORBOTRON_CAPTION_URL: &str = "https://morbotron.com/api/caption";
const MORBOTRON_IMAGE_URL: &str = "https://morbotron.com/img";
const MORBOTRON_RANDOM_URL: &str = "https://morbotron.com/api/random";

// Common search terms for random screenshots when no query is provided
const RANDOM_SEARCH_TERMS: &[&str] = &[
    "fry", "leela", "bender", "professor", "zoidberg", "amy", "hermes", "zapp", "kif",
    "nibbler", "mom", "robot", "hypnotoad", "scruffy", "nixon", "calculon", "lrrr", "morbo",
    "roberto", "url", "smitty", "linda", "hedonismbot", "flexo", "donbot", "clamps", "joey",
    "elzar", "wernstrom", "cubert", "dwight", "labarbara", "leo", "guenter", "bubblegum",
    "farnsworth", "brannigan", "planet express", "good news", "bite", "shiny", "metal"
];

#[derive(Debug, Clone)]
pub struct MorbotronClient {
    http_client: HttpClient,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct MorbotronSearchResult {
    #[serde(rename = "Id")]
    id: Option<u64>,
    #[serde(rename = "Episode")]
    episode: String,
    #[serde(rename = "Timestamp")]
    timestamp: u64,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MorbotronResult {
    pub episode: String,
    pub episode_title: String,
    pub season: u32,
    pub episode_number: u32,
    pub timestamp: String,
    pub image_url: String,
    pub caption: String,
}

impl MorbotronClient {
    pub fn new() -> Self {
        info!("Creating Morbotron client");
        
        // Create HTTP client with reasonable timeouts
        let http_client = HttpClient::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client");
            
        Self { http_client }
    }

    // Get a random screenshot from Morbotron
    pub async fn random(&self) -> Result<Option<MorbotronResult>> {
        info!("Getting random Morbotron screenshot");
        
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
    
    // Try to get a random screenshot using Morbotron's random API
    async fn get_random_direct(&self) -> Result<Option<MorbotronResult>> {
        // Make the request to the random API
        let random_response = self.http_client.get(MORBOTRON_RANDOM_URL)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to get random screenshot from Morbotron: {}", e))?;
            
        if !random_response.status().is_success() {
            return Err(anyhow!("Morbotron random API failed with status: {}", random_response.status()));
        }
        
        // Parse the response - the structure is different from what we expected
        // The random API returns a complex object with Episode, Frame, and Subtitles
        let random_result: serde_json::Value = random_response.json()
            .await
            .map_err(|e| anyhow!("Failed to parse Morbotron random result: {}", e))?;
            
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
        let image_url = format!("{}/{}/{}.jpg", MORBOTRON_IMAGE_URL, episode, timestamp);
        
        Ok(Some(MorbotronResult {
            episode: episode.to_string(),
            episode_title,
            season,
            episode_number,
            timestamp: timestamp.to_string(),
            image_url,
            caption,
        }))
    }

    pub async fn search(&self, query: &str) -> Result<Option<MorbotronResult>> {
        info!("Morbotron search for: {}", query);
        
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
            let variations = generate_as_phrase_variations(query);
            for variation in variations {
                info!("Trying 'as' phrase variation: {}", variation);
                if let Some(result) = self.search_with_strategy(&variation).await? {
                    info!("Found result with 'as' phrase variation");
                    return Ok(Some(result));
                }
            }
        }
        
        // 5. Try with variations for common speech patterns
        let speech_variations = generate_speech_pattern_variations(query);
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
                    word_lower.len() > 3 && !is_common_word(&word_lower)
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
        let fuzzy_variations = generate_fuzzy_variations(query);
        for variation in fuzzy_variations {
            info!("Trying fuzzy variation: {}", variation);
            if let Some(result) = self.search_with_strategy(&variation).await? {
                info!("Found result with fuzzy variation");
                return Ok(Some(result));
            }
        }
        
        // No results found with any strategy
        info!("No Morbotron results found for query: {}", query);
        Ok(None)
    }

    // Internal method to perform the actual API call with a specific search strategy
    async fn search_with_strategy(&self, query: &str) -> Result<Option<MorbotronResult>> {
        // URL encode the query
        let encoded_query = urlencoding::encode(query);
        let search_url = format!("{}?q={}", MORBOTRON_BASE_URL, encoded_query);
        
        // Make the search request
        let search_response = self.http_client.get(&search_url)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to search Morbotron: {}", e))?;
            
        if !search_response.status().is_success() {
            return Err(anyhow!("Morbotron search failed with status: {}", search_response.status()));
        }
        
        // Parse the search results
        let search_results: Vec<MorbotronSearchResult> = search_response.json()
            .await
            .map_err(|e| anyhow!("Failed to parse Morbotron search results: {}", e))?;
            
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
    async fn get_caption_for_frame(&self, episode: &str, timestamp: u64) -> Result<Option<MorbotronResult>> {
        // Get the caption for this frame
        let caption_url = format!("{}?e={}&t={}", MORBOTRON_CAPTION_URL, episode, timestamp);
        
        let caption_response = self.http_client.get(&caption_url)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to get caption from Morbotron: {}", e))?;
            
        if !caption_response.status().is_success() {
            return Err(anyhow!("Morbotron caption request failed with status: {}", caption_response.status()));
        }
        
        // Parse the caption result as a generic JSON Value first
        let caption_result: serde_json::Value = caption_response.json()
            .await
            .map_err(|e| anyhow!("Failed to parse Morbotron caption result: {}", e))?;
            
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
        let image_url = format!("{}/{}/{}.jpg", MORBOTRON_IMAGE_URL, episode, timestamp);
        
        Ok(Some(MorbotronResult {
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

// Format a Morbotron result for display
fn format_morbotron_result(result: &MorbotronResult) -> String {
    format!(
        "**S{:02}E{:02} - {}**\n{}\n\n{}",
        result.season, 
        result.episode_number, 
        result.episode_title,
        result.image_url,
        format_caption(&result.caption)
    )
}

// This function will be called from main.rs to handle the !morbotron command
pub async fn handle_morbotron_command(
    http: &Http, 
    msg: &Message, 
    search_term: Option<String>,
    morbotron_client: &MorbotronClient,
    gemini_client: Option<&GeminiClient>
) -> Result<()> {
    // If no search term is provided, get a random screenshot
    if search_term.is_none() {
        info!("Morbotron request for random screenshot");
        
        // Send a "searching" message
        let searching_msg = match msg.channel_id.say(http, "Finding a random Futurama moment...").await {
            Ok(msg) => Some(msg),
            Err(e) => {
                error!("Error sending searching message: {:?}", e);
                None
            }
        };
        
        // Get a random screenshot
        match morbotron_client.random().await {
            Ok(Some(result)) => {
                // Format the response
                let response = format_morbotron_result(&result);
                
                // Edit the searching message if we have one, otherwise send a new message
                if let Some(mut search_msg) = searching_msg {
                    if let Err(e) = search_msg.edit(http, serenity::builder::EditMessage::new().content(&response)).await {
                        error!("Error editing searching message: {:?}", e);
                        // Try sending a new message if editing fails
                        if let Err(e) = msg.channel_id.say(http, &response).await {
                            error!("Error sending Morbotron result: {:?}", e);
                        }
                    }
                } else {
                    if let Err(e) = msg.channel_id.say(http, &response).await {
                        error!("Error sending Morbotron result: {:?}", e);
                    }
                }
            },
            Ok(None) => {
                let error_msg = "Couldn't find any Futurama screenshots. Oh my, yes!";
                
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
                let error_msg = format!("Error finding a random Futurama screenshot: {}", e);
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
    info!("Morbotron request with search term: {}", term);
    
    // Send a "searching" message
    let searching_msg = match msg.channel_id.say(http, format!("Searching for Futurama scene: \"{}\"...", term)).await {
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
        let enhanced_search = EnhancedMorbotronSearch::new(gemini.clone(), morbotron_client.clone(), google_client);
        enhanced_search.search(&term).await
    } else {
        info!("Using regular search (Gemini API not available)");
        morbotron_client.search(&term).await
    };
    
    // Process the search result
    match search_result {
        Ok(Some(result)) => {
            // Format the response
            let response = format_morbotron_result(&result);
            
            // Edit the searching message if we have one, otherwise send a new message
            if let Some(mut search_msg) = searching_msg {
                if let Err(e) = search_msg.edit(http, serenity::builder::EditMessage::new().content(&response)).await {
                    error!("Error editing searching message: {:?}", e);
                    // Try sending a new message if editing fails
                    if let Err(e) = msg.channel_id.say(http, &response).await {
                        error!("Error sending Morbotron result: {:?}", e);
                    }
                }
            } else {
                if let Err(e) = msg.channel_id.say(http, &response).await {
                    error!("Error sending Morbotron result: {:?}", e);
                }
            }
        },
        Ok(None) => {
            let error_msg = format!("No Futurama scenes found for '{}'. Try a different phrase or wording.", term);
            
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
            let error_msg = format!("Error searching for Futurama scene: {}", e);
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
// Helper function to check if a word is a common word that should be ignored in some contexts
fn is_common_word(word: &str) -> bool {
    const COMMON_WORDS: &[&str] = &[
        "the", "and", "that", "this", "with", "for", "was", "not", 
        "you", "have", "are", "they", "what", "from", "but", "its",
        "his", "her", "their", "your", "our", "who", "which", "when",
        "where", "why", "how", "all", "any", "some", "many", "much",
        "more", "most", "other", "such", "than", "then", "too", "very",
        "just", "now", "also", "into", "only", "over", "under", "same",
        "about", "after", "before", "between", "during", "through", "above",
        "below", "down", "off", "out", "since", "upon", "while", "within",
        "without", "across", "along", "among", "around", "behind", "beside",
        "beyond", "near", "toward", "against", "despite", "except", "like",
        "until", "because", "although", "unless", "whereas", "whether"
    ];
    
    COMMON_WORDS.contains(&word)
}

// Generate variations for "as X as Y" phrases
fn generate_as_phrase_variations(query: &str) -> Vec<String> {
    let mut variations = Vec::new();
    
    // For phrases like "as safe as they said"
    if query.contains(" as ") {
        let words: Vec<&str> = query.split_whitespace().collect();
        
        // Find all "as" positions
        let as_positions: Vec<usize> = words.iter()
            .enumerate()
            .filter(|(_, &word)| word.to_lowercase() == "as")
            .map(|(i, _)| i)
            .collect();
            
        // If we have at least two "as" words
        if as_positions.len() >= 2 {
            for i in 0..as_positions.len() - 1 {
                let pos1 = as_positions[i];
                let pos2 = as_positions[i + 1];
                
                // If they're part of an "as X as Y" pattern
                if pos2 > pos1 + 1 {
                    // Try just the phrase between the "as" words
                    let middle_phrase: Vec<&str> = words[(pos1 + 1)..pos2].to_vec();
                    variations.push(middle_phrase.join(" "));
                    
                    // Try the phrase with the second "as"
                    let extended_phrase: Vec<&str> = words[(pos1 + 1)..=pos2].to_vec();
                    variations.push(extended_phrase.join(" "));
                    
                    // Try the phrase after the second "as"
                    if pos2 < words.len() - 1 {
                        let after_phrase: Vec<&str> = words[(pos2 + 1)..].to_vec();
                        variations.push(after_phrase.join(" "));
                    }
                    
                    // Try the full "as X as Y" phrase
                    let full_phrase: Vec<&str> = words[pos1..].to_vec();
                    variations.push(full_phrase.join(" "));
                }
            }
        }
    }
    
    variations
}

// Generate variations for common speech patterns
fn generate_speech_pattern_variations(query: &str) -> Vec<String> {
    let mut variations = Vec::new();
    let words: Vec<&str> = query.split_whitespace().collect();
    
    // For phrases with "that" or "which" - try removing them
    if query.contains(" that ") || query.contains(" which ") {
        let filtered_words: Vec<&str> = words.iter()
            .filter(|&&word| word.to_lowercase() != "that" && word.to_lowercase() != "which")
            .copied()
            .collect();
        variations.push(filtered_words.join(" "));
    }
    
    // For phrases with "isn't" or "wasn't" - try the expanded form
    if query.contains("isn't") {
        variations.push(query.replace("isn't", "is not"));
    }
    if query.contains("wasn't") {
        variations.push(query.replace("wasn't", "was not"));
    }
    
    // For phrases with "they said" or "they say" - try removing them
    if query.contains(" they said") || query.contains(" they say") {
        let without_they_said = query
            .replace(" they said", "")
            .replace(" they say", "");
        variations.push(without_they_said);
    }
    
    // For phrases with brackets or parentheses - try without them
    if query.contains('[') || query.contains(']') || 
       query.contains('(') || query.contains(')') {
        let without_brackets = query
            .replace('[', "")
            .replace(']', "")
            .replace('(', "")
            .replace(')', "");
        variations.push(without_brackets);
        
        // Also try extracting just what's inside brackets
        if let Some(start) = query.find('[') {
            if let Some(end) = query.find(']') {
                if end > start {
                    let inside_brackets = &query[(start + 1)..end];
                    variations.push(inside_brackets.to_string());
                }
            }
        }
        
        if let Some(start) = query.find('(') {
            if let Some(end) = query.find(')') {
                if end > start {
                    let inside_parens = &query[(start + 1)..end];
                    variations.push(inside_parens.to_string());
                }
            }
        }
    }
    
    // For phrases with "you know" - try removing it
    if query.contains("you know") {
        variations.push(query.replace("you know", ""));
    }
    
    variations
}

// Generate fuzzy variations of the query
fn generate_fuzzy_variations(query: &str) -> Vec<String> {
    let mut variations = Vec::new();
    let words: Vec<&str> = query.split_whitespace().collect();
    
    // Skip very short queries
    if words.len() <= 1 {
        return variations;
    }
    
    // Try with only significant words (longer than 3 chars and not common)
    let significant_words: Vec<&str> = words.iter()
        .filter(|&&word| {
            let word_lower = word.to_lowercase();
            word_lower.len() > 3 && !is_common_word(&word_lower)
        })
        .copied()
        .collect();
        
    if !significant_words.is_empty() {
        variations.push(significant_words.join(" "));
    }
    
    // Try with different word orders for key phrases
    if words.len() >= 3 {
        // For each triplet of words, try different permutations
        for i in 0..words.len() - 2 {
            let w1 = words[i];
            let w2 = words[i + 1];
            let w3 = words[i + 2];
            
            // Original order: w1 w2 w3
            // Try: w1 w3 w2
            variations.push(format!("{} {} {}", w1, w3, w2));
            
            // Try: w2 w1 w3
            variations.push(format!("{} {} {}", w2, w1, w3));
            
            // Try: w3 w1 w2
            variations.push(format!("{} {} {}", w3, w1, w2));
        }
    }
    
    // For phrases with "not as X as Y" - try "not X as Y"
    if query.contains("not as") && query.contains(" as ") {
        let not_as_variation = query
            .replace("not as", "not")
            .replace(" as ", " ");
        variations.push(not_as_variation);
    }
    
    variations
}
