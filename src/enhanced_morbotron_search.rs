use anyhow::Result;
use tracing::{info, error};
use crate::gemini_api::GeminiClient;
use crate::morbotron::{MorbotronClient, MorbotronResult};
use crate::google_search::GoogleSearchClient;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use strsim::jaro_winkler;
use regex::Regex;

// A struct to hold the popularity data for quotes
#[derive(Debug, Deserialize, Serialize)]
struct QuotePopularity {
    quotes: HashMap<String, u32>, // Maps quote text to popularity score
}

pub struct EnhancedMorbotronSearch {
    gemini_client: GeminiClient,
    morbotron_client: MorbotronClient,
    google_client: GoogleSearchClient,
}

impl EnhancedMorbotronSearch {
    pub fn new(gemini_client: GeminiClient, morbotron_client: MorbotronClient, google_client: GoogleSearchClient) -> Self {
        Self {
            gemini_client,
            morbotron_client,
            google_client,
        }
    }
    
    pub async fn search(&self, query: &str) -> Result<Option<MorbotronResult>> {
        info!("Enhanced Morbotron search for: {}", query);
        
        // 1) Use the morbotron API to search for the terms exactly as provided
        info!("Step 1: Trying direct search with Morbotron API");
        match self.morbotron_client.search(query).await {
            Ok(Some(result)) => {
                info!("Found result with direct Morbotron API search");
                return Ok(Some(result));
            },
            Ok(None) => {
                info!("No results from direct Morbotron API search, trying site search");
            },
            Err(e) => {
                error!("Error with direct Morbotron API search: {}, trying site search", e);
            }
        }
        
        // 2) Use the direct site search to search for the terms exactly as provided
        info!("Step 2: Trying direct site search");
        let site_search_query = format!("site:morbotron.com \"{}\"", query);
        match self.google_client.search(&site_search_query).await {
            Ok(Some(search_result)) => {
                info!("Found direct site search result: {} - {}", search_result.title, search_result.url);
                
                // Try to extract the frame ID from the URL
                if let Some(frame_id) = extract_frame_id_from_url(&search_result.url) {
                    info!("Extracted frame ID from URL: {}", frame_id);
                    
                    // Construct a direct URL to the frame
                    let frame_parts: Vec<&str> = frame_id.split('_').collect();
                    if frame_parts.len() >= 2 {
                        let episode_code = frame_parts[0];
                        let timestamp = frame_parts[1];
                        
                        // Create a MorbotronResult directly
                        let result = MorbotronResult {
                            episode: episode_code.to_string(),
                            episode_title: search_result.title.clone(),
                            episode_number: 0, // We don't have this info
                            season: 0,         // We don't have this info
                            timestamp: timestamp.to_string(),
                            image_url: format!("https://morbotron.com/img/{}/{}", episode_code, timestamp),
                            caption: query.to_string(), // Use the original query as the caption
                        };
                        
                        info!("Created direct result from frame ID");
                        return Ok(Some(result));
                    }
                }
                
                // If we couldn't extract a frame ID, try to use the URL directly
                if search_result.url.contains("morbotron.com/meme/") {
                    // Extract the episode and timestamp from the URL
                    let url_parts: Vec<&str> = search_result.url.split('/').collect();
                    if url_parts.len() >= 5 {
                        let episode_code = url_parts[4];
                        let timestamp = if url_parts.len() >= 6 { url_parts[5] } else { "0" };
                        
                        // Create a MorbotronResult directly
                        let result = MorbotronResult {
                            episode: episode_code.to_string(),
                            episode_title: search_result.title.clone(),
                            episode_number: 0, // We don't have this info
                            season: 0,         // We don't have this info
                            timestamp: timestamp.to_string(),
                            image_url: format!("https://morbotron.com/img/{}/{}", episode_code, timestamp),
                            caption: query.to_string(), // Use the original query as the caption
                        };
                        
                        info!("Created direct result from URL");
                        return Ok(Some(result));
                    }
                }
            },
            Ok(None) => {
                info!("No results from direct site search, trying fuzzy search");
            },
            Err(e) => {
                error!("Error with direct site search: {}, trying fuzzy search", e);
            }
        }
        
        // 3) IF and only if neither of those yield results, try a fuzzy search in the same order
        info!("Step 3: Trying fuzzy search with Morbotron API");
        
        // Try with fuzzy variations of the query
        let fuzzy_variations = generate_fuzzy_variations(query);
        for variation in &fuzzy_variations {
            info!("Trying fuzzy variation with Morbotron API: {}", variation);
            match self.morbotron_client.search(variation).await {
                Ok(Some(result)) => {
                    info!("Found result with fuzzy Morbotron API search");
                    return Ok(Some(result));
                },
                _ => {}
            }
        }
        
        // Try fuzzy variations with site search
        for variation in &fuzzy_variations {
            info!("Trying fuzzy variation with site search: {}", variation);
            let site_search_query = format!("site:morbotron.com \"{}\"", variation);
            match self.google_client.search(&site_search_query).await {
                Ok(Some(search_result)) => {
                    info!("Found fuzzy site search result: {} - {}", search_result.title, search_result.url);
                    
                    // Try to extract the frame ID from the URL
                    if let Some(frame_id) = extract_frame_id_from_url(&search_result.url) {
                        info!("Extracted frame ID from URL: {}", frame_id);
                        
                        // Construct a direct URL to the frame
                        let frame_parts: Vec<&str> = frame_id.split('_').collect();
                        if frame_parts.len() >= 2 {
                            let episode_code = frame_parts[0];
                            let timestamp = frame_parts[1];
                            
                            // Create a MorbotronResult directly
                            let result = MorbotronResult {
                                episode: episode_code.to_string(),
                                episode_title: search_result.title.clone(),
                                episode_number: 0, // We don't have this info
                                season: 0,         // We don't have this info
                                timestamp: timestamp.to_string(),
                                image_url: format!("https://morbotron.com/img/{}/{}", episode_code, timestamp),
                                caption: variation.to_string(), // Use the variation as the caption
                            };
                            
                            info!("Created fuzzy result from frame ID");
                            return Ok(Some(result));
                        }
                    }
                },
                _ => {}
            }
        }
        
        // If all else fails, return None
        info!("No results found with any search strategy");
        Ok(None)
    }
    
