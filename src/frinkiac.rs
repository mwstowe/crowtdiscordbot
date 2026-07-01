use crate::gemini_api::GeminiClient;
use crate::text_formatting;
use anyhow::{anyhow, Result};
use rand::seq::IndexedRandom;
use reqwest::Client as HttpClient;
use serenity::all::Http;

use serenity::builder::CreateMessage;
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
    pub start_timestamp: u64,
    pub end_timestamp: u64,
    pub subtitles: Vec<TimedSubtitle>,
    pub gif_url: Option<String>,
}

impl FrinkiacClient {
    pub fn new() -> Self {
        info!("Creating Frinkiac client");

        // Create HTTP client with reasonable timeouts
        let http_client = HttpClient::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            http_client,
            last_query: std::sync::RwLock::new(None),
            current_index: std::sync::RwLock::new(0),
        }
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
            let mut rng = rand::rng();
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

        // Deduplicate results by episode (API returns many frames from same scene)
        let mut seen_episodes = std::collections::HashSet::new();
        let unique_results: Vec<&serde_json::Value> = search_results
            .iter()
            .filter(|r| {
                let ep = r.get("Episode").and_then(|v| v.as_str()).unwrap_or("");
                seen_episodes.insert(ep.to_string())
            })
            .collect();

        if unique_results.is_empty() {
            return Ok(None);
        }

        // Pick the next result, rotating through results for repeated queries
        let index = {
            let mut last_q = self.last_query.write().unwrap();
            let mut idx = self.current_index.write().unwrap();
            if last_q.as_deref() == Some(query) {
                *idx = (*idx + 1) % unique_results.len();
            } else {
                *last_q = Some(query.to_string());
                *idx = 0;
            }
            *idx
        };

        let result = unique_results[index];

        // Extract the episode and timestamp
        let episode = result
            .get("Episode")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing Episode in search result"))?;

        let timestamp = result
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

                let timed_subs: Vec<TimedSubtitle> = subtitles
                    .iter()
                    .filter_map(|s| {
                        Some(TimedSubtitle {
                            text: s.get("Content")?.as_str()?.to_string(),
                            start: s.get("StartTimestamp")?.as_u64()?,
                            end: s.get("EndTimestamp")?.as_u64()?,
                        })
                    })
                    .collect();

                // Extract subtitle time range
                let start_ts = timed_subs.first().map(|s| s.start).unwrap_or(timestamp);
                let end_ts = timed_subs.last().map(|s| s.end).unwrap_or(timestamp + 4000);

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
                    start_timestamp: start_ts,
                    end_timestamp: end_ts,
                    subtitles: timed_subs,
                    gif_url: None,
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

        let timed_subs: Vec<TimedSubtitle> = subtitles
            .iter()
            .filter_map(|s| {
                Some(TimedSubtitle {
                    text: s.get("Content")?.as_str()?.to_string(),
                    start: s.get("StartTimestamp")?.as_u64()?,
                    end: s.get("EndTimestamp")?.as_u64()?,
                })
            })
            .collect();

