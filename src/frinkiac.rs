use crate::gemini_api::GeminiClient;
use crate::text_formatting;
use anyhow::{anyhow, Result};
use rand::seq::SliceRandom;
use reqwest::Client as HttpClient;
use serenity::all::Http;
use serenity::model::channel::Message;
use std::time::Duration;
use tracing::{error, info};

// API endpoints
const FRINKIAC_BASE_URL: &str = "https://frinkiac.com/api/search";
const FRINKIAC_CAPTION_URL: &str = "https://frinkiac.com/api/caption";
const FRINKIAC_IMAGE_URL: &str = "https://frinkiac.com/img";
const FRINKIAC_MEME_URL: &str = "https://frinkiac.com/meme";
const FRINKIAC_RANDOM_URL: &str = "https://frinkiac.com/api/random";

// Common search terms for random screenshots when no query is provided
const RANDOM_SEARCH_TERMS: &[&str] = &[
    "excellent",
    "stupid sexy",
    "cromulent",
    "embiggen",
    "steamed hams",
    "dental plan",
    "unpossible",
    "choo-choose",
    "boo-urns",
    "eat my shorts",
    "don't have a cow",
    "ay caramba",
    "ha ha",
    "d'oh",
    "spider pig",
    "worst ever",
    "perfectly cromulent",
    "glaven",
    "cowabunga",
    "sacrilicious",
    "yoink",
    "i'm in danger",
    "hi everybody",
    "hi dr nick",
    "inflammable",
    "purple monkey dishwasher",
    "cheese eating surrender monkeys",
    "i for one welcome",
    "i was saying boo-urns",
    "it's a trap",
    "i'm troy mcclure",
    "i can't believe",
    "you don't win friends",
    "i'm so hungry",
    "my eyes the goggles",
    "everything's coming up milhouse",
    "that's a paddlin",
    "stupid babies need the most attention",
    "i sleep in a racing car",
    "i sleep in a big bed",
    "you'll have to speak up",
    "i was elected to lead not to read",
    "i'm not not licking toads",
    "i'm going to allow this",
    "i've made my choice",
    "i like the way snrub thinks",
    "i've been calling her crandall",
    "i'm a brick",
    "i'm a unitard",
    "i'm idaho",
    "i'm a star wars",
    "i'm a level 5 vegan",
    "i'm a man of few words",
    "i'm better than dirt",
    "i'm directly under the earth's sun now",
    "i'm disrespectful to dirt",
    "i'm full of chocolate",
    "i'm in a rage",
    "i'm in flavor country",
    "i'm kent brockman",
    "i'm lenny",
    "i'm lisa simpson",
    "i'm not a state",
    "i'm not made of money",
    "i'm not normally a praying man",
    "i'm not popular enough",
    "i'm not saying it's aliens",
    "i'm not wearing a tie at all",
    "i'm on my way",
    "i'm proud of you",
    "i'm seeing double",
    "i'm so excited",
    "i'm sorry i'm not as smart",
    "i'm surrounded by idiots",
    "i'm the lizard queen",
    "i'm tired of these jokes",
    "i'm troy mcclure",
    "i'm with stupid",
    "i'm your worst nightmare",
];

