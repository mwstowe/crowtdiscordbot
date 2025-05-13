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

#[derive(Debug, Deserialize)]
struct FrinkiacSearchResult {
    #[serde(rename = "Id")]
    id: Option<u64>,
    #[serde(rename = "Episode")]
    episode: String,
    #[serde(rename = "Timestamp")]
    timestamp: u64,
}

#[derive(Debug, Deserialize)]
struct FrinkiacCaptionResult {
    #[serde(rename = "Episode")]
    episode: Option<FrinkiacEpisode>,
    #[serde(rename = "Subtitles")]
    subtitles: Vec<FrinkiacSubtitle>,
    #[serde(rename = "Framerate")]
    framerate: Option<f64>,
}

#[derive(Debug, Deserialize)]
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
    async fn get_caption_for_frame(&self, episode: &str, timestamp: u64) -> Result<Option<FrinkiacResult>> {
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
            info!("No Frinkiac results found for query: {}", query);
            return Ok(None);
        }
        
        // Take the first result
        let first_result = &search_results[0];
        let episode = &first_result.episode;
        let timestamp = first_result.timestamp;
        
        // Get the caption for this frame
        self.get_caption_for_frame(episode, timestamp).await
    }
}

// Format a Frinkiac result for display
fn format_frinkiac_result(result: &FrinkiacResult) -> String {
    format!(
        "**S{:02}E{:02} - {}**\n{}\n\n\"{}\"",
        result.season, 
        result.episode_number, 
        result.episode_title,
        result.image_url,
        result.caption
    )
}

// This function will be called from main.rs to handle the !frinkiac command
pub async fn handle_frinkiac_command(
    http: &Http, 
    msg: &Message, 
    search_term: Option<String>,
    frinkiac_client: &FrinkiacClient
) -> Result<()> {
    // If no search term is provided, get a random screenshot
    if search_term.is_none() {
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
    
    // If a search term is provided, search for it
    let term = search_term.unwrap();
    info!("Frinkiac request with search term: {}", term);
    
    // Send a "searching" message
    let searching_msg = match msg.channel_id.say(http, format!("Searching for: {}...", term)).await {
        Ok(msg) => Some(msg),
        Err(e) => {
            error!("Error sending searching message: {:?}", e);
            None
        }
    };
    
    // Perform the search
    match frinkiac_client.search(&term).await {
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
            let error_msg = format!("No Frinkiac results found for '{}'.", term);
            
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
            let error_msg = format!("Error performing Frinkiac search: {}", e);
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