        // Extract subtitle time range
        let start_ts = timed_subs.first().map(|s| s.start).unwrap_or(timestamp);
        let end_ts = timed_subs.last().map(|s| s.end).unwrap_or(timestamp + 4000);

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
            start_timestamp: start_ts,
            end_timestamp: end_ts,
            subtitles: timed_subs,
            gif_url: None,
        }))
    }

    /// Expand subtitles to sentence boundaries by fetching adjacent captions.
    /// If the first subtitle starts mid-sentence, fetches earlier context.
    /// If the last subtitle ends mid-sentence, fetches later context.
    pub async fn expand_to_sentence_boundaries(&self, result: &mut FrinkiacResult) {
        let episode = result._episode.clone();

        // Check if first subtitle starts mid-sentence (lowercase first char)
        let first_starts_mid = result
            .subtitles
            .first()
            .is_some_and(|s| s.text.chars().next().is_some_and(|c| c.is_lowercase()));
        let first_start = result.subtitles.first().map(|s| s.start).unwrap_or(0);

        if first_starts_mid {
            let earlier_ts = first_start.saturating_sub(2000);
            let expanded =
                if let Ok(Some(earlier)) = self.get_caption_for_frame(&episode, earlier_ts).await {
                    let mut to_prepend = Vec::new();
                    for sub in earlier.subtitles.iter().rev() {
                        if sub.end <= first_start {
                            to_prepend.push(sub.clone());
                            if sub.text.chars().next().is_some_and(|c| c.is_uppercase()) {
                                break;
                            }
                        }
                    }
                    to_prepend.reverse();
                    if let Some(first_new) = to_prepend.first() {
                        result.start_timestamp = first_new.start;
                        for (i, sub) in to_prepend.into_iter().enumerate() {
                            result.subtitles.insert(i, sub);
                        }
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };

            // If we couldn't find the sentence start, drop the dangling fragment
            if !expanded && !result.subtitles.is_empty() {
                let removed = result.subtitles.remove(0);
                if let Some(new_first) = result.subtitles.first() {
                    result.start_timestamp = new_first.start;
                }
                info!("Dropped dangling subtitle fragment: {:?}", removed.text);
            }
        }

        // Check if last subtitle ends mid-sentence
        let last_ends_mid = result.subtitles.last().is_some_and(|s| {
            s.text.ends_with(',')
                || (!s.text.ends_with('.')
                    && !s.text.ends_with('!')
                    && !s.text.ends_with('?')
                    && !s.text.ends_with('"'))
        });
        let last_start = result.subtitles.last().map(|s| s.start).unwrap_or(0);
        let last_end = result.subtitles.last().map(|s| s.end).unwrap_or(0);

        if last_ends_mid {
            let later_ts = last_start;
            if let Ok(Some(later)) = self.get_caption_for_frame(&episode, later_ts).await {
                for sub in &later.subtitles {
                    if sub.start >= last_end {
                        result.end_timestamp = sub.end;
                        result.subtitles.push(sub.clone());
                        if sub.text.ends_with('.')
                            || sub.text.ends_with('!')
                            || sub.text.ends_with('?')
                            || sub.text.ends_with('"')
                        {
                            break;
                        }
                    }
                }
            }
        }
    }
}

// Format a caption to proper sentence case and separate different speakers
fn format_caption(caption: &str) -> String {
    text_formatting::format_caption(caption, text_formatting::SIMPSONS_PROPER_NOUNS)
}

/// A subtitle with timing for GIF overlay
#[derive(Debug, Clone)]
pub struct TimedSubtitle {
    pub text: String,
    pub start: u64,
    pub end: u64,
}

/// Merge subtitle fragments that are continuations of the same sentence.
/// If a subtitle starts with lowercase or continues a sentence ending with a comma,
/// merge it with the previous one.
pub fn merge_subtitle_fragments(subs: &[TimedSubtitle]) -> Vec<TimedSubtitle> {
    if subs.is_empty() {
        return Vec::new();
    }

    let mut merged: Vec<TimedSubtitle> = Vec::new();

    for sub in subs {
        let should_merge = if let Some(prev) = merged.last() {
            // Merge if: previous ends with comma, or current starts with lowercase
            prev.text.ends_with(',') || sub.text.chars().next().is_some_and(|c| c.is_lowercase())
        } else {
            false
        };

        if should_merge {
            let prev = merged.last_mut().unwrap();
            prev.text = format!("{} {}", prev.text, sub.text);
            prev.end = sub.end;
        } else {
            merged.push(sub.clone());
        }
    }

    merged
}

// Generate a GIF from a Frinkiac result using the render API
pub async fn generate_gif(
    base_url: &str,
    episode: &str,
    start: u64,
    end: u64,
    subtitles: &[TimedSubtitle],
    font_size: u32,
    font: &str,
) -> Option<String> {
    let url = format!("{base_url}/api/render/gif/stream");

    let overlays: Vec<serde_json::Value> = subtitles
        .iter()
        .map(|sub| {
            serde_json::json!({
                "text": sub.text,
                "font": font,
                "x": 50,
                "y": 90,
                "text_align": "c",
                "all_caps": true,
                "size": font_size,
                "color": [255, 255, 255, 255],
                "start": sub.start.saturating_sub(start),
                "end": sub.end.saturating_sub(start)
            })
        })
        .collect();

    let body = serde_json::json!([{
        "episode": episode,
        "start": start,
        "end": end,
        "overlays": overlays
    }]);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .ok()?;

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        error!("GIF generation failed with status: {}", response.status());
        return None;
    }

    let text = response.text().await.ok()?;

    // Parse newline-delimited JSON, find the line with "url"
    for line in text.lines().rev() {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(gif_path) = val.get("url").and_then(|v| v.as_str()) {
                let full_url = format!("{base_url}{gif_path}");
                info!("Generated GIF: {}", full_url);
                return Some(full_url);
            }
        }
    }

    error!("GIF generation did not return a URL");
    None
}

