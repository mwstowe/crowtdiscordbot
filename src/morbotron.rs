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
use crate::text_formatting;
use crate::screenshot_search_common;

// API endpoints
const MORBOTRON_BASE_URL: &str = "https://morbotron.com/api/search";
const MORBOTRON_CAPTION_URL: &str = "https://morbotron.com/api/caption";
const MORBOTRON_IMAGE_URL: &str = "https://morbotron.com/img";
const MORBOTRON_RANDOM_URL: &str = "https://morbotron.com/api/random";

// Common search terms for random screenshots when no query is provided
const RANDOM_SEARCH_TERMS: &[&str] = &[
    "good news everyone", "bite my shiny metal", "robot", "bender", "fry", "leela",
    "professor", "zoidberg", "amy", "hermes", "zapp", "nibbler", "hypnotoad",
    "shut up and take my money", "i don't want to live on this planet anymore",
    "futurama", "planet express", "why not zoidberg", "to shreds you say",
    "death by snu snu", "blackjack and hookers", "i'm back baby", "woop woop woop",
    "oh my yes", "robot devil", "kill all humans", "nixon", "agnew", "morbo",
    "all glory to the hypnotoad", "slurm", "suicide booth", "what if", "technically correct",
    "i'm 40% something", "scruffy", "kif", "brannigan's law", "neutral", "robot mafia",
    "flexo", "roberto", "calculon", "lrrr", "omicron persei 8", "robot hell",
    "anthology of interest", "farnsworth", "mom", "universe b", "universe 1", "what if machine",
    "new new york", "old new york", "robot", "human", "mutant", "sewer", "space",
    "future", "year 3000", "delivery", "package", "ship", "planet", "alien", "robot"
];

// Morbotron search result structure
#[derive(Deserialize, Debug)]
struct MorbotronSearchResult {
    #[serde(rename = "Id")]
    id: u64,
    #[serde(rename = "Episode")]
    episode: String,
    #[serde(rename = "Timestamp")]
    timestamp: u64,
}

// Morbotron caption result structure
#[derive(Deserialize, Debug)]
struct MorbotronCaptionResult {
    Subtitles: Vec<MorbotronSubtitle>,
    Episode: MorbotronEpisode,
}

#[derive(Deserialize, Debug)]
struct MorbotronSubtitle {
    Content: String,
    StartTimestamp: u64,
    EndTimestamp: u64,
}

#[derive(Deserialize, Debug)]
struct MorbotronEpisode {
    Title: String,
    Season: u32,
    Episode: u32,
}

// Morbotron result structure for returning to the caller
#[derive(Debug, Clone)]
pub struct MorbotronResult {
    pub episode: String,
    pub season: u32,
    pub episode_number: u32,
    pub episode_title: String,
    pub timestamp: String,
    pub image_url: String,
    pub caption: String,
}

// Morbotron client for searching and retrieving captions
#[derive(Clone)]
pub struct MorbotronClient {
    http_client: HttpClient,
}

impl MorbotronClient {
    // Create a new Morbotron client
    pub fn new() -> Self {
        let http_client = HttpClient::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
            
        MorbotronClient {
            http_client,
        }
    }
    
    // Get a random screenshot
    pub async fn random(&self) -> Result<Option<MorbotronResult>> {
        // Choose a random search term
        let random_term = RANDOM_SEARCH_TERMS.choose(&mut rand::thread_rng())
            .ok_or_else(|| anyhow!("Failed to choose random search term"))?;
            
        info!("Using random search term: {}", random_term);
        self.search(random_term).await
    }
    
    // Search for a screenshot matching the query
    pub async fn search(&self, query: &str) -> Result<Option<MorbotronResult>> {
        info!("Morbotron search for: {}", query);
        
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
        
        // If all else fails, return None
        info!("No results found for query: {}", query);
        Ok(None)
    }

