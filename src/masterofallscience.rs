use anyhow::{Result, anyhow};
use serenity::model::channel::Message;
use serenity::all::Http;
use tracing::{error, info};
use reqwest::Client as HttpClient;
use serde::Deserialize;
use serde_json;
use std::time::Duration;
use rand::seq::SliceRandom;
use crate::text_formatting;
use crate::gemini_api::GeminiClient;

// API endpoints
const MASTEROFALLSCIENCE_BASE_URL: &str = "https://masterofallscience.com/api/search";
const MASTEROFALLSCIENCE_CAPTION_URL: &str = "https://masterofallscience.com/api/caption";
const MASTEROFALLSCIENCE_IMAGE_URL: &str = "https://masterofallscience.com/img";
const MASTEROFALLSCIENCE_RANDOM_URL: &str = "https://masterofallscience.com/api/random";

// Common search terms for random screenshots when no query is provided
const RANDOM_SEARCH_TERMS: &[&str] = &[
    "wubba lubba", "pickle rick", "morty", "rick", "summer", "beth", "jerry", "portal gun",
    "meeseeks", "schwifty", "plumbus", "gazorpazorp", "interdimensional", "szechuan",
    "tiny rick", "bird person", "mr meeseeks", "unity", "council of ricks", "cronenberg",
    "squanch", "evil morty", "mr poopybutthole", "butter robot", "purge", "microverse",
    "dimension c-137", "eyeholes", "show me what you got", "get schwifty", "aw geez",
    "aw jeez", "oh my god", "i'm pickle rick", "wubba lubba dub dub", "grass tastes bad",
    "lick lick lick my balls", "and that's the way the news goes", "hit the sack jack",
    "uh oh somersault jump", "aids", "burger time", "rubber baby buggy bumpers",
    "it's a rick and morty thing", "nobody exists on purpose", "nobody belongs anywhere",
    "everybody's gonna die", "come watch tv", "i'm mr meeseeks look at me", "existence is pain",
    "can do", "keep your requests simple", "i just wanna die", "we all wanna die",
    "life is pain", "your failures are your own old man", "i'm mr meeseeks",
    "ooh he's tryin", "meeseeks don't usually have to exist this long", "things are getting weird",
    "what about your short game", "i'm a bit of a stickler meeseeks", "square your shoulders",
    "follow through", "that's not a real song", "there's one every season", "get schwifty",
    "i like what you got", "disqualified", "show me what you got", "head bent over",
    "raised up posterior", "oh yeah", "you gotta get schwifty", "take off your pants and your panties",
    "shit on the floor", "time to get schwifty in here", "i'm mr bulldops", "i'm mr bulldops",
    "don't analyze it", "it's working", "that's my catchphrase", "that's my catchphrase",
    "ricky ticky tavy", "grass tastes bad", "lick lick lick my balls", "uh oh somersault jump",
    "aids", "burger time", "rubber baby buggy bumpers", "and that's the way the news goes",
    "hit the sack jack", "shum shum schlippity dop", "wubba lubba dub dub", "much obliged",
    "blow me", "no no blow me", "eek barba durkle", "that's a pretty fucked up ooh la la",
    "and that's why i always say shum shum schlippity dop", "i don't give a fuck",
    "i'm pickle rick", "solenya", "i turned myself into a pickle morty", "pickle rick",
    "i'm pickle rick", "i'm tiny rick", "let me out", "let me out", "this is not a dance",
    "i'm begging for help", "i'm screaming for help", "please come let me out", "i'm dying in a vat",
    "in the garage", "tiny rick", "wubba lubba dub dub", "i'm tiny rick", "fuck yeah",
    "tiny rick", "old rick", "gotta save tiny rick", "help me morty", "help me summer",
    "i'm gonna die", "tiny rick is dying", "let me out of here", "i'm old rick trapped in a young body",
    "tiny rick", "fuck tiny rick", "i love tiny rick", "save me", "let me out", "help me",
    "i'm trapped in a vat", "i'm dying", "i'm going to die", "i'm going to die in here",
    "i'm going to die in a vat", "i'm going to die in the garage", "i'm going to die in a vat in the garage",
];

// JSON structs for MasterOfAllScience API responses
#[derive(Debug, Deserialize)]
struct MasterOfAllScienceSearchResult {
    #[serde(rename = "Id")]
    id: u64,
    #[serde(rename = "Episode")]
    episode: String,
    #[serde(rename = "Timestamp")]
    timestamp: u64,
}

