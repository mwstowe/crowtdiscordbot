use crate::db_utils;
use crate::duckduckgo_search::DuckDuckGoSearchClient;
use crate::gemini_api::GeminiClient;
use crate::multi_response_generator::MultiResponseGenerator;
use crate::news_verification;
use anyhow::Result;
use serenity::http::Http;
use serenity::model::channel::Message;
use serenity::model::id::ChannelId;
use serenity::prelude::*;
use std::sync::Arc;
use tokio_rusqlite::Connection;
use tracing::{error, info};

/// Maximum number of recent fact topics to remember for deduplication
const MAX_RECENT_TOPICS: usize = 200;

/// Minimum significant-word overlap (Jaccard, 0.0-1.0) to consider two topics duplicates
const SIMILARITY_THRESHOLD: f64 = 0.34;

/// Common words that carry no subject meaning and should be ignored when
/// comparing two topics for similarity.
const STOPWORDS: &[&str] = &[
    "the", "a", "an", "of", "and", "or", "in", "on", "at", "to", "for", "with", "by", "from",
    "about", "as", "is", "are", "was", "were", "be", "been", "that", "this", "these", "those",
    "it", "its", "how", "why", "what", "when", "where", "which", "who", "new", "first", "most",
    "per", "up", "out", "over", "than", "more", "less", "not", "no", "into",
];

/// Reduce a topic string to a set of significant (non-stopword) lowercase words.
/// This is the "subject key" used for comparing topics regardless of phrasing.
fn subject_words(topic: &str) -> Vec<String> {
    topic
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3 && !STOPWORDS.contains(w))
        .map(|w| w.to_string())
        .collect()
}

/// Compare two topics for subject similarity (phrasing-independent).
fn topics_are_similar(a: &str, b: &str) -> bool {
    let a_words = subject_words(a);
    let b_words = subject_words(b);
    if a_words.is_empty() || b_words.is_empty() {
        return false;
    }

    // Shared significant words (set intersection)
    let shared: Vec<&String> = a_words.iter().filter(|w| b_words.contains(w)).collect();
    let common = shared.len();
    if common == 0 {
        return false;
    }

    // Jaccard similarity over the union of significant words
    let union: usize = {
        let mut all: Vec<&String> = a_words.iter().chain(b_words.iter()).collect();
        all.sort();
        all.dedup();
        all.len()
    };
    let similarity = common as f64 / union as f64;

    // A single shared word counts as a duplicate only if it is distinctive
    // (a longer word like "molasses" or "filibuster" is a strong subject
    // signal; short generic words like "war" or "list" are not).
    let has_distinctive_shared = shared.iter().any(|w| w.len() >= 6);

    // Duplicate if: two+ shared subject words, OR one distinctive shared word,
    // OR high overall overlap.
    common >= 2 || has_distinctive_shared || similarity >= SIMILARITY_THRESHOLD
}

/// Check if a topic is too similar to any recently shared topic.
fn is_duplicate_topic(topic: &str, recent_topics: &[String]) -> bool {
    for prev in recent_topics {
        if topics_are_similar(topic, prev) {
            info!(
                "Fact topic rejected (similar to recent): '{}' vs '{}'",
                topic, prev
            );
            return true;
        }
    }
    false
}

/// Extract topic from response in "TOPIC: description ENDTOPIC" format
fn extract_topic_from_response(response: &str) -> Option<String> {
    let topic_start = response.find("TOPIC:")?;
    let after_topic = &response[topic_start + 6..];

    // Look for ENDTOPIC delimiter first
    if let Some(end_pos) = after_topic.find("ENDTOPIC") {
        let topic = after_topic[..end_pos].trim();
        if !topic.is_empty() {
            return Some(topic.to_string());
        }
    }

    // Fallback: take only first 8 words after TOPIC: as the search query
    let topic: String = after_topic
        .split_whitespace()
        .take(8)
        .collect::<Vec<_>>()
        .join(" ");
    if !topic.is_empty() {
        Some(topic)
    } else {
        None
    }
}

