use crate::gemini_api::GeminiClient;
use crate::text_formatting;
use anyhow::{anyhow, Result};
use rand::seq::IndexedRandom;
use reqwest::Client as HttpClient;
use serde::Deserialize;
use serenity::all::Http;
use serenity::builder::CreateMessage;
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
    #[serde(rename = "Content", default)]
    content: String,
}

#[derive(Debug, Deserialize)]
struct MorbotronCaptionResult {
    #[serde(rename = "Episode")]
    episode: MorbotronEpisode,
    #[serde(rename = "Subtitles")]
    subtitles: Vec<MorbotronSubtitle>,
}

#[derive(Debug, Deserialize)]
struct MorbotronSubtitle {
    #[serde(rename = "Id")]
    _id: u64,
    #[serde(rename = "RepresentativeTimestamp")]
    representative_timestamp: u64,
    #[serde(rename = "Episode")]
    _episode: String,
    #[serde(rename = "StartTimestamp")]
    start_timestamp: u64,
    #[serde(rename = "EndTimestamp")]
    end_timestamp: u64,
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
    pub start_timestamp: u64,
    pub end_timestamp: u64,
    pub subtitles: Vec<crate::frinkiac::TimedSubtitle>,
    pub gif_url: Option<String>,
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
            .choose(&mut rand::rng())
            .ok_or_else(|| anyhow!("Failed to choose random search term"))?;

        info!("Using random search term: {}", random_term);

        // Search for the random term
        let results = self.search_api(random_term).await?;
        if !results.is_empty() {
            // Choose a random result
            let random_result = results
                .choose(&mut rand::rng())
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

        // If this is a repeat of the last query, cycle through the cached ranked
        // results for variety (staying within the exact-match pool when one exists).
        // Compute the target frame while holding the locks, then drop them before
        // the await (the lock guards are not Send).
        let repeat_target: Option<(String, u64)> = {
            let last_query = self.last_query.read().unwrap_or_else(|e| e.into_inner());
            let last_results = self.last_results.read().unwrap_or_else(|e| e.into_inner());
            if last_query.as_deref() == Some(query) && !last_results.is_empty() {
                let query_lower = query.trim_matches('"').to_lowercase();
                let is_phrase = query_lower.contains(' ');
                let exact_count = if is_phrase {
                    last_results
                        .iter()
                        .filter(|r| r.content.to_lowercase().contains(&query_lower))
                        .count()
                } else {
                    last_results.len()
                };
                let pool_len = if exact_count > 0 {
                    exact_count
                } else {
                    last_results.len()
                };

                let index = {
                    let mut idx = self
                        .current_index
                        .write()
                        .unwrap_or_else(|e| e.into_inner());
                    *idx = (*idx + 1) % pool_len;
                    *idx
                };

                info!(
                    "Same query as last time, using result {} of {} (exact pool: {})",
                    index + 1,
                    last_results.len(),
                    pool_len
                );

                Some((
                    last_results[index].episode.clone(),
                    last_results[index].timestamp,
                ))
            } else {
                None
            }
        };

        if let Some((episode, timestamp)) = repeat_target {
            return self.get_caption_for_frame(&episode, timestamp).await;
        }

        // New query: reset cycling state.
        *self
            .current_index
            .write()
            .unwrap_or_else(|e| e.into_inner()) = 0;

        // Strict direct search (phrase queries require an exact substring match).
        if let Some(result) = self.ranked_search(query, true).await? {
            return Ok(Some(result));
        }

        // Strict quoted search for multi-word queries.
        if query.contains(' ') {
            if let Some(result) = self.ranked_search(&format!("\"{query}\""), true).await? {
                return Ok(Some(result));
            }
        }

        // Last resort: non-strict word-level best match (API relevance order),
        // rather than a fully random unrelated screenshot.
        info!(
            "No exact match for '{}', trying non-strict word-level best match",
            query
        );
        if let Some(result) = self.ranked_search(query, false).await? {
            return Ok(Some(result));
        }

        info!("No results found for query: {}", query);
        Ok(None)
    }