    // Helper method to extract quotes from text (looks for text in quotes)
    fn extract_quote_from_text(&self, text: &str) -> Option<String> {
        let re = Regex::new(r#""([^"]+)""#).ok()?;
        re.captures(text).and_then(|caps| caps.get(1).map(|m| m.as_str().to_string()))
    }
    
    // Helper method to extract episode information from text
    fn extract_episode_info(&self, text: &str) -> Option<String> {
        // Look for season/episode patterns like S01E01 or Season 1 Episode 1
        let re1 = Regex::new(r"S(\d+)E(\d+)").ok()?;
        if let Some(caps) = re1.captures(text) {
            if let (Some(season), Some(episode)) = (caps.get(1), caps.get(2)) {
                return Some(format!("S{}E{}", season.as_str(), episode.as_str()));
            }
        }
        
        let re2 = Regex::new(r"Season\s+(\d+)\s+Episode\s+(\d+)").ok()?;
        if let Some(caps) = re2.captures(text) {
            if let (Some(season), Some(episode)) = (caps.get(1), caps.get(2)) {
                return Some(format!("S{:02}E{:02}", 
                    season.as_str().parse::<u32>().unwrap_or(0),
                    episode.as_str().parse::<u32>().unwrap_or(0)));
            }
        }
        
        None
    }
    
    // Helper method to extract potential quotes from text
    fn extract_potential_quotes(&self, title: &str, snippet: &str) -> Vec<String> {
        let mut quotes = Vec::new();
        
        // Look for quoted text
        let re = Regex::new(r#""([^"]+)""#).ok().unwrap_or_else(|| Regex::new(r".*").unwrap());
        
        for text in &[title, snippet] {
            for cap in re.captures_iter(text) {
                if let Some(quote) = cap.get(1) {
                    quotes.push(quote.as_str().to_string());
                }
            }
        }
        
        quotes
    }
    
    // Helper method to check if a result contains the search terms
    fn result_contains_search_terms(&self, result: &MorbotronResult, query: &str) -> bool {
        let query_lower = query.to_lowercase();
        let caption_lower = result.caption.to_lowercase();
        let episode_title_lower = result.episode_title.to_lowercase();
        
        // Split query into words
        let query_words: Vec<&str> = query_lower.split_whitespace().collect();
        
        // Check if all query words are in the caption or episode title
        for word in &query_words {
            if !caption_lower.contains(word) && !episode_title_lower.contains(word) {
                return false;
            }
        }
        
        true
    }
    
    // Calculate a total relevance score for a result
    fn calculate_total_score(&self, result: &MorbotronResult, query: &str, search_term: &str) -> f32 {
        let query_lower = query.to_lowercase();
        let caption_lower = result.caption.to_lowercase();
        let episode_title_lower = result.episode_title.to_lowercase();
        
        // Split query into words
        let query_words: Vec<&str> = query_lower.split_whitespace().collect();
        
        // Count how many query words are in the caption and title
        let caption_matching_words = query_words.iter()
            .filter(|&word| caption_lower.contains(word))
            .count();
        
        let title_matching_words = query_words.iter()
            .filter(|&word| episode_title_lower.contains(word))
            .count();
        
        // Calculate score based on proportion of matching words in caption
        let caption_proportion = caption_matching_words as f32 / query_words.len() as f32;
        
        // Calculate score based on proportion of matching words in title
        let title_proportion = title_matching_words as f32 / query_words.len() as f32;
        
        // Give a bonus if all words match in either caption or title
        let all_words_bonus = if caption_matching_words == query_words.len() || title_matching_words == query_words.len() {
            0.5
        } else {
            0.0
        };
        
        // Combine scores with weights
        let combined_score = (caption_proportion * 0.7) + (title_proportion * 0.3) + all_words_bonus;
        
        // Cap at 1.0
        combined_score.min(1.0)
    }
}

// Helper function to extract frame ID from a Morbotron URL
fn extract_frame_id_from_url(url: &str) -> Option<String> {
    // Look for patterns like /meme/S05E08/1211643 or /caption/S05E08_1211643
    let meme_re = regex::Regex::new(r"/meme/([^/]+)/(\d+)").ok()?;
    if let Some(caps) = meme_re.captures(url) {
        if let (Some(episode), Some(timestamp)) = (caps.get(1), caps.get(2)) {
            return Some(format!("{}_{}", episode.as_str(), timestamp.as_str()));
        }
    }
    
    // Look for patterns like /caption/S05E08_1211643
    let caption_re = regex::Regex::new(r"/caption/([^/]+)").ok()?;
    if let Some(caps) = caption_re.captures(url) {
        if let Some(frame_id) = caps.get(1) {
            return Some(frame_id.as_str().to_string());
        }
    }
    
    None
}

// Generate fuzzy variations of a query
fn generate_fuzzy_variations(query: &str) -> Vec<String> {
    let mut variations = Vec::new();
    
    // Add the original query
    variations.push(query.to_string());
    
    // Add lowercase version
    variations.push(query.to_lowercase());
    
    // Add version without punctuation
    let no_punct = query.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>();
    variations.push(no_punct);
    
    // Add version with first letter of each word capitalized
    let title_case = query.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<Vec<String>>()
        .join(" ");
    variations.push(title_case);
    
    // Remove duplicates
    variations.sort();
    variations.dedup();
    
    variations
}

// Helper function to extract season and episode information from text
fn extract_season_episode_from_text(text: &str) -> Option<String> {
    // Look for "Season X Episode Y" format
    let season_episode_re = regex::Regex::new(r"Season\s+(\d+)\s+Episode\s+(\d+)").ok()?;
    if let Some(caps) = season_episode_re.captures(text) {
        if let (Some(season), Some(episode)) = (caps.get(1), caps.get(2)) {
            return Some(format!("S{:02}E{:02}", 
                season.as_str().parse::<u32>().unwrap_or(0),
                episode.as_str().parse::<u32>().unwrap_or(0)));
        }
    }
    
    // Look for S##E## format
    let se_format_re = regex::Regex::new(r"S(\d+)E(\d+)").ok()?;
    if let Some(caps) = se_format_re.captures(text) {
        if let (Some(season), Some(episode)) = (caps.get(1), caps.get(2)) {
            return Some(format!("S{:02}E{:02}", 
                season.as_str().parse::<u32>().unwrap_or(0),
                episode.as_str().parse::<u32>().unwrap_or(0)));
        }
    }
    
    None
}

// Extract phrases from a quote (split by punctuation)
fn extract_phrases(text: &str) -> Vec<String> {
    let mut phrases = Vec::new();
    
    // Split by common punctuation that separates phrases
    let separators = ['.', '!', '?', ';', ':', ','];
    
    let mut start = 0;
    for (i, c) in text.char_indices() {
        if separators.contains(&c) {
            if i > start {
                let phrase = text[start..i].trim().to_string();
                if !phrase.is_empty() {
                    phrases.push(phrase);
                }
            }
            start = i + 1;
        }
    }
    
    // Add the last phrase if there is one
    if start < text.len() {
        let phrase = text[start..].trim().to_string();
        if !phrase.is_empty() {
            phrases.push(phrase);
        }
    }
    
    // If no phrases were found (no punctuation), add the whole text as one phrase
    if phrases.is_empty() && !text.trim().is_empty() {
        phrases.push(text.trim().to_string());
    }
    
    phrases
}

// Helper function to extract episode code from episode title
fn extract_episode_code_from_title(episode_title: &str) -> Option<String> {
    // Try to extract season and episode numbers from the title
    if let Some(season_episode) = extract_season_episode_from_text(episode_title) {
        return Some(season_episode);
    }
    
    // Look for "Season X Episode Y" format
    let season_episode_re = regex::Regex::new(r"Season\s+(\d+)\s+Episode\s+(\d+)").ok();
    if let Some(re) = season_episode_re {
        if let Some(caps) = re.captures(episode_title) {
            if let (Some(season), Some(episode)) = (caps.get(1), caps.get(2)) {
                return Some(format!("S{:02}E{:02}", 
                    season.as_str().parse::<u32>().unwrap_or(0),
                    episode.as_str().parse::<u32>().unwrap_or(0)));
            }
        }
    }
    
    // No episode code found
    None
}

// Helper function to check if a word is a common word that should be ignored in some contexts
fn is_common_word(word: &str) -> bool {
    const COMMON_WORDS: &[&str] = &[
        "the", "and", "that", "have", "for", "not", "with", "you", "this", "but",
        "his", "from", "they", "she", "will", "say", "would", "can", "been", "one",
        "all", "were", "when", "there", "what", "them", "some", "her", "who", "could",
        "make", "like", "time", "just", "him", "know", "take", "into", "year", "your",
        "good", "more", "than", "then", "look", "only", "come", "its", "over", "think",
        "also", "back", "after", "use", "two", "how", "our", "work", "first", "well",
        "way", "even", "new", "want", "because", "any", "these", "give", "day", "most",
    ];
    
    COMMON_WORDS.contains(&word.to_lowercase().as_str())
}