/// Remove the TOPIC tag from the response text for display
fn strip_topic_from_response(response: &str) -> String {
    if let Some(topic_start) = response.find("TOPIC:") {
        let before = &response[..topic_start];
        let after_topic = &response[topic_start + 6..];

        if let Some(end_pos) = after_topic.find("ENDTOPIC") {
            let rest = after_topic[end_pos + 8..].trim_start();

            // Always strip the TOPIC tag — it's a search hint, not display text
            let before_clean = before.trim_end();
            if before_clean.is_empty() {
                rest.to_string()
            } else if rest.is_empty() {
                before_clean.to_string()
            } else {
                // If the before part ends with a comma (mid-sentence split),
                // lowercase the first character of rest to maintain grammar
                let rest_adjusted = if before_clean.ends_with(',') {
                    let mut chars = rest.chars();
                    match chars.next() {
                        Some(c) if c.is_uppercase() => {
                            format!("{}{}", c.to_lowercase(), chars.as_str())
                        }
                        _ => rest.to_string(),
                    }
                } else {
                    rest.to_string()
                };
                format!("{} {}", before_clean, rest_adjusted)
            }
        } else {
            // No ENDTOPIC found — return everything before TOPIC
            before.trim().to_string()
        }
    } else {
        response.to_string()
    }
}

/// Check if stripping the TOPIC tag left a dangling subject reference.
/// This catches cases like "TOPIC: origin of the term filibuster ENDTOPIC The word comes from..."
/// where "The word" refers to "filibuster" which only existed inside the TOPIC tag.
fn has_dangling_subject(response: &str, topic: &str) -> bool {
    let topic_start = match response.find("TOPIC:") {
        Some(pos) => pos,
        None => return false,
    };
    let before = &response[..topic_start];
    let after_topic = &response[topic_start + 6..];
    let rest = match after_topic.find("ENDTOPIC") {
        Some(end_pos) => after_topic[end_pos + 8..].trim_start(),
        None => return false,
    };

    // Common dangling reference patterns that signal the subject was inside the TOPIC
    let dangling_starters = [
        "The word ",
        "The term ",
        "The name ",
        "The phrase ",
        "The concept ",
        "It ",
        "It's ",
        "Its ",
        "This ",
        "That ",
        "the word ",
        "the term ",
        "the name ",
        "the phrase ",
        "the concept ",
        "it ",
        "it's ",
        "its ",
        "this ",
        "that ",
    ];

    let starts_with_dangling = dangling_starters
        .iter()
        .any(|pattern| rest.starts_with(pattern));

    if !starts_with_dangling {
        return false;
    }

    // Check if the key subject from the TOPIC actually appears in the text before the TOPIC.
    // If it does, the reference isn't dangling — the subject was already introduced.
    let topic_lower = topic.to_lowercase();
    let before_lower = before.to_lowercase();

    // Extract meaningful words from topic (skip common prefixes like "origin of the")
    let skip_words = [
        "origin",
        "of",
        "the",
        "term",
        "word",
        "name",
        "history",
        "meaning",
        "etymology",
        "definition",
        "concept",
        "phrase",
    ];
    let key_words: Vec<&str> = topic_lower
        .split_whitespace()
        .filter(|w| !skip_words.contains(w))
        .collect();

    // If any key word from the topic appears before the tag, the reference is grounded
    let subject_in_before = key_words.iter().any(|word| before_lower.contains(word));

    // Dangling if the subject is NOT mentioned before the TOPIC tag
    !subject_in_before
}

/// Search for an article using DuckDuckGo
async fn try_search_for_article(query: &str) -> Option<String> {
    info!("Searching DuckDuckGo for fact source: {}", query);
    let client = DuckDuckGoSearchClient::new();
    match client.search(query).await {
        Ok(Some(result)) => {
            info!("Found search result: {} - {}", result.title, result.url);
            Some(result.url)
        }
        Ok(None) => {
            info!("No search results found for: {}", query);
            None
        }
        Err(e) => {
            error!("DuckDuckGo search failed: {:?}", e);
            None
        }
    }
}

