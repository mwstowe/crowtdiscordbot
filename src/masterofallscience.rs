use anyhow::{Result, anyhow};
use serenity::model::channel::Message;
use serenity::all::Http;
use tracing::{error, info};
use reqwest::Client as HttpClient;
use serde::Deserialize;
use serde_json;
use std::time::Duration;
use rand::seq::SliceRandom;
use std::future::Future;
use std::pin::Pin;
use std::sync::RwLock;
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
#[derive(Debug, Deserialize, Clone)]
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
    Episode: MasterOfAllScienceEpisode,
    Frame: MasterOfAllScienceFrame,
    Subtitles: Vec<MasterOfAllScienceSubtitle>,
    Nearby: Vec<MasterOfAllScienceNearbyFrame>,
}

#[derive(Debug, Deserialize)]
struct MasterOfAllScienceFrame {
    Id: u64,
    Episode: String,
    Timestamp: u64,
}

#[derive(Debug, Deserialize)]
struct MasterOfAllScienceNearbyFrame {
    Id: u64,
    Episode: String,
    Timestamp: u64,
}

#[derive(Debug, Deserialize)]
struct MasterOfAllScienceSubtitle {
    Id: u64,
    RepresentativeTimestamp: u64,
    Episode: String,
    StartTimestamp: u64,
    EndTimestamp: u64,
    Content: String,
    Language: String,
}

#[derive(Debug, Deserialize)]
struct MasterOfAllScienceEpisode {
    Id: u64,
    Key: String,
    Season: u32,
    EpisodeNumber: u32,
    Title: String,
    Director: String,
    Writer: String,
    OriginalAirDate: String,
    WikiLink: String,
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
    last_query: RwLock<Option<String>>,
    last_results: RwLock<Vec<MasterOfAllScienceSearchResult>>,
    current_index: RwLock<usize>,
}

impl MasterOfAllScienceClient {
    pub fn new() -> Self {
        info!("Creating MasterOfAllScience client");
        
        // Create HTTP client with reasonable timeouts
        let http_client = HttpClient::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client");
            
        Self { 
            http_client,
            last_query: RwLock::new(None),
            last_results: RwLock::new(Vec::new()),
            current_index: RwLock::new(0),
        }
    }
    
    // Get a random screenshot
    pub async fn random(&self) -> Result<Option<MasterOfAllScienceResult>> {
        info!("Getting random MasterOfAllScience screenshot");
        
        // Choose a random search term
        let random_term = RANDOM_SEARCH_TERMS.choose(&mut rand::thread_rng())
            .ok_or_else(|| anyhow!("Failed to choose random search term"))?;
            
        info!("Using random search term: {}", random_term);
        
        // Search for the random term
        let results = self.search_api(random_term).await?;
        if !results.is_empty() {
            // Choose a random result
            let random_result = results.choose(&mut rand::thread_rng())
                .ok_or_else(|| anyhow!("Failed to choose random result"))?;
                
            return self.get_caption_for_frame(&random_result.episode, random_result.timestamp).await;
        }
        
        // If no results, return None
        Ok(None)
    }
    
    // Search for a screenshot matching the query
    pub async fn search(&self, query: &str) -> Result<Option<MasterOfAllScienceResult>> {
        info!("MasterOfAllScience search for: {}", query);
        
        // Check if this is the same query as last time
        let same_query;
        let result_to_use;
        
        {
            let last_query = self.last_query.read().unwrap();
            let last_results = self.last_results.read().unwrap();
            let last_results_len = last_results.len();
            
            if let Some(last_q) = last_query.as_ref() {
                if last_q == query && last_results_len > 0 {
                    // Same query, increment the index to cycle through results
                    let mut index = *self.current_index.read().unwrap() + 1;
                    if index >= last_results_len {
                        index = 0; // Wrap around to the beginning
                    }
                    
                    // Update the current index
                    *self.current_index.write().unwrap() = index;
                    info!("Same query as last time, using result {} of {}", index + 1, last_results_len);
                    
                    // Get the result at the current index
                    result_to_use = Some((last_results[index].episode.clone(), last_results[index].timestamp));
                    same_query = true;
                } else {
                    same_query = false;
                    result_to_use = None;
                }
            } else {
                same_query = false;
                result_to_use = None;
            }
        }
        
        // If it's the same query, use the result we found
        if same_query && result_to_use.is_some() {
            let (episode, timestamp) = result_to_use.unwrap();
            return self.get_caption_for_frame(&episode, timestamp).await;
        }
        
        // New query, reset the index and fetch new results
        *self.current_index.write().unwrap() = 0;
        
        // Try a direct search first
        let results = self.search_api(query).await?;
        if !results.is_empty() {
            // Store the query and results for next time
            *self.last_query.write().unwrap() = Some(query.to_string());
            *self.last_results.write().unwrap() = results.clone();
            
            info!("Found {} results with direct search", results.len());
            let first_result = &results[0];
            return self.get_caption_for_frame(&first_result.episode, first_result.timestamp).await;
        }
        
        // If direct search fails and it's a multi-word query, try with quotes
        if query.contains(' ') {
            let quoted_query = format!("\"{}\"", query);
            let results = self.search_api(&quoted_query).await?;
            if !results.is_empty() {
                // Store the query and results for next time
                *self.last_query.write().unwrap() = Some(query.to_string());
                *self.last_results.write().unwrap() = results.clone();
                
                info!("Found {} results with quoted search", results.len());
                let first_result = &results[0];
                return self.get_caption_for_frame(&first_result.episode, first_result.timestamp).await;
            }
        }
        
        // If all else fails, return a random result
        info!("No results found for query: {}, returning random result", query);
        // Use Box::pin to avoid recursion issues
        Box::pin(self.random()).await
    }
    
    async fn get_caption_for_frame(&self, episode: &str, timestamp: u64) -> Result<Option<MasterOfAllScienceResult>> {
        // Use the correct URL format: /api/caption?e=S01E02&t=242434
        let caption_url = format!("{}?e={}&t={}", MASTEROFALLSCIENCE_CAPTION_URL, episode, timestamp);
        info!("Using caption URL: {}", caption_url);
        
        // Make the caption request
        let caption_response = self.http_client.get(&caption_url)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to get MasterOfAllScience caption: {}", e))?;
            
        let status = caption_response.status();
        info!("Caption API response status: {}", status);
        
        if !status.is_success() {
            // If the request failed, return a random result instead of an error
            info!("Caption request failed with status: {}, returning random result", status);
            return Box::pin(self.random()).await;
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
        let episode_number = caption_result.Episode.EpisodeNumber;
        
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
    
    // Internal method to search the API and return the raw results
    async fn search_api(&self, query: &str) -> Result<Vec<MasterOfAllScienceSearchResult>> {
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
        
        Ok(search_results)
    }
}

// Format a caption to proper sentence case and separate different speakers
fn format_caption(caption: &str) -> String {
    text_formatting::format_caption(caption, text_formatting::RICK_AND_MORTY_PROPER_NOUNS)
}

// Format a MasterOfAllScience result for display
pub fn format_masterofallscience_result(result: &MasterOfAllScienceResult) -> String {
    format!(
        "**S{:02}E{:02} - {}**\n{}\n\n{}",
        result.season, 
        result.episode_number, 
        result.episode_title,
        result.image_url,
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