// Result struct for Frinkiac searches
#[derive(Debug, Clone)]
pub struct FrinkiacResult {
    pub _episode: String,
    pub episode_title: String,
    pub season: u32,
    pub episode_number: u32,
    pub _timestamp: String,
    pub image_url: String,
    pub _meme_url: String,
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
            }
            Ok(None) => {
                info!("No results from direct random API, trying fallback method");
            }
            Err(e) => {
                info!(
                    "Error from direct random API: {}, trying fallback method",
                    e
                );
            }
        }

        // Fallback: Use a random search term from our list
        // Select a random term before the async operation to avoid Send issues
        let random_term = {
            let mut rng = rand::thread_rng();
            RANDOM_SEARCH_TERMS
                .choose(&mut rng)
                .ok_or_else(|| anyhow!("Failed to select random search term"))?
                .to_string() // Convert to owned String to avoid lifetime issues
        };

        info!("Using random search term: {}", random_term);

        // Use search_with_strategy directly to avoid recursion
        self.search_with_strategy(&random_term).await
    }

    // Try to get a random screenshot using Frinkiac's random API
    async fn get_random_direct(&self) -> Result<Option<FrinkiacResult>> {
        // Make the request to the random API
        let random_response = self
            .http_client
            .get(FRINKIAC_RANDOM_URL)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to get random screenshot from Frinkiac: {}", e))?;

        if !random_response.status().is_success() {
            return Err(anyhow!(
                "Frinkiac random request failed with status: {}",
                random_response.status()
            ));
        }

        // Parse the random result as a generic JSON Value first
        let random_result: serde_json::Value = random_response
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse Frinkiac random result: {}", e))?;

        // Extract the episode and timestamp
        let episode = random_result
            .get("Episode")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing Episode in random result"))?;

        let timestamp = random_result
            .get("Timestamp")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow!("Missing Timestamp in random result"))?;

        // Get the caption for this frame
        self.get_caption_for_frame(episode, timestamp).await
    }

    pub async fn search(&self, query: &str) -> Result<Option<FrinkiacResult>> {
        info!("Frinkiac search for: {}", query);

        // Try a direct search first
        if let Some(result) = self.search_with_strategy(query).await? {
            info!("Found result with direct search");
            return Ok(Some(result));
        }

        // If direct search fails and it's a multi-word query, try with quotes
        if query.contains(' ') {
            if let Some(result) = self.search_with_strategy(&format!("\"{query}\"")).await? {
                info!("Found result with quoted search");
                return Ok(Some(result));
            }
        }

        // If all else fails, return a random result
        info!(
            "No results found for query: {}, returning random result",
            query
        );
        self.get_random_direct().await
    }

    // Internal method to perform the actual API call with a specific search strategy
    async fn search_with_strategy(&self, query: &str) -> Result<Option<FrinkiacResult>> {
        // URL encode the query
        let encoded_query = urlencoding::encode(query);
        let search_url = format!("{FRINKIAC_BASE_URL}?q={encoded_query}");

        // Make the search request
        let search_response = self
            .http_client
            .get(&search_url)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to search Frinkiac: {}", e))?;

        if !search_response.status().is_success() {
            return Err(anyhow!(
                "Frinkiac search failed with status: {}",
                search_response.status()
            ));
        }

        // Parse the search results
        let search_results: Vec<serde_json::Value> = search_response
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse Frinkiac search results: {}", e))?;

        // If no results, return None
        if search_results.is_empty() {
            info!("No results found for query: {}", query);
            return Ok(None);
        }

        // Just take the first result
        let first_result = &search_results[0];

        // Extract the episode and timestamp
        let episode = first_result
            .get("Episode")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing Episode in search result"))?;

        let timestamp = first_result
            .get("Timestamp")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow!("Missing Timestamp in search result"))?;

        // Get the caption for this frame
        self.get_caption_for_frame(episode, timestamp).await
    }

    // Get caption and details for a specific frame
    pub async fn get_caption_for_frame(
        &self,
        episode: &str,
        timestamp: u64,
    ) -> Result<Option<FrinkiacResult>> {
        // Get the caption for this frame
        let caption_url = format!("{FRINKIAC_CAPTION_URL}?e={episode}&t={timestamp}");

        info!("Fetching caption from URL: {}", caption_url);

        let caption_response = self
            .http_client
            .get(&caption_url)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to get caption from Frinkiac: {}", e))?;

        let status = caption_response.status();
        info!("Caption API response status: {}", status);

        if !status.is_success() {
            // If we get a 404, try with a different URL format
            if status.as_u16() == 404 {
                info!("Got 404 for caption, trying alternative URL format");

                // Try with a different format - some episodes might be formatted differently
                let alt_episode = if episode.contains("E") || episode.contains("S") {
                    // If it's already in SxxExx format, try with just the episode number
                    let parts: Vec<&str> = episode.split(['E', 'S']).collect();
                    if parts.len() > 1 {
                        parts[parts.len() - 1].to_string()
                    } else {
                        episode.to_string()
                    }
                } else {
                    // If it's not in SxxExx format, try with that format
                    let episode_num = episode.parse::<u32>().unwrap_or(1);
                    format!("S01E{episode_num:02}")
                };

                let alt_caption_url =
                    format!("{FRINKIAC_CAPTION_URL}?e={alt_episode}&t={timestamp}");
                info!("Trying alternative caption URL: {}", alt_caption_url);

                let alt_caption_response = self
                    .http_client
                    .get(&alt_caption_url)
                    .send()
                    .await
                    .map_err(|e| anyhow!("Failed to get caption with alternative URL: {}", e))?;

                if !alt_caption_response.status().is_success() {
                    return Err(anyhow!(
                        "Frinkiac caption request failed with both URL formats"
                    ));
                }

                // Parse the caption result as a generic JSON Value
                let caption_result: serde_json::Value = alt_caption_response
                    .json()
                    .await
                    .map_err(|e| anyhow!("Failed to parse Frinkiac caption result: {}", e))?;

                // Extract episode information
                let episode_info = caption_result
                    .get("Episode")
                    .ok_or_else(|| anyhow!("Missing Episode info in caption result"))?;

                let episode_title = episode_info
                    .get("Title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown")
                    .to_string();

                let season = episode_info
                    .get("Season")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;

                let episode_number = episode_info
                    .get("EpisodeNumber")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;

                // Extract subtitles/caption
                let subtitles = caption_result
                    .get("Subtitles")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| anyhow!("Missing Subtitles in caption result"))?;

                let caption = subtitles
                    .iter()
                    .filter_map(|s| s.get("Content").and_then(|c| c.as_str()))
                    .collect::<Vec<&str>>()
                    .join(" ");

                // Format the image URL
                let image_url = format!("{FRINKIAC_IMAGE_URL}/{alt_episode}/{timestamp}.jpg");

                // Format the meme URL (for sharing)
                let meme_url = format!("{FRINKIAC_MEME_URL}/{alt_episode}/{timestamp}.jpg");

                return Ok(Some(FrinkiacResult {
                    _episode: alt_episode,
                    episode_title,
                    season,
                    episode_number,
                    _timestamp: timestamp.to_string(),
                    image_url,
                    _meme_url: meme_url,
                    caption: format_caption(&caption),
                }));
            }

            return Err(anyhow!(
                "Frinkiac caption request failed with status: {}",
                caption_response.status()
            ));
        }

        // Parse the caption result as a generic JSON Value first
        let caption_result: serde_json::Value = caption_response
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse Frinkiac caption result: {}", e))?;

        // Extract episode information
        let episode_info = caption_result
            .get("Episode")
            .ok_or_else(|| anyhow!("Missing Episode info in caption result"))?;

        let episode_title = episode_info
            .get("Title")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();

        let season = episode_info
            .get("Season")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        let episode_number = episode_info
            .get("EpisodeNumber")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        // Extract subtitles/caption
        let subtitles = caption_result
            .get("Subtitles")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow!("Missing Subtitles in caption result"))?;

        let caption = subtitles
            .iter()
            .filter_map(|s| s.get("Content").and_then(|c| c.as_str()))
            .collect::<Vec<&str>>()
            .join(" ");

        // Format the image URL
        let image_url = format!("{FRINKIAC_IMAGE_URL}/{episode}/{timestamp}.jpg");

        // Format the meme URL (for sharing)
        let meme_url = format!("{FRINKIAC_MEME_URL}/{episode}/{timestamp}.jpg");

        Ok(Some(FrinkiacResult {
            _episode: episode.to_string(),
            episode_title,
            season,
            episode_number,
            _timestamp: timestamp.to_string(),
            image_url,
            _meme_url: meme_url,
            caption: format_caption(&caption),
        }))
    }
}