// Handle fact interjection with Message object
pub async fn handle_fact_interjection(
    ctx: &Context,
    msg: &Message,
    gemini_client: &GeminiClient,
    _multi_response_generator: &Option<MultiResponseGenerator>,
    message_db: &Option<Arc<Connection>>,
    bot_name: &str,
    gemini_context_messages: usize,
) -> Result<bool> {
    let context_messages = if let Some(db) = message_db {
        match db_utils::get_recent_messages_with_reply_context(
            db.clone(),
            gemini_context_messages,
            Some(msg.channel_id.to_string().as_str()),
        )
        .await
        {
            Ok(messages) => messages,
            Err(e) => {
                error!(
                    "Error retrieving recent messages for fact interjection: {:?}",
                    e
                );
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    handle_fact_interjection_common(
        &ctx.http,
        msg.channel_id,
        gemini_client,
        _multi_response_generator,
        &context_messages,
        bot_name,
        message_db,
    )
    .await
}

// Handle fact interjection for spontaneous interjections (without Message object)
pub async fn handle_spontaneous_fact_interjection(
    http: &Http,
    channel_id: ChannelId,
    gemini_client: &GeminiClient,
    _multi_response_generator: &Option<MultiResponseGenerator>,
    message_db: &Option<Arc<Connection>>,
    bot_name: &str,
    gemini_context_messages: usize,
) -> Result<bool> {
    let context_messages = if let Some(db) = message_db {
        match db_utils::get_recent_messages_with_reply_context(
            db.clone(),
            gemini_context_messages,
            Some(&channel_id.to_string()),
        )
        .await
        {
            Ok(messages) => messages,
            Err(e) => {
                error!(
                    "Error retrieving recent messages for spontaneous fact interjection: {:?}",
                    e
                );
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    handle_fact_interjection_common(
        http,
        channel_id,
        gemini_client,
        _multi_response_generator,
        &context_messages,
        bot_name,
        message_db,
    )
    .await
}

/// Send a fact response with typing delay
async fn send_fact_response(http: &Http, channel_id: ChannelId, response: &str) {
    if let Err(e) = channel_id.broadcast_typing(http).await {
        error!(
            "Failed to send typing indicator for fact interjection: {:?}",
            e
        );
    }

    let words = response.split_whitespace().count();
    let delay_secs = (words as f32 * 0.2).clamp(2.0, 5.0) as u64;
    tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;

    if let Err(e) = channel_id.say(http, response).await {
        error!("Error sending fact interjection: {:?}", e);
    } else {
        info!("Fact interjection sent: {}", response);
    }
}

#[allow(clippy::type_complexity)]
async fn handle_fact_interjection_common(
    http: &Http,
    channel_id: ChannelId,
    gemini_client: &GeminiClient,
    _multi_response_generator: &Option<MultiResponseGenerator>,
    context_messages: &[(String, String, Option<String>, String, Option<String>)],
    _bot_name: &str,
    message_db: &Option<Arc<Connection>>,
) -> Result<bool> {
    // Format context for the prompt
    let context_text = if !context_messages.is_empty() {
        let mut chronological_messages = context_messages.to_owned();
        chronological_messages.reverse();

        let formatted_messages: Vec<String> = chronological_messages
            .iter()
            .map(
                |(_author, display_name, _pronouns, content, reply_context)| {
                    if let Some(reply) = reply_context {
                        format!("{}: {} (in reply to: {})", display_name, content, reply)
                    } else {
                        format!("{}: {}", display_name, content)
                    }
                },
            )
            .collect();
        formatted_messages.join("\n")
    } else {
        info!(
            "No context available for fact interjection in channel_id: {}",
            channel_id
        );
        "".to_string()
    };

    let fact_prompt = gemini_client
        .prompt_templates()
        .format_fact_interjection(&context_text);

    // fact_prompt is already fully formed (personality + context baked in).
    // Always use generate_content directly to avoid re-wrapping with personality.
    let response_result = match gemini_client.generate_content(&fact_prompt).await {
        Ok(response) => {
            let trimmed = response.trim().to_string();
            if trimmed.to_lowercase() == "pass" {
                Ok(None)
            } else {
                Ok(Some(trimmed))
            }
        }
        Err(e) => Err(e),
    };

    let sent = match response_result {
        Ok(Some(response)) => {
            // Check if the response looks like the prompt itself
            if response.contains("{bot_name}")
                || response.contains("{context}")
                || response.contains("Guidelines:")
            {
                error!("Fact interjection: API returned prompt template instead of response");
                return Ok(false);
            }

            // Extract topic and use search-first approach
            if let Some(topic) = extract_topic_from_response(&response) {
                info!("Extracted fact topic for search: {}", topic);

                // Load recently shared topics from the database (survives restarts)
                let recent_topics: Vec<String> = if let Some(db) = message_db {
                    match db_utils::load_recent_fact_topics(db, MAX_RECENT_TOPICS).await {
                        Ok(topics) => topics,
                        Err(e) => {
                            error!("Failed to load recent fact topics: {:?}", e);
                            Vec::new()
                        }
                    }
                } else {
                    Vec::new()
                };

                // Check if this topic is too similar to recently shared facts
                if is_duplicate_topic(&topic, &recent_topics) {
                    info!(
                        "Fact interjection skipped: topic '{}' too similar to recent facts",
                        topic
                    );
                    return Ok(false);
                }

                let display_response = strip_topic_from_response(&response);

                // Guard: don't send if stripping the TOPIC left an incomplete sentence
                if display_response.ends_with(',')
                    || display_response.ends_with(':')
                    || display_response.ends_with("...")
                    || display_response.ends_with(';')
                {
                    info!(
                        "Fact interjection skipped: response is incomplete after stripping TOPIC: '{}'",
                        display_response
                    );
                    return Ok(false);
                }

                // Guard: don't send if the TOPIC contained the subject and
                // the remaining text has a dangling reference to it
                if has_dangling_subject(&response, &topic) {
                    info!(
                        "Fact interjection skipped: dangling subject reference after stripping TOPIC '{}': '{}'",
                        topic, display_response
                    );
                    return Ok(false);
                }

                if let Some(url) = try_search_for_article(&topic).await {
                    // Validate the search result
                    match news_verification::verify_news_article(
                        gemini_client,
                        gemini_client.http_client(),
                        &topic,
                        &url,
                        &display_response,
                    )
                    .await
                    {
                        Ok(true) => {
                            info!("Fact search result validated: {}", url);
                            let final_response = format!("{} Source: {}", display_response, url);
                            send_fact_response(http, channel_id, &final_response).await;
                            if let Some(db) = message_db {
                                if let Err(e) =
                                    db_utils::record_fact_topic(db, &topic, MAX_RECENT_TOPICS).await
                                {
                                    error!("Failed to record fact topic: {:?}", e);
                                }
                            }
                        }
                        _ => {
                            info!("Fact search result failed validation - skipping (likely hallucinated)");
                        }
                    }
                } else {
                    info!("No search results for fact topic - skipping (likely hallucinated)");
                }
            } else {
                info!("No TOPIC found in fact response - skipping (cannot verify)");
            }
            true
        }
        Ok(None) => {
            info!("Fact interjection evaluation: decided to PASS - no response sent");
            false
        }
        Err(e) => {
            error!("Error generating fact interjection: {:?}", e);
            false
        }
    };

    Ok(sent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_event_different_phrasing_is_duplicate() {
        assert!(topics_are_similar(
            "Great Molasses Flood 1919",
            "Boston molasses disaster deaths"
        ));
        assert!(topics_are_similar(
            "Emu War Australia 1932",
            "great emu war"
        ));
    }

    #[test]
    fn unrelated_topics_are_not_duplicates() {
        assert!(!topics_are_similar(
            "Great Molasses Flood 1919",
            "IBM quantum computing breakthrough"
        ));
        assert!(!topics_are_similar(
            "Finland highest coffee consumption",
            "octopus three hearts blue blood"
        ));
    }

    #[test]
    fn stopwords_do_not_cause_false_matches() {
        // Only shared words are stopwords -> should NOT match
        assert!(!topics_are_similar(
            "the history of the printing press",
            "the story of the first airplane"
        ));
    }

    #[test]
    fn single_short_generic_shared_word_does_not_match() {
        // Share only "war" (3 chars, not distinctive) and nothing else -> no match
        assert!(!topics_are_similar(
            "cold war espionage",
            "war of the roses"
        ));
    }

    #[test]
    fn is_duplicate_topic_matches_against_recent_list() {
        let recent = vec![
            "IBM quantum computing breakthrough".to_string(),
            "Great Molasses Flood 1919".to_string(),
        ];
        // Same subject as an entry in the list -> duplicate
        assert!(is_duplicate_topic(
            "Boston molasses disaster deaths",
            &recent
        ));
        // Fresh subject not in the list -> not a duplicate
        assert!(!is_duplicate_topic(
            "octopus three hearts blue blood",
            &recent
        ));
        // Empty history -> never a duplicate
        assert!(!is_duplicate_topic("anything at all here", &[]));
    }
}