    // Internal method to perform the actual API call with a specific search strategy
    async fn search_with_strategy(&self, query: &str) -> Result<Option<MorbotronResult>> {
        // URL encode the query
        let encoded_query = urlencoding::encode(query);
        let search_url = format!("{}?q={}", MORBOTRON_BASE_URL, encoded_query);
        
        info!("Sending request to Morbotron API: {}", search_url);
        
        // Make the search request
        let search_response = self.http_client.get(&search_url)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to search Morbotron: {}", e))?;
            
        let status = search_response.status();
        info!("Morbotron API response status: {}", status);
        
        if !status.is_success() {
            return Err(anyhow!("Morbotron search failed with status: {}", status));
        }
        
        // Get the response body as text first
        let response_body = search_response.text().await
            .map_err(|e| anyhow!("Failed to get Morbotron response body: {}", e))?;
        
        info!("Morbotron API response body: {}", response_body);
        
        // Parse the search results
        let search_results: Vec<MorbotronSearchResult> = match serde_json::from_str::<Vec<MorbotronSearchResult>>(&response_body) {
            Ok(results) => {
                info!("Successfully parsed Morbotron search results: {} results", results.len());
                results
            },
            Err(e) => {
                error!("Failed to parse Morbotron search results: {}. Response body: {}", e, response_body);
                return Err(anyhow!("Failed to parse Morbotron search results: {}", e));
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
    async fn get_caption_for_frame(&self, episode: &str, timestamp: u64) -> Result<Option<MorbotronResult>> {
        // Build the caption URL
        let caption_url = format!("{}/{}/{}", MORBOTRON_CAPTION_URL, episode, timestamp);
        
        // Make the caption request
        let caption_response = self.http_client.get(&caption_url)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to get Morbotron caption: {}", e))?;
            
        if !caption_response.status().is_success() {
            return Err(anyhow!("Morbotron caption request failed with status: {}", caption_response.status()));
        }
        
        // Parse the caption result
        let caption_result: MorbotronCaptionResult = caption_response.json()
            .await
            .map_err(|e| anyhow!("Failed to parse Morbotron caption: {}", e))?;
            
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
        let image_url = format!("{}/{}/{}.jpg", MORBOTRON_IMAGE_URL, episode, timestamp);
        
        // Extract episode information
        let episode_title = caption_result.Episode.Title.clone();
        let season = caption_result.Episode.Season;
        let episode_number = caption_result.Episode.Episode;
        
        // Return the result
        Ok(Some(MorbotronResult {
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
    text_formatting::format_caption(caption, text_formatting::FUTURAMA_PROPER_NOUNS)
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
    args: Option<String>,
    morbotron_client: &MorbotronClient,
    gemini_client: Option<&GeminiClient>
) -> Result<()> {
    // Parse arguments to support filtering by season/episode
    let (search_term, season_filter, episode_filter) = if let Some(args_str) = args {
        parse_morbotron_args(&args_str)
    } else {
        (None, None, None)
    };
    
    // Show a "searching" message that we'll edit later with the result
    let searching_msg = if let Ok(sent_msg) = msg.channel_id.say(http, "🔍 Searching Futurama quotes...").await {
        Some(sent_msg)
    } else {
        None
    };
    
    // Determine whether to use enhanced search or regular search
    let mut search_result = if let Some(term) = &search_term {
        // Always use regular search directly
        info!("Using regular search directly");
        morbotron_client.search(term).await
    } else {
        // If no search term but we have filters, get a random screenshot
        morbotron_client.random().await
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
            let error_msg = "Couldn't find any Futurama screenshots. Good news, everyone! I'm a failure!";
            
            // Edit the searching message if we have one, otherwise send a new message
            if let Some(mut search_msg) = searching_msg {
                if let Err(e) = search_msg.edit(http, serenity::builder::EditMessage::new().content(error_msg)).await {
                    error!("Error editing searching message: {:?}", e);
                    // Try sending a new message if editing fails
                    if let Err(e) = msg.channel_id.say(http, error_msg).await {
                        error!("Error sending Morbotron error message: {:?}", e);
                    }
                }
            } else {
                if let Err(e) = msg.channel_id.say(http, error_msg).await {
                    error!("Error sending Morbotron error message: {:?}", e);
                }
            }
        },
        Err(e) => {
            // Create a user-friendly error message
            let user_error_msg = "Couldn't find any Futurama screenshots. Good news, everyone! I'm a failure!";
            
            // Log the detailed error for debugging
            error!("Error searching Morbotron: {}", e);
            
            // Edit the searching message if we have one, otherwise send a new message
            if let Some(mut search_msg) = searching_msg {
                if let Err(e) = search_msg.edit(http, serenity::builder::EditMessage::new().content(user_error_msg)).await {
                    error!("Error editing searching message: {:?}", e);
                    // Try sending a new message if editing fails
                    if let Err(e) = msg.channel_id.say(http, user_error_msg).await {
                        error!("Error sending Morbotron error message: {:?}", e);
                    }
                }
            } else {
                if let Err(e) = msg.channel_id.say(http, user_error_msg).await {
                    error!("Error sending Morbotron error message: {:?}", e);
                }
            }
        }
    }
    
    Ok(())
}

// Parse arguments for the !morbotron command
// Format: !morbotron [search term] [-s season] [-e episode]
fn parse_morbotron_args(args: &str) -> (Option<String>, Option<u32>, Option<u32>) {
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
