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

        // Use search_with_strategy directly to avoid recursion (non-strict: a random
        // term should always yield something).
        self.search_with_strategy(&random_term, false).await
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

        // Try a direct search first, strictly (phrase queries require an exact
        // substring match; word-salad word-level hits are rejected).
        if let Some(result) = self.search_with_strategy(query, true).await? {
            info!("Found result with direct search");
            return Ok(Some(result));
        }

        // If direct search fails and it's a multi-word query, try with quotes (strict).
        if query.contains(' ') {
            if let Some(result) = self
                .search_with_strategy(&format!("\"{query}\""), true)
                .await?
            {
                info!("Found result with quoted search");
                return Ok(Some(result));
            }
        }

        // No exact phrase match found. As a last resort, allow a non-strict
        // word-level best match (API relevance order) rather than a fully random,
        // unrelated screenshot — but only if the API actually returned something.
        info!(
            "No exact match for '{}', trying non-strict word-level best match",
            query
        );
        if let Some(result) = self.search_with_strategy(query, false).await? {
            info!("Found result with non-strict word-level search");
            return Ok(Some(result));
        }

        // Nothing matched at all.
        info!("No results found for query: {}", query);
        Ok(None)
    }

    // Internal method to perform the actual API call with a specific search strategy.
    // When `strict` is true, phrase queries with no exact substring match return None
    // instead of falling back to word-level (AND) matches, which are often unrelated.
    async fn search_with_strategy(
        &self,
        query: &str,
        strict: bool,
    ) -> Result<Option<FrinkiacResult>> {
        // URL encode the query
        let encoded_query = urlencoding::encode(query);

        // First try classic seasons (1-11) with API filter
        let classic_url = format!("{FRINKIAC_BASE_URL}?q={encoded_query}&smin=1&smax=11");
        let classic_results = self.fetch_search_results(&classic_url).await?;

        // Then get unfiltered results (for later seasons fallback)
        let all_url = format!("{FRINKIAC_BASE_URL}?q={encoded_query}");
        let all_results = self.fetch_search_results(&all_url).await?;

        // For multi-word queries, prioritize results containing the exact phrase
        // The API does word-level matching, so "my cube" returns anything with "my" AND "cube"
        // We filter client-side to prefer results with the exact phrase as a substring
        let query_lower = query.trim_matches('"').to_lowercase();
        let is_phrase_query = query_lower.contains(' ');

        // Deduplicate each set by episode
        let classic_unique = Self::dedupe_by_episode(&classic_results);
        let later_unique: Vec<&serde_json::Value> = Self::dedupe_by_episode(&all_results)
            .into_iter()
            .filter(|r| {
                let ep = r.get("Episode").and_then(|v| v.as_str()).unwrap_or("");
                let season: u32 = ep
                    .strip_prefix('S')
                    .and_then(|s| s.split('E').next())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(99);
                season > 11
            })
            .collect();

        // Helper: does a candidate's caption contain the exact phrase as a substring?
        let contains_phrase = |r: &serde_json::Value| -> bool {
            r.get("Content")
                .and_then(|v| v.as_str())
                .is_some_and(|c| c.to_lowercase().contains(&query_lower))
        };

        // If this is a phrase query, partition results into exact matches and the rest.
        // IMPORTANT: preserve Frinkiac's relevance order within each partition.
        let (classic_exact, classic_rest): (Vec<&serde_json::Value>, Vec<&serde_json::Value>) =
            if is_phrase_query {
                classic_unique.into_iter().partition(|r| contains_phrase(r))
            } else {
                (classic_unique, Vec::new())
            };

        let (later_exact, later_rest): (Vec<&serde_json::Value>, Vec<&serde_json::Value>) =
            if is_phrase_query {
                later_unique.into_iter().partition(|r| contains_phrase(r))
            } else {
                (later_unique, Vec::new())
            };

        // Build the exact-match tier (classics before later seasons), keeping API order,
        // then sort by match tightness so the closest match wins. Ties keep API order
        // because sort_by is stable.
        let mut exact: Vec<&serde_json::Value> =
            classic_exact.into_iter().chain(later_exact).collect();
        if is_phrase_query {
            exact.sort_by(|a, b| {
                Self::phrase_score(b, &query_lower)
                    .partial_cmp(&Self::phrase_score(a, &query_lower))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        let exact_count = exact.len();

        // Non-exact tier keeps API relevance order (classics first).
        let rest: Vec<&serde_json::Value> = classic_rest.into_iter().chain(later_rest).collect();

        let ordered_results: Vec<&serde_json::Value> =
            exact.iter().copied().chain(rest.iter().copied()).collect();

        if is_phrase_query {
            info!(
                "Phrase query '{}': {} exact matches out of {} total results",
                query_lower,
                exact_count,
                ordered_results.len()
            );
        }

        if ordered_results.is_empty() {
            info!("No results found for query: {}", query);
            return Ok(None);
        }

        // For phrase queries with NO exact match, the API only did word-level AND matching,
        // which routinely produces unrelated word-salad frames (e.g. "never-ending story"
        // matching "never" + "story" in different sentences). In strict mode, treat that as
        // "no good match" so search() can decide how to fall back. In non-strict mode, allow
        // the API's top (relevance-ordered) result as a best-effort.
        if strict && is_phrase_query && exact_count == 0 {
            info!(
                "Phrase query '{}' had no exact matches (strict); reporting no good match",
                query_lower
            );
            return Ok(None);
        }

        // Determine the pool we cycle within: exact matches if any exist, else everything.
        let pool_len = if exact_count > 0 {
            exact_count
        } else {
            ordered_results.len()
        };

        // Selection: on the FIRST call for a query, always return the best match (index 0).
        // On repeated identical queries, rotate for variety — but only within the pool.
        let index = {
            let mut last_q = self.last_query.write().unwrap_or_else(|e| e.into_inner());
            let mut idx = self
                .current_index
                .write()
                .unwrap_or_else(|e| e.into_inner());
            if last_q.as_deref() == Some(query) {
                *idx = (*idx + 1) % pool_len;
            } else {
                *last_q = Some(query.to_string());
                *idx = 0;
            }
            *idx
        };

        let result = ordered_results[index];

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
                            representative_ts: s.get("RepresentativeTimestamp")?.as_u64()?,
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
                    representative_ts: s.get("RepresentativeTimestamp")?.as_u64()?,
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
    /// Fetch raw search results from a frinkiac search URL
    async fn fetch_search_results(&self, url: &str) -> Result<Vec<serde_json::Value>> {
        let response = self.http_client.get(url).send().await?;
        if !response.status().is_success() {
            return Ok(Vec::new());
        }
        let results: Vec<serde_json::Value> = response.json().await.unwrap_or_default();
        Ok(results)
    }

    /// Deduplicate search results by episode, keeping the first occurrence of each
    fn dedupe_by_episode(results: &[serde_json::Value]) -> Vec<&serde_json::Value> {
        let mut seen = std::collections::HashSet::new();
        results
            .iter()
            .filter(|r| {
                let ep = r.get("Episode").and_then(|v| v.as_str()).unwrap_or("");
                seen.insert(ep.to_string())
            })
            .collect()
    }

    /// Score how tightly a candidate caption matches the query phrase.
    /// Higher is better; 0.0 if the phrase isn't present. Delegates to the
    /// shared `phrase_score_str` helper (also used by Morbotron).
    fn phrase_score(result: &serde_json::Value, query_lower: &str) -> f64 {
        let content = result.get("Content").and_then(|v| v.as_str()).unwrap_or("");
        phrase_score_str(content, query_lower)
    }

    pub async fn expand_to_sentence_boundaries(&self, result: &mut FrinkiacResult) {
        let episode = result._episode.clone();

        // Maximum number of adjacent-card hops in each direction. Bounds API calls
        // and prevents runaway expansion on captions that never hit a clean boundary.
        const MAX_HOPS: usize = 4;

        // The caption window we already have often contains the whole sentence
        // (the API returns several adjacent cards per frame). The loops below only
        // need to hop outward when the window's *edge* card is itself mid-sentence.
        //
        // Hops are anchored on the edge card's RepresentativeTimestamp, which is
        // guaranteed to be an indexed ("on-grid") frame, and stepped by the spacing
        // observed within the current window. Probing arbitrary off-grid timestamps
        // (e.g. end+1) returns 404 on the sparse, irregular frame grid, which is why
        // earlier attempts silently failed.
        let step = observed_frame_step(&result.subtitles);

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

                let (first_start, first_rep) = match result.subtitles.first() {
                    Some(s) => (s.start, s.representative_ts),
                    None => break,
                };
                if first_rep == 0 {
                    break;
                }

                // Probe one frame-step before the edge card's on-grid anchor.
                let probe_ts = first_rep.saturating_sub(step).max(1);
                let earlier = match self.get_caption_for_frame(&episode, probe_ts).await {
                    Ok(Some(e)) => e,
                    _ => break,
                };

                // Prepend any cards from the probed window that fall strictly before
                // our current start (dedupe by start timestamp), in chronological order.
                let preceding: Vec<TimedSubtitle> = earlier
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

                // Stop once the (new) first subtitle begins a sentence.
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

                let (last_end, last_rep) = match result.subtitles.last() {
                    Some(s) => (s.end, s.representative_ts),
                    None => break,
                };
                if last_rep == 0 {
                    break;
                }

                // Probe one frame-step past the edge card's on-grid anchor.
                let probe_ts = last_rep.saturating_add(step);
                let later = match self.get_caption_for_frame(&episode, probe_ts).await {
                    Ok(Some(l)) => l,
                    _ => break,
                };

                // Append any cards from the probed window that start at/after our
                // current end, in chronological order.
                let following: Vec<TimedSubtitle> = later
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

/// True if the text begins partway through a sentence (lowercase first letter).
/// Leading punctuation/quotes/speaker dashes are skipped before checking.
pub(crate) fn starts_mid_sentence(text: &str) -> bool {
    text.chars()
        .find(|c| c.is_alphabetic())
        .is_some_and(|c| c.is_lowercase())
}

/// True if the text forms a complete sentence ending (terminal punctuation).
pub(crate) fn ends_sentence(text: &str) -> bool {
    let t = text.trim_end();
    t.ends_with('.')
        || t.ends_with('!')
        || t.ends_with('?')
        || t.ends_with('"')
        || t.ends_with('\u{201d}') // right double quotation mark
}

/// True if the text ends partway through a sentence (no terminal punctuation,
/// or a trailing comma).
pub(crate) fn ends_mid_sentence(text: &str) -> bool {
    let t = text.trim_end();
    t.ends_with(',') || !ends_sentence(t)
}

/// Estimate the indexed-frame step (ms) to use when hopping to adjacent caption
/// cards. Derived from the spacing between consecutive cards in the current
/// window; falls back to a conservative default when the window has too few
/// cards to measure. Clamped to a sane range so a single outlier gap can't
/// produce a wild probe.
pub(crate) fn observed_frame_step(subs: &[TimedSubtitle]) -> u64 {
    // Default when we can't observe spacing: ~1 card width. The grid is sparse
    // and irregular across sources, so this is a best-effort anchor step; probing
    // lands within a neighbouring card's window because windows overlap.
    const DEFAULT_STEP: u64 = 1200;
    const MIN_STEP: u64 = 600;
    const MAX_STEP: u64 = 2500;

    // Gaps between consecutive cards' representative timestamps (on-grid anchors).
    let mut gaps: Vec<u64> = subs
        .windows(2)
        .filter_map(|w| {
            let a = w[0].representative_ts;
            let b = w[1].representative_ts;
            if b > a {
                Some(b - a)
            } else {
                None
            }
        })
        .collect();

    if gaps.is_empty() {
        return DEFAULT_STEP;
    }

    // Use the smallest observed gap: adjacent cards are the closest neighbours,
    // so the minimum spacing best approximates one hop.
    gaps.sort_unstable();
    gaps[0].clamp(MIN_STEP, MAX_STEP)
}

/// Score how tightly a caption matches a query phrase. Higher is better.
/// Returns 0.0 if the phrase isn't present as a substring.
///
/// Rewards the phrase being a large fraction of the caption (so a caption that
/// *is* the quote beats one where the phrase is buried in a long multi-sentence
/// run), with a bonus when the caption is (nearly) exactly the phrase.
/// `query_lower` must already be lowercased; `caption` is lowercased internally.
pub(crate) fn phrase_score_str(caption: &str, query_lower: &str) -> f64 {
    let content = caption.to_lowercase();
    if content.is_empty() || !content.contains(query_lower) {
        return 0.0;
    }

    let query_len = query_lower.chars().count() as f64;
    let content_len = content.chars().count().max(1) as f64;

    // Fraction of the caption occupied by the query (0.0–1.0).
    let coverage = (query_len / content_len).min(1.0);

    // Bonus if the trimmed caption equals the phrase (ignoring surrounding
    // punctuation/whitespace) — the ideal "closest match".
    let trimmed = content.trim_matches(|c: char| !c.is_alphanumeric());
    let exact_bonus = if trimmed == query_lower { 1.0 } else { 0.0 };

    coverage + exact_bonus
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
    /// A valid frame timestamp for this subtitle (always maps to an indexed frame)
    pub representative_ts: u64,
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
#[allow(clippy::too_many_arguments)]
pub async fn generate_gif(
    http_client: &reqwest::Client,
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

    let response = http_client
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
async fn send_frinkiac_result(
    http: &Http,
    msg: &Message,
    result: &FrinkiacResult,
    http_client: &reqwest::Client,
) {
    if let Some(gif_url) = &result.gif_url {
        let title = format!(
            "{} (Season {}, Episode {})",
            result.episode_title, result.season, result.episode_number
        );

        // Download the GIF and upload as attachment for reliable display
        let mut sent = false;
        match http_client.get(gif_url.as_str()).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(bytes) = resp.bytes().await {
                    let attachment = serenity::builder::CreateAttachment::bytes(
                        bytes.to_vec(),
                        "frinkiac.gif".to_string(),
                    );
                    let message = CreateMessage::new()
                        .content(&title)
                        .add_file(attachment)
                        .reference_message(msg);
                    match msg.channel_id.send_message(http, message).await {
                        Ok(_) => {
                            sent = true;
                        }
                        Err(e) => {
                            error!("Error sending Frinkiac GIF attachment: {:?}", e);
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
            if let Err(e) = msg.reply(http, &response).await {
                error!("Error sending Frinkiac result fallback: {:?}", e);
            }
        }
    } else {
        let response = format_frinkiac_result(result);
        if let Err(e) = msg.reply(http, &response).await {
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
                    &frinkiac_client.http_client,
                    "https://frinkiac.com",
                    &result._episode,
                    result.start_timestamp,
                    result.end_timestamp,
                    &merge_subtitle_fragments(&result.subtitles),
                    0,
                    "akbar",
                )
                .await;
                send_frinkiac_result(http, msg, &result, &frinkiac_client.http_client).await;
            }
            Ok(None) => {
                let _ = msg
                    .reply(http, "Couldn't find any Simpsons screenshots. D'oh!")
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
                        &frinkiac_client.http_client,
                        "https://frinkiac.com",
                        &result._episode,
                        result.start_timestamp,
                        result.end_timestamp,
                        &merge_subtitle_fragments(&result.subtitles),
                        0,
                        "akbar",
                    )
                    .await;
                    send_frinkiac_result(http, msg, &result, &frinkiac_client.http_client).await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn starts_mid_sentence_detects_lowercase_opening() {
        assert!(starts_mid_sentence("just like the one on the sign."));
        assert!(!starts_mid_sentence("That's false advertising."));
        // Leading punctuation / speaker dash is skipped before judging.
        assert!(!starts_mid_sentence("\"Man, that's some story.\""));
        assert!(starts_mid_sentence("- and then he left."));
    }

    #[test]
    fn ends_sentence_and_mid_detection() {
        assert!(ends_sentence("That's false advertising!"));
        assert!(ends_sentence("Is it though?"));
        assert!(ends_sentence("\"the end.\""));
        assert!(!ends_sentence("Man, that is"));

        assert!(ends_mid_sentence("Man, that is flagrant"));
        assert!(ends_mid_sentence("one thing,"));
        assert!(!ends_mid_sentence("That's false advertising!"));
    }

    #[test]
    fn phrase_score_prefers_tighter_match() {
        let q = "false advertising";

        // Caption that is essentially the phrase should outscore one where the
        // phrase is buried in a long multi-sentence run.
        let tight = json!({ "Content": "Man, that is flagrant false advertising!" });
        let buried = json!({
            "Content": "just like the one on the sign. D'oh! Nuts. \
                        That's false advertising. Sorry, sir. No refunds."
        });

        let s_tight = FrinkiacClient::phrase_score(&tight, q);
        let s_buried = FrinkiacClient::phrase_score(&buried, q);
        assert!(
            s_tight > s_buried,
            "tight={s_tight} should beat buried={s_buried}"
        );
    }

    #[test]
    fn phrase_score_exact_caption_gets_bonus() {
        let q = "false advertising";
        let exact = json!({ "Content": "False advertising!" });
        let partial = json!({ "Content": "that is false advertising for sure right" });
        assert!(
            FrinkiacClient::phrase_score(&exact, q) > FrinkiacClient::phrase_score(&partial, q)
        );
    }

    #[test]
    fn phrase_score_zero_when_absent() {
        assert_eq!(
            FrinkiacClient::phrase_score(&json!({ "Content": "d'oh!" }), "false advertising"),
            0.0
        );
    }

    fn ts(rep: u64) -> TimedSubtitle {
        TimedSubtitle {
            text: String::new(),
            start: rep.saturating_sub(100),
            end: rep.saturating_add(100),
            representative_ts: rep,
        }
    }

    #[test]
    fn observed_frame_step_uses_min_gap_clamped() {
        // Gaps: 1877, 1877 -> min 1877 (within range)
        let subs = vec![ts(133008), ts(134885), ts(136762)];
        assert_eq!(observed_frame_step(&subs), 1877);

        // Fewer than 2 cards -> default
        assert_eq!(observed_frame_step(&[ts(100000)]), 1200);
        assert_eq!(observed_frame_step(&[]), 1200);

        // Tiny gap clamps up to MIN_STEP (600)
        let close = vec![ts(1000), ts(1200)]; // gap 200
        assert_eq!(observed_frame_step(&close), 600);

        // Huge gap clamps down to MAX_STEP (2500)
        let far = vec![ts(1000), ts(99999)]; // gap ~99k
        assert_eq!(observed_frame_step(&far), 2500);
    }
}
