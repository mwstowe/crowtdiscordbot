use anyhow::{Result, anyhow};
use serenity::model::channel::Message;
use serenity::all::Http;
use tracing::{error, info};
use reqwest::Client as HttpClient;
use serde::Deserialize;
use serde_json;
use std::time::Duration;
use rand::seq::SliceRandom;

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
            info!("No Morbotron results found for query: {}", query);
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

// Format a Morbotron result for display
fn format_morbotron_result(result: &MorbotronResult) -> String {
    format!(
        "**S{:02}E{:02} - {}**\n{}\n\n\"{}\"",
        result.season, 
        result.episode_number, 
        result.episode_title,
        result.image_url,
        result.caption
    )
}

// This function will be called from main.rs to handle the !morbotron command
pub async fn handle_morbotron_command(
    http: &Http, 
    msg: &Message, 
    search_term: Option<String>,
    morbotron_client: &MorbotronClient
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
    let searching_msg = match msg.channel_id.say(http, format!("Searching for: {}...", term)).await {
        Ok(msg) => Some(msg),
        Err(e) => {
            error!("Error sending searching message: {:?}", e);
            None
        }
    };
    
    // Perform the search
    match morbotron_client.search(&term).await {
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
            let error_msg = format!("No Morbotron results found for '{}'.", term);
            
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
            let error_msg = format!("Error performing Morbotron search: {}", e);
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
