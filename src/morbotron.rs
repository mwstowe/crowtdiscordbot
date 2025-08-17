use crate::gemini_api::GeminiClient;
use crate::text_formatting;
use anyhow::{anyhow, Result};
use rand::seq::SliceRandom;
use reqwest::Client as HttpClient;
use serde::Deserialize;
use serenity::all::Http;
use serenity::model::channel::Message;
use std::sync::RwLock;
use std::time::Duration;
use tracing::{error, info};

// API endpoints
const MORBOTRON_BASE_URL: &str = "https://morbotron.com/api/search";
const MORBOTRON_CAPTION_URL: &str = "https://morbotron.com/api/caption";
const MORBOTRON_IMAGE_URL: &str = "https://morbotron.com/img";

// Common search terms for random screenshots when no query is provided
const RANDOM_SEARCH_TERMS: &[&str] = &[
    "good news everyone",
    "bite my shiny metal",
    "robot",
    "bender",
    "fry",
    "leela",
    "professor",
    "zoidberg",
    "amy",
    "hermes",
    "zapp",
    "nibbler",
    "hypnotoad",
    "shut up and take my money",
    "i don't want to live on this planet anymore",
    "futurama",
    "planet express",
    "why not zoidberg",
    "to shreds you say",
    "death by snu snu",
    "blackjack and hookers",
    "i'm back baby",
    "woop woop woop",
    "oh my yes",
    "robot devil",
    "kill all humans",
    "nixon",
    "agnew",
    "morbo",
    "all glory to the hypnotoad",
    "slurm",
    "suicide booth",
    "what if",
    "technically correct",
];

// JSON structs for Morbotron API responses
#[derive(Debug, Deserialize, Clone)]
struct MorbotronSearchResult {
    #[serde(rename = "Id")]
    _id: u64,
    #[serde(rename = "Episode")]
    episode: String,
    #[serde(rename = "Timestamp")]
    timestamp: u64,
}

#[derive(Debug, Deserialize)]
struct MorbotronCaptionResult {
    #[serde(rename = "Episode")]
    episode: MorbotronEpisode,
    #[serde(rename = "Frame")]
    _frame: MorbotronFrame,
    #[serde(rename = "Subtitles")]
    subtitles: Vec<MorbotronSubtitle>,
    #[serde(rename = "Nearby")]
    _nearby: Vec<MorbotronNearbyFrame>,
}

#[derive(Debug, Deserialize)]
struct MorbotronFrame {
    #[serde(rename = "Id")]
    _id: u64,
    #[serde(rename = "Episode")]
    _episode: String,
    #[serde(rename = "Timestamp")]
    _timestamp: u64,
}

#[derive(Debug, Deserialize)]
struct MorbotronNearbyFrame {
    #[serde(rename = "Id")]
    _id: u64,
    #[serde(rename = "Episode")]
    _episode: String,
    #[serde(rename = "Timestamp")]
    _timestamp: u64,
}

#[derive(Debug, Deserialize)]
struct MorbotronSubtitle {
    #[serde(rename = "Id")]
    _id: u64,
    #[serde(rename = "RepresentativeTimestamp")]
    _representative_timestamp: u64,
    #[serde(rename = "Episode")]
    _episode: String,
    #[serde(rename = "StartTimestamp")]
    _start_timestamp: u64,
    #[serde(rename = "EndTimestamp")]
    _end_timestamp: u64,
    #[serde(rename = "Content")]
    content: String,
    #[serde(rename = "Language")]
    _language: String,
}

#[derive(Debug, Deserialize)]
struct MorbotronEpisode {
    #[serde(rename = "Id")]
    _id: u64,
    #[serde(rename = "Key")]
    _key: String,
    #[serde(rename = "Season")]
    season: u32,
    #[serde(rename = "EpisodeNumber")]
    episode_number: u32,
    #[serde(rename = "Title")]
    title: String,
    #[serde(rename = "Director")]
    _director: String,
    #[serde(rename = "Writer")]
    _writer: String,
    #[serde(rename = "OriginalAirDate")]
    _original_air_date: String,
    #[serde(rename = "WikiLink")]
    _wiki_link: String,
}

// Result struct for Morbotron searches
#[derive(Debug, Clone)]
pub struct MorbotronResult {
    pub _episode: String,
    pub season: u32,
    pub episode_number: u32,
    pub episode_title: String,
    pub _timestamp: String,
    pub image_url: String,
    pub caption: String,
}

pub struct MorbotronClient {
    http_client: HttpClient,
    last_query: RwLock<Option<String>>,
    last_results: RwLock<Vec<MorbotronSearchResult>>,
    current_index: RwLock<usize>,
}