    /// Run the API search, rank/dedupe results, cache them for cycling, and return
    /// the best (first) result's caption. When `strict` is true, phrase queries
    /// with no exact substring match return None instead of a word-level hit.
    async fn ranked_search(&self, query: &str, strict: bool) -> Result<Option<MorbotronResult>> {
        let results = self.search_api(query).await?;
        if results.is_empty() {
            return Ok(None);
        }

        let query_lower = query.trim_matches('"').to_lowercase();
        let is_phrase_query = query_lower.contains(' ');

        // Dedupe by episode, keeping the first (most relevant) occurrence.
        let deduped = Self::dedupe_by_episode(results);

        let prioritized: Vec<MorbotronSearchResult> = if is_phrase_query {
            let (mut exact, rest): (Vec<_>, Vec<_>) = deduped
                .into_iter()
                .partition(|r| r.content.to_lowercase().contains(&query_lower));

            info!(
                "Phrase query '{}': {} exact matches out of {} total (deduped)",
                query_lower,
                exact.len(),
                exact.len() + rest.len()
            );

            if exact.is_empty() && strict {
                info!(
                    "Phrase query '{}' had no exact matches (strict); reporting no good match",
                    query_lower
                );
                return Ok(None);
            }

            // Rank exact matches by tightness; stable sort keeps API order on ties.
            exact.sort_by(|a, b| {
                crate::frinkiac::phrase_score_str(&b.content, &query_lower)
                    .partial_cmp(&crate::frinkiac::phrase_score_str(&a.content, &query_lower))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            if exact.is_empty() {
                // non-strict: fall back to word-level results in API order
                rest
            } else {
                exact.into_iter().chain(rest).collect()
            }
        } else {
            deduped
        };

        if prioritized.is_empty() {
            return Ok(None);
        }

        // Cache the ranked, deduped list for repeat-query cycling.
        *self.last_query.write().unwrap_or_else(|e| e.into_inner()) = Some(query.to_string());
        *self.last_results.write().unwrap_or_else(|e| e.into_inner()) = prioritized.clone();

        info!("Found {} results (ranked/deduped)", prioritized.len());
        let first = &prioritized[0];
        self.get_caption_for_frame(&first.episode, first.timestamp)
            .await
    }

    /// Deduplicate search results by episode, keeping the first occurrence of each.
    fn dedupe_by_episode(results: Vec<MorbotronSearchResult>) -> Vec<MorbotronSearchResult> {
        let mut seen = std::collections::HashSet::new();
        results
            .into_iter()
            .filter(|r| seen.insert(r.episode.clone()))
            .collect()
    }

    async fn get_caption_for_frame(
        &self,
        episode: &str,
        timestamp: u64,
    ) -> Result<Option<MorbotronResult>> {
        // Use the correct URL format: /api/caption?e=S01E02&t=242434
        let caption_url = format!("{MORBOTRON_CAPTION_URL}?e={episode}&t={timestamp}");
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

        let timed_subs: Vec<crate::frinkiac::TimedSubtitle> = caption_result
            .subtitles
            .iter()
            .map(|s| crate::frinkiac::TimedSubtitle {
                text: s.content.clone(),
                start: s.start_timestamp,
                end: s.end_timestamp,
                representative_ts: s.representative_timestamp,
            })
            .collect();

        // Build the image URL
        let image_url = format!("{MORBOTRON_IMAGE_URL}/{episode}/{timestamp}.jpg");

        // Extract episode information
        let episode_title = caption_result.episode.title.clone();
        let season = caption_result.episode.season;
        let episode_number = caption_result.episode.episode_number;

        let start_ts = timed_subs.first().map(|s| s.start).unwrap_or(timestamp);
        let end_ts = timed_subs.last().map(|s| s.end).unwrap_or(timestamp + 4000);

        // Return the result
        Ok(Some(MorbotronResult {
            _episode: episode.to_string(),
            season,
            episode_number,
            episode_title,
            _timestamp: timestamp.to_string(),
            image_url,
            caption: format_caption(&caption),
            start_timestamp: start_ts,
            end_timestamp: end_ts,
            subtitles: timed_subs,
            gif_url: None,
        }))
    }

    // Internal method to search the API and return the raw results
    async fn search_api(&self, query: &str) -> Result<Vec<MorbotronSearchResult>> {
        // URL encode the query
        let encoded_query = urlencoding::encode(query);
        let search_url = format!("{MORBOTRON_BASE_URL}?q={encoded_query}");

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

    /// Expand subtitles to sentence boundaries by stepping through adjacent
    /// caption cards using their real timestamps (shared logic with Frinkiac).
    pub async fn expand_to_sentence_boundaries(&self, result: &mut MorbotronResult) {
        use crate::frinkiac::{ends_mid_sentence, ends_sentence, starts_mid_sentence};

        let episode = result._episode.clone();

        // Bound the number of adjacent-card hops in each direction.
        const MAX_HOPS: usize = 4;

        // --- Expand backwards to the start of the sentence ---
        if result
            .subtitles
            .first()
            .is_some_and(|s| starts_mid_sentence(&s.text))
        {
            let mut hops = 0;
            loop {
                if hops >= MAX_HOPS {
                    break;
                }
                hops += 1;

                let first_start = match result.subtitles.first() {
                    Some(s) => s.start,
                    None => break,
                };
                if first_start == 0 {
                    break;
                }

                // Probe just before the current first card's start to reach the previous card.
                let probe_ts = first_start.saturating_sub(1);
                let earlier = match self.get_caption_for_frame(&episode, probe_ts).await {
                    Ok(Some(e)) => e,
                    _ => break,
                };

                let preceding: Vec<crate::frinkiac::TimedSubtitle> = earlier
                    .subtitles
                    .into_iter()
                    .filter(|s| s.end <= first_start)
                    .collect();
                if preceding.is_empty() {
                    break;
                }

                result.start_timestamp = preceding.first().map(|s| s.start).unwrap_or(first_start);
                for (i, sub) in preceding.iter().cloned().enumerate() {
                    result.subtitles.insert(i, sub);
                }

                if result
                    .subtitles
                    .first()
                    .is_some_and(|s| !starts_mid_sentence(&s.text))
                {
                    break;
                }
            }
        }

        // --- Expand forwards to the end of the sentence ---
        if result
            .subtitles
            .last()
            .is_some_and(|s| ends_mid_sentence(&s.text))
        {
            let mut hops = 0;
            loop {
                if hops >= MAX_HOPS {
                    break;
                }
                hops += 1;

                // Probe just past the current last card to reach the next one. Using the
                // max of (last_end + 1) and (representative_ts + 1) keeps the probe on a
                // valid indexed frame. NOTE: the API returns the card *containing* the
                // probe timestamp, so a single-line quote whose neighbour isn't indexed
                // near last_end simply won't expand — we accept that over guessing, and
                // never drop the existing text.
                let (last_end, probe_ts) = match result.subtitles.last() {
                    Some(s) => (
                        s.end,
                        s.end
                            .saturating_add(1)
                            .max(s.representative_ts.saturating_add(1)),
                    ),
                    None => break,
                };

                let later = match self.get_caption_for_frame(&episode, probe_ts).await {
                    Ok(Some(l)) => l,
                    _ => break,
                };

                let following: Vec<crate::frinkiac::TimedSubtitle> = later
                    .subtitles
                    .into_iter()
                    .filter(|s| s.start >= last_end)
                    .collect();
                if following.is_empty() {
                    break;
                }

                let mut reached_end = false;
                for sub in following {
                    result.end_timestamp = sub.end;
                    let terminal = ends_sentence(&sub.text);
                    result.subtitles.push(sub);
                    if terminal {
                        reached_end = true;
                        break;
                    }
                }
                if reached_end {
                    break;
                }
            }
        }
    }
}

// Format a caption to proper sentence case and separate different speakers
fn format_caption(caption: &str) -> String {
    text_formatting::format_caption(caption, text_formatting::FUTURAMA_PROPER_NOUNS)
}

// Format a Morbotron result for display
fn format_morbotron_result(result: &MorbotronResult) -> String {
    let season = result.season;
    let episode_number = result.episode_number;
    let episode_title = &result.episode_title;
    let media_url = result.gif_url.as_deref().unwrap_or(&result.image_url);
    if result.gif_url.is_some() {
        format!("**S{season:02}E{episode_number:02} - {episode_title}**\n{media_url}")
    } else {
        let caption = &result.caption;
        format!("**S{season:02}E{episode_number:02} - {episode_title}**\n{media_url}\n\n{caption}")
    }
}

async fn send_morbotron_result(
    http: &Http,
    msg: &Message,
    result: &MorbotronResult,
    http_client: &reqwest::Client,
) {
    if let Some(gif_url) = &result.gif_url {
        let title = format!(
            "S{:02}E{:02} - {}",
            result.season, result.episode_number, result.episode_title
        );

        // Download the GIF and upload as attachment for reliable display
        let mut sent = false;
        match http_client.get(gif_url.as_str()).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(bytes) = resp.bytes().await {
                    let attachment = serenity::builder::CreateAttachment::bytes(
                        bytes.to_vec(),
                        "morbotron.gif".to_string(),
                    );
                    let message = CreateMessage::new().content(&title).add_file(attachment);
                    match msg.channel_id.send_message(http, message).await {
                        Ok(_) => {
                            sent = true;
                        }
                        Err(e) => {
                            error!("Error sending Morbotron GIF attachment: {:?}", e);
                            // Fall through to URL fallback below
                        }
                    }
                }
            }
            _ => {}
        }

        // Fallback: send the GIF URL as text (handles 413 too-large errors)
        if !sent {
            let response = format!("{}\n{}", title, gif_url);
            if let Err(e) = msg.channel_id.say(http, &response).await {
                error!("Error sending Morbotron result fallback: {:?}", e);
            }
        }
    } else {
        let response = format_morbotron_result(result);
        if let Err(e) = msg.channel_id.say(http, &response).await {
            error!("Error sending Morbotron result: {:?}", e);
        }
    }
}

// This function will be called from main.rs to handle the !morbotron command
pub async fn handle_morbotron_command(
    http: &Http,
    msg: &Message,
    args: Option<String>,
    morbotron_client: &MorbotronClient,
    _gemini_client: Option<&GeminiClient>,
) -> Result<()> {
    // Show typing indicator while we search
    let _ = msg.channel_id.broadcast_typing(http).await;

    // If no search term is provided, get a random screenshot
    if args.is_none() {
        info!("Morbotron request for random screenshot");

        let response = match morbotron_client.random().await {
            Ok(Some(mut result)) => {
                morbotron_client
                    .expand_to_sentence_boundaries(&mut result)
                    .await;
                result.gif_url = crate::frinkiac::generate_gif(
                    &morbotron_client.http_client,
                    "https://morbotron.com",
                    &result._episode,
                    result.start_timestamp,
                    result.end_timestamp,
                    &crate::frinkiac::merge_subtitle_fragments(&result.subtitles),
                    0,
                    "fr",
                )
                .await;
                send_morbotron_result(http, msg, &result, &morbotron_client.http_client).await;
                return Ok(());
            }
            Ok(None) => {
                "Couldn't find any Futurama screenshots. Bite my shiny metal...".to_string()
            }
            Err(e) => {
                error!("Error getting random Morbotron screenshot: {:?}", e);
                "Error getting Futurama screenshot. Bite my shiny metal...".to_string()
            }
        };

        if let Err(e) = msg.channel_id.say(http, &response).await {
            error!("Error sending Morbotron result: {:?}", e);
        }
        return Ok(());
    }

    // If we have a search term, search for it
    if let Some(term) = args {
        info!("Morbotron search for: {}", term);

        match morbotron_client.search(&term).await {
            Ok(Some(mut result)) => {
                morbotron_client
                    .expand_to_sentence_boundaries(&mut result)
                    .await;
                result.gif_url = crate::frinkiac::generate_gif(
                    &morbotron_client.http_client,
                    "https://morbotron.com",
                    &result._episode,
                    result.start_timestamp,
                    result.end_timestamp,
                    &crate::frinkiac::merge_subtitle_fragments(&result.subtitles),
                    0,
                    "fr",
                )
                .await;
                send_morbotron_result(http, msg, &result, &morbotron_client.http_client).await;
            }
            Ok(None) => {
                let _ = msg
                    .channel_id
                    .say(
                        http,
                        format!("Couldn't find any Futurama screenshots matching \"{term}\"."),
                    )
                    .await;
            }
            Err(e) => {
                error!("Error searching Morbotron: {:?}", e);
                let _ = msg
                    .channel_id
                    .say(
                        http,
                        "Error searching Futurama quotes. Bite my shiny metal...",
                    )
                    .await;
            }
        }
    }

    Ok(())
}