#[derive(Debug, Deserialize)]
struct MasterOfAllScienceCaptionResult {
    Subtitles: Vec<MasterOfAllScienceSubtitle>,
    Episode: MasterOfAllScienceEpisode,
}

#[derive(Debug, Deserialize)]
struct MasterOfAllScienceSubtitle {
    Content: String,
    StartTimestamp: u64,
    EndTimestamp: u64,
}

#[derive(Debug, Deserialize)]
struct MasterOfAllScienceEpisode {
    Title: String,
    Season: u32,
    Episode: u32,
}

// Result struct for MasterOfAllScience searches
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

pub struct MasterOfAllScienceClient {
    http_client: HttpClient,
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
    
    // Get a random screenshot
    pub async fn random(&self) -> Result<Option<MasterOfAllScienceResult>> {
        info!("Getting random MasterOfAllScience screenshot");
        
        // Choose a random search term
        let random_term = RANDOM_SEARCH_TERMS.choose(&mut rand::thread_rng())
            .ok_or_else(|| anyhow!("Failed to choose random search term"))?;
            
        info!("Using random search term: {}", random_term);
        
        // Use search_with_strategy directly to avoid recursion
        self.search_with_strategy(random_term).await
    }
    
    // Search for a screenshot matching the query
    pub async fn search(&self, query: &str) -> Result<Option<MasterOfAllScienceResult>> {
        info!("MasterOfAllScience search for: {}", query);
        
        // Try a direct search first
        if let Some(result) = self.search_with_strategy(query).await? {
            info!("Found result with direct search");
            return Ok(Some(result));
        }
        
        // If direct search fails and it's a multi-word query, try with quotes
        if query.contains(' ') {
            if let Some(result) = self.search_with_strategy(&format!("\"{}\"", query)).await? {
                info!("Found result with quoted search");
                return Ok(Some(result));
            }
        }
        
        // If all else fails, return a random result
        info!("No results found for query: {}, returning random result", query);
        self.random().await
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
            info!("No results found for query: {}", query);
            return Ok(None);
        }
        
        // Just take the first result
        let first_result = &search_results[0];
        let episode = &first_result.episode;
        let timestamp = first_result.timestamp;
        
        // Get the caption for this frame
        self.get_caption_for_frame(episode, timestamp).await
    }
    
    // Get the caption for a specific frame
    async fn get_caption_for_frame(&self, episode: &str, timestamp: u64) -> Result<Option<MasterOfAllScienceResult>> {
        // Build the caption URL
        let caption_url = format!("{}/{}/{}", MASTEROFALLSCIENCE_CAPTION_URL, episode, timestamp);
        
        info!("Fetching caption from URL: {}", caption_url);
        
        // Make the caption request
        let caption_response = self.http_client.get(&caption_url)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to get MasterOfAllScience caption: {}", e))?;
            
        let status = caption_response.status();
        info!("Caption API response status: {}", status);
        
        if !status.is_success() {
            // If we get a 404, try with a different URL format
            if status.as_u16() == 404 {
                info!("Got 404 for caption, trying alternative URL format");
                
                // Try with a different format - some episodes might be formatted differently
                let alt_episode = if episode.contains("E") || episode.contains("S") {
                    // If it's already in SxxExx format, try with just the episode number
                    let parts: Vec<&str> = episode.split(|c| c == 'E' || c == 'S').collect();
                    if parts.len() > 1 {
                        parts[parts.len() - 1].to_string()
                    } else {
                        episode.to_string()
                    }
                } else {
                    // If it's not in SxxExx format, try with that format
                    format!("S01E{:02}", episode.parse::<u32>().unwrap_or(1))
                };
                
                let alt_caption_url = format!("{}/{}/{}", MASTEROFALLSCIENCE_CAPTION_URL, alt_episode, timestamp);
                info!("Trying alternative caption URL: {}", alt_caption_url);
                
                let alt_caption_response = self.http_client.get(&alt_caption_url)
                    .send()
                    .await
                    .map_err(|e| anyhow!("Failed to get MasterOfAllScience caption with alternative URL: {}", e))?;
                    
                if !alt_caption_response.status().is_success() {
                    return Err(anyhow!("MasterOfAllScience caption request failed with both URL formats"));
                }
                
                // Parse the caption result
                let caption_result: MasterOfAllScienceCaptionResult = alt_caption_response.json()
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
                let image_url = format!("{}/{}/{}.jpg", MASTEROFALLSCIENCE_IMAGE_URL, alt_episode, timestamp);
                
                // Extract episode information
                let episode_title = caption_result.Episode.Title.clone();
                let season = caption_result.Episode.Season;
                let episode_number = caption_result.Episode.Episode;
                
                // Return the result
                return Ok(Some(MasterOfAllScienceResult {
                    episode: alt_episode,
                    season,
                    episode_number,
                    episode_title,
                    timestamp: timestamp.to_string(),
                    image_url,
                    caption: format_caption(&caption),
                }));
            }
            
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
            caption: format_caption(&caption),
        }))
    }
}