// Format a caption to proper sentence case and separate different speakers
fn format_caption(caption: &str) -> String {
    text_formatting::format_caption(caption, text_formatting::SIMPSONS_PROPER_NOUNS)
}

// Format a Frinkiac result for display
pub fn format_frinkiac_result(result: &FrinkiacResult) -> String {
    let image_url = &result.image_url;
    let episode_title = &result.episode_title;
    let season = result.season;
    let episode_number = result.episode_number;
    let caption = &result.caption;
    format!(
        "{image_url}\n{episode_title} (Season {season}, Episode {episode_number})\n{caption}"
    )
}

// Parse arguments for the frinkiac command
fn parse_frinkiac_args(args: &str) -> (Option<String>, Option<u32>, Option<u32>) {
    let mut search_term = None;
    let mut season_filter = None;
    let mut episode_filter = None;

    let mut current_arg = String::new();
    let mut expecting_season = false;
    let mut expecting_episode = false;

    for part in args.split_whitespace() {
        if expecting_season {
            if let Ok(season) = part.parse::<u32>() {
                season_filter = Some(season);
            }
            expecting_season = false;
            continue;
        }

        if expecting_episode {
            if let Ok(episode) = part.parse::<u32>() {
                episode_filter = Some(episode);
            }
            expecting_episode = false;
            continue;
        }

        if part == "-s" || part == "--season" {
            expecting_season = true;
        } else if part == "-e" || part == "--episode" {
            expecting_episode = true;
        } else {
            if !current_arg.is_empty() {
                current_arg.push(' ');
            }
            current_arg.push_str(part);
        }
    }

    if !current_arg.is_empty() {
        search_term = Some(current_arg);
    }

    (search_term, season_filter, episode_filter)
}