// Format a Frinkiac result for display
pub fn format_frinkiac_result(result: &FrinkiacResult) -> String {
    let media_url = result.gif_url.as_deref().unwrap_or(&result.image_url);
    let episode_title = &result.episode_title;
    let season = result.season;
    let episode_number = result.episode_number;
    if result.gif_url.is_some() {
        // Caption is baked into the GIF
        format!("{episode_title} (Season {season}, Episode {episode_number})\n{media_url}")
    } else {
        let caption = &result.caption;
        format!(
            "{episode_title} (Season {season}, Episode {episode_number})\n{media_url}\n{caption}"
        )
    }
}

/// Send a frinkiac result as a Discord embed (GIF with clickable title) or plain text fallback
async fn send_frinkiac_result(http: &Http, msg: &Message, result: &FrinkiacResult) {
    if let Some(gif_url) = &result.gif_url {
        let title = format!(
            "{} (Season {}, Episode {})",
            result.episode_title, result.season, result.episode_number
        );

        // Download the GIF and upload as attachment for reliable display
        match reqwest::Client::new().get(gif_url.as_str()).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(bytes) = resp.bytes().await {
                    let attachment = serenity::builder::CreateAttachment::bytes(
                        bytes.to_vec(),
                        "frinkiac.gif".to_string(),
                    );
                    let message = CreateMessage::new().content(title).add_file(attachment);
                    if let Err(e) = msg.channel_id.send_message(http, message).await {
                        error!("Error sending Frinkiac GIF attachment: {:?}", e);
                    }
                }
            }
            _ => {
                // Fallback: send the URL as text
                let response = format!("{}\n{}", title, gif_url);
                if let Err(e) = msg.channel_id.say(http, &response).await {
                    error!("Error sending Frinkiac result: {:?}", e);
                }
            }
        }
    } else {
        let response = format_frinkiac_result(result);
        if let Err(e) = msg.channel_id.say(http, &response).await {
            error!("Error sending Frinkiac result: {:?}", e);
        }
    }
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

    // Show typing indicator while we search
    let _ = msg.channel_id.broadcast_typing(http).await;

    // If no search term is provided, get a random screenshot
    if search_term.is_none() && season_filter.is_none() && episode_filter.is_none() {
        info!("Frinkiac request for random screenshot");

        match frinkiac_client.random().await {
            Ok(Some(mut result)) => {
                frinkiac_client
                    .expand_to_sentence_boundaries(&mut result)
                    .await;
                result.gif_url = generate_gif(
                    "https://frinkiac.com",
                    &result._episode,
                    result.start_timestamp,
                    result.end_timestamp,
                    &merge_subtitle_fragments(&result.subtitles),
                    0,
                    "akbar",
                )
                .await;
                send_frinkiac_result(http, msg, &result).await;
            }
            Ok(None) => {
                let _ = msg
                    .channel_id
                    .say(http, "Couldn't find any Simpsons screenshots. D'oh!")
                    .await;
            }
            Err(e) => {
                error!("Error getting random Frinkiac screenshot: {:?}", e);
                let _ = msg
                    .channel_id
                    .say(http, "Error getting Frinkiac screenshot. D'oh!")
                    .await;
            }
        };

        return Ok(());
    }

    // If we have a search term, search for it
    if let Some(term) = search_term {
        info!("Frinkiac search for: {}", term);

        match frinkiac_client.search(&term).await {
            Ok(Some(mut result)) => {
                let filtered_out = season_filter.is_some_and(|s| result.season != s)
                    || episode_filter.is_some_and(|e| result.episode_number != e);

                if filtered_out {
                    let _ = msg.channel_id.say(http, format!("Couldn't find any Simpsons screenshots matching \"{term}\" in the specified season/episode.")).await;
                } else {
                    frinkiac_client
                        .expand_to_sentence_boundaries(&mut result)
                        .await;
                    result.gif_url = generate_gif(
                        "https://frinkiac.com",
                        &result._episode,
                        result.start_timestamp,
                        result.end_timestamp,
                        &merge_subtitle_fragments(&result.subtitles),
                        0,
                        "akbar",
                    )
                    .await;
                    send_frinkiac_result(http, msg, &result).await;
                }
            }
            Ok(None) => {
                let _ = msg
                    .channel_id
                    .say(
                        http,
                        format!("Couldn't find any Simpsons screenshots matching \"{term}\"."),
                    )
                    .await;
            }
            Err(e) => {
                error!("Error searching Frinkiac: {:?}", e);
                let _ = msg
                    .channel_id
                    .say(http, "Error searching Frinkiac. D'oh!")
                    .await;
            }
        }
    } else {
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
    last_query: std::sync::RwLock<Option<String>>,
    current_index: std::sync::RwLock<usize>,
}