impl MorbotronClient {
    pub fn new() -> Self {
        info!("Creating Morbotron client");

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
    pub async fn random(&self) -> Result<Option<MorbotronResult>> {
        info!("Getting random Morbotron screenshot");

        // Choose a random search term
        let random_term = RANDOM_SEARCH_TERMS
            .choose(&mut rand::thread_rng())
            .ok_or_else(|| anyhow!("Failed to choose random search term"))?;

        info!("Using random search term: {}", random_term);

        // Search for the random term
        let results = self.search_api(random_term).await?;
        if !results.is_empty() {
            // Choose a random result
            let random_result = results
                .choose(&mut rand::thread_rng())
                .ok_or_else(|| anyhow!("Failed to choose random result"))?;

            return self
                .get_caption_for_frame(&random_result.episode, random_result.timestamp)
                .await;
        }

        // If no results, return None
        Ok(None)
    }

    // Search for a screenshot matching the query
    pub async fn search(&self, query: &str) -> Result<Option<MorbotronResult>> {
        info!("Morbotron search for: {}", query);

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
                    info!(
                        "Same query as last time, using result {} of {}",
                        index + 1,
                        last_results_len
                    );

                    // Get the result at the current index
                    result_to_use = Some((
                        last_results[index].episode.clone(),
                        last_results[index].timestamp,
                    ));
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
            return self
                .get_caption_for_frame(&first_result.episode, first_result.timestamp)
                .await;
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
                return self
                    .get_caption_for_frame(&first_result.episode, first_result.timestamp)
                    .await;
            }
        }

        // If all else fails, return a random result
        info!(
            "No results found for query: {}, returning random result",
            query
        );
        // Use Box::pin to avoid recursion issues
        Box::pin(self.random()).await
    }

    async fn get_caption_for_frame(
        &self,
        episode: &str,
        timestamp: u64,
    ) -> Result<Option<MorbotronResult>> {
        // Use the correct URL format: /api/caption?e=S01E02&t=242434
        let caption_url = format!("{}?e={}&t={}", MORBOTRON_CAPTION_URL, episode, timestamp);
        info!("Using caption URL: {}", caption_url);

        // Make the caption request
        let caption_response = self
            .http_client
            .get(&caption_url)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to get Morbotron caption: {}", e))?;

        let status = caption_response.status();
        info!("Caption API response status: {}", status);

        if !status.is_success() {
            // If the request failed, return a random result instead of an error
            info!(
                "Caption request failed with status: {}, returning random result",
                status
            );
            return Box::pin(self.random()).await;
        }

        // Parse the caption result
        let caption_result: MorbotronCaptionResult = caption_response
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse Morbotron caption: {}", e))?;

        // If no subtitles, return None
        if caption_result.subtitles.is_empty() {
            return Ok(None);
        }

        // Extract the caption text
        let caption = caption_result
            .subtitles
            .iter()
            .map(|s| s.content.clone())
            .collect::<Vec<String>>()
            .join("\n");

        // Build the image URL
        let image_url = format!("{}/{}/{}.jpg", MORBOTRON_IMAGE_URL, episode, timestamp);

        // Extract episode information
        let episode_title = caption_result.episode.title.clone();
        let season = caption_result.episode.season;
        let episode_number = caption_result.episode.episode_number;

        // Return the result
        Ok(Some(MorbotronResult {
            _episode: episode.to_string(),
            season,
            episode_number,
            episode_title,
            _timestamp: timestamp.to_string(),
            image_url,
            caption: format_caption(&caption),
        }))
    }

    // Internal method to search the API and return the raw results
    async fn search_api(&self, query: &str) -> Result<Vec<MorbotronSearchResult>> {
        // URL encode the query
        let encoded_query = urlencoding::encode(query);
        let search_url = format!("{}?q={}", MORBOTRON_BASE_URL, encoded_query);

        info!("Sending request to Morbotron API: {}", search_url);

        // Make the search request
        let search_response = self
            .http_client
            .get(&search_url)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to search Morbotron: {}", e))?;

        let status = search_response.status();
        info!("Morbotron API response status: {}", status);

        if !status.is_success() {
            return Err(anyhow!("Morbotron search failed with status: {}", status));
        }

        // Get the response body as text first
        let response_body = search_response
            .text()
            .await
            .map_err(|e| anyhow!("Failed to get Morbotron response body: {}", e))?;

        info!("Morbotron API response body: {}", response_body);

        // Parse the search results
        let search_results: Vec<MorbotronSearchResult> =
            match serde_json::from_str::<Vec<MorbotronSearchResult>>(&response_body) {
                Ok(results) => {
                    info!(
                        "Successfully parsed Morbotron search results: {} results",
                        results.len()
                    );
                    results
                }
                Err(e) => {
                    error!(
                        "Failed to parse Morbotron search results: {}. Response body: {}",
                        e, response_body
                    );
                    return Err(anyhow!("Failed to parse Morbotron search results: {}", e));
                }
            };

        Ok(search_results)
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
        result.caption
    )
}