// Format a caption to proper sentence case and separate different speakers
fn format_caption(caption: &str) -> String {
    text_formatting::format_caption(caption, text_formatting::RICK_AND_MORTY_PROPER_NOUNS)
}

// Format a MasterOfAllScience result for display
pub fn format_masterofallscience_result(result: &MasterOfAllScienceResult) -> String {
    format!(
        "{}\n{} (Season {}, Episode {})\n{}",
        result.image_url,
        result.episode_title,
        result.season,
        result.episode_number,
        result.caption
    )
}

// This function will be called from main.rs to handle the !masterofallscience command
pub async fn handle_masterofallscience_command(
    http: &Http, 
    msg: &Message, 
    args: Option<String>,
    masterofallscience_client: &MasterOfAllScienceClient,
    _gemini_client: Option<&GeminiClient>
) -> Result<()> {
    // If no search term is provided, get a random screenshot
    if args.is_none() {
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
                    // Send a new message
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
                            error!("Error sending error message: {:?}", e);
                        }
                    }
                } else {
                    // Send a new message
                    if let Err(e) = msg.channel_id.say(http, error_msg).await {
                        error!("Error sending error message: {:?}", e);
                    }
                }
            },
            Err(e) => {
                error!("Error getting random MasterOfAllScience screenshot: {:?}", e);
                
                let error_msg = "Error getting Rick and Morty screenshot. Wubba lubba dub dub!";
                
                // Edit the searching message if we have one, otherwise send a new message
                if let Some(mut search_msg) = searching_msg {
                    if let Err(e) = search_msg.edit(http, serenity::builder::EditMessage::new().content(error_msg)).await {
                        error!("Error editing searching message: {:?}", e);
                        // Try sending a new message if editing fails
                        if let Err(e) = msg.channel_id.say(http, error_msg).await {
                            error!("Error sending error message: {:?}", e);
                        }
                    }
                } else {
                    // Send a new message
                    if let Err(e) = msg.channel_id.say(http, error_msg).await {
                        error!("Error sending error message: {:?}", e);
                    }
                }
            }
        }
        
        return Ok(());
    }
    
    // If we have a search term, search for it
    if let Some(term) = args {
        info!("MasterOfAllScience search for: {}", term);
        
        // Show a "searching" message that we'll edit later with the result
        let searching_msg = match msg.channel_id.say(http, "🔍 Searching Rick and Morty quotes...").await {
            Ok(msg) => Some(msg),
            Err(e) => {
                error!("Error sending searching message: {:?}", e);
                None
            }
        };
        
        // Search for the term
        match masterofallscience_client.search(&term).await {
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
                    // Send a new message
                    if let Err(e) = msg.channel_id.say(http, &response).await {
                        error!("Error sending MasterOfAllScience result: {:?}", e);
                    }
                }
            },
            Ok(None) => {
                let error_msg = format!("Couldn't find any Rick and Morty screenshots matching \"{}\".", term);
                
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
                    // Send a new message
                    if let Err(e) = msg.channel_id.say(http, &error_msg).await {
                        error!("Error sending error message: {:?}", e);
                    }
                }
            },
            Err(e) => {
                error!("Error searching MasterOfAllScience: {:?}", e);
                
                let error_msg = "Error searching Rick and Morty quotes. Wubba lubba dub dub!";
                
                // Edit the searching message if we have one, otherwise send a new message
                if let Some(mut search_msg) = searching_msg {
                    if let Err(e) = search_msg.edit(http, serenity::builder::EditMessage::new().content(error_msg)).await {
                        error!("Error editing searching message: {:?}", e);
                        // Try sending a new message if editing fails
                        if let Err(e) = msg.channel_id.say(http, error_msg).await {
                            error!("Error sending error message: {:?}", e);
                        }
                    }
                } else {
                    // Send a new message
                    if let Err(e) = msg.channel_id.say(http, error_msg).await {
                        error!("Error sending error message: {:?}", e);
                    }
                }
            }
        }
    }
    
    Ok(())
}