// This function will be called from main.rs to handle the !frinkiac command
pub async fn handle_frinkiac_command(
    http: &Http,
    msg: &Message,
    args: Option<String>,
    frinkiac_client: &FrinkiacClient,
    _gemini_client: Option<&GeminiClient>,
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
        let searching_msg = match msg
            .channel_id
            .say(http, "Finding a random Simpsons moment...")
            .await
        {
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
                            error!("Error sending Frinkiac result: {:?}", e);
                        }
                    }
                } else {
                    // Send a new message
                    if let Err(e) = msg.channel_id.say(http, &response).await {
                        error!("Error sending Frinkiac result: {:?}", e);
                    }
                }
            }
            Ok(None) => {
                let error_msg = "Couldn't find any Simpsons screenshots. D'oh!";

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
                error!("Error getting random Frinkiac screenshot: {:?}", e);

                let error_msg = "Error getting Frinkiac screenshot. D'oh!";

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
    if let Some(term) = search_term {
        info!("Frinkiac search for: {}", term);

        // Show a "searching" message that we'll edit later with the result
        let searching_msg = match msg
            .channel_id
            .say(http, "🔍 Searching Simpsons quotes...")
            .await
        {
            Ok(msg) => Some(msg),
            Err(e) => {
                error!("Error sending searching message: {:?}", e);
                None
            }
        };

        // Search for the term
        match frinkiac_client.search(&term).await {
            Ok(Some(result)) => {
                // Apply filters if needed
                let mut filtered_out = false;

                // Filter by season if specified
                if let Some(season) = season_filter {
                    if result.season != season {
                        filtered_out = true;
                        info!(
                            "Result filtered out: season {} doesn't match filter {}",
                            result.season, season
                        );
                    }
                }

                // Filter by episode if specified
                if let Some(episode) = episode_filter {
                    if result.episode_number != episode {
                        filtered_out = true;
                        info!(
                            "Result filtered out: episode {} doesn't match filter {}",
                            result.episode_number, episode
                        );
                    }
                }

                // If filtered out, return appropriate message
                if filtered_out {
                    let error_msg = format!("Couldn't find any Simpsons screenshots matching \"{term}\" in the specified season/episode.");

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

                    return Ok(());
                }

                // Format the response
                let response = format_frinkiac_result(&result);

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
                            error!("Error sending Frinkiac result: {:?}", e);
                        }
                    }
                } else {
                    // Send a new message
                    if let Err(e) = msg.channel_id.say(http, &response).await {
                        error!("Error sending Frinkiac result: {:?}", e);
                    }
                }
            }
            Ok(None) => {
                let error_msg = format!(
                    "Couldn't find any Simpsons screenshots matching \"{term}\"."
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
                error!("Error searching Frinkiac: {:?}", e);

                let error_msg = "Error searching Frinkiac. D'oh!";

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
    } else {
        // If we only have filters but no search term, that's not supported
        let error_msg = "Please provide a search term with season/episode filters.";
        if let Err(e) = msg.channel_id.say(http, error_msg).await {
            error!("Error sending error message: {:?}", e);
        }
    }

    Ok(())
}

// Frinkiac client struct
pub struct FrinkiacClient {
    http_client: HttpClient,
}