// This function will be called from main.rs to handle the !morbotron command
pub async fn handle_morbotron_command(
    http: &Http,
    msg: &Message,
    args: Option<String>,
    morbotron_client: &MorbotronClient,
    _gemini_client: Option<&GeminiClient>,
) -> Result<()> {
    // If no search term is provided, get a random screenshot
    if args.is_none() {
        info!("Morbotron request for random screenshot");

        // Send a "searching" message
        let searching_msg = match msg
            .channel_id
            .say(http, "Finding a random Futurama moment...")
            .await
        {
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
                    if let Err(e) = search_msg
                        .edit(
                            http,
                            serenity::builder::EditMessage::new().content(&response),
                        )
                        .await
                    {
                        error!("Error editing searching message: {:?}", e);
                        // Try sending a new message if editing fails
                        if let Err(e) = msg.channel_id.say(http, &response).await {
                            error!("Error sending Morbotron result: {:?}", e);
                        }
                    }
                } else {
                    // Send a new message
                    if let Err(e) = msg.channel_id.say(http, &response).await {
                        error!("Error sending Morbotron result: {:?}", e);
                    }
                }
            }
            Ok(None) => {
                let error_msg = "Couldn't find any Futurama screenshots. Bite my shiny metal...";

                // Edit the searching message if we have one, otherwise send a new message
                if let Some(mut search_msg) = searching_msg {
                    if let Err(e) = search_msg
                        .edit(
                            http,
                            serenity::builder::EditMessage::new().content(error_msg),
                        )
                        .await
                    {
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
            Err(e) => {
                error!("Error getting random Morbotron screenshot: {:?}", e);

                let error_msg = "Error getting Futurama screenshot. Bite my shiny metal...";

                // Edit the searching message if we have one, otherwise send a new message
                if let Some(mut search_msg) = searching_msg {
                    if let Err(e) = search_msg
                        .edit(
                            http,
                            serenity::builder::EditMessage::new().content(error_msg),
                        )
                        .await
                    {
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
        info!("Morbotron search for: {}", term);

        // Show a "searching" message that we'll edit later with the result
        let searching_msg = match msg
            .channel_id
            .say(http, "🔍 Searching Futurama quotes...")
            .await
        {
            Ok(msg) => Some(msg),
            Err(e) => {
                error!("Error sending searching message: {:?}", e);
                None
            }
        };

        // Search for the term
        match morbotron_client.search(&term).await {
            Ok(Some(result)) => {
                // Format the response
                let response = format_morbotron_result(&result);

                // Edit the searching message if we have one, otherwise send a new message
                if let Some(mut search_msg) = searching_msg {
                    if let Err(e) = search_msg
                        .edit(
                            http,
                            serenity::builder::EditMessage::new().content(&response),
                        )
                        .await
                    {
                        error!("Error editing searching message: {:?}", e);
                        // Try sending a new message if editing fails
                        if let Err(e) = msg.channel_id.say(http, &response).await {
                            error!("Error sending Morbotron result: {:?}", e);
                        }
                    }
                } else {
                    // Send a new message
                    if let Err(e) = msg.channel_id.say(http, &response).await {
                        error!("Error sending Morbotron result: {:?}", e);
                    }
                }
            }
            Ok(None) => {
                let error_msg = format!(
                    "Couldn't find any Futurama screenshots matching \"{}\".",
                    term
                );

                // Edit the searching message if we have one, otherwise send a new message
                if let Some(mut search_msg) = searching_msg {
                    if let Err(e) = search_msg
                        .edit(
                            http,
                            serenity::builder::EditMessage::new().content(&error_msg),
                        )
                        .await
                    {
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
            }
            Err(e) => {
                error!("Error searching Morbotron: {:?}", e);

                let error_msg = "Error searching Futurama quotes. Bite my shiny metal...";

                // Edit the searching message if we have one, otherwise send a new message
                if let Some(mut search_msg) = searching_msg {
                    if let Err(e) = search_msg
                        .edit(
                            http,
                            serenity::builder::EditMessage::new().content(error_msg),
                        )
                        .await
                    {
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
