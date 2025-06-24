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

// Constants for the Gemini prompt
const GEMINI_MORBOTRON_PROMPT: &str = r#"You are a Futurama quote expert tasked with finding the most relevant quote based on search terms.

Search terms: "{}"

Instructions:
1. Find a Futurama quote that EXPLICITLY contains the search terms when possible
2. If no exact match exists, find quotes that contain synonyms or related concepts
3. Prioritize quotes that include the EXACT search terms in them
4. Consider famous quotes that might relate to these concepts
5. Return your response in this exact JSON format:
   {{
     "quote": "The exact quote text",
     "episode": "Season X Episode Y: Episode Title",
     "character": "Character who said it"
   }}
6. If you can't find a relevant quote, respond with: {{"result": "pass"}}

Examples:
- Search: "bite metal"
- Response: {{"quote": "Bite my shiny metal ass!", "episode": "Season 1 Episode 1: Space Pilot 3000", "character": "Bender"}}

- Search: "good news"
- Response: {{"quote": "Good news, everyone!", "episode": "Various episodes", "character": "Professor Farnsworth"}}

- Search: "blackjack hookers"
- Response: {{"quote": "I'll make my own theme park with blackjack and hookers!", "episode": "Season 1 Episode 2: The Series Has Landed", "character": "Bender"}}

- Search: "death snu"
- Response: {{"quote": "Death by snu-snu!", "episode": "Season 3 Episode 1: Amazon Women in the Mood", "character": "Femputer"}}

- Search: "unknown phrase"
- Response: {{"result": "pass"}}

Remember to return ONLY the JSON with no additional text."#;

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

    // Main search function that uses search engine to enhance the search
    pub async fn search(&self, query: &str) -> Result<Option<MorbotronResult>> {
        info!("Enhanced Morbotron search for: {}", query);
        
        // Try direct site search on morbotron.com
        let direct_site_search = format!("site:morbotron.com {}", query);
        info!("Trying direct site search: {}", direct_site_search);
        
        // Use the Google search client with the site-specific search
        match self.google_client.search(&direct_site_search).await {
            Ok(Some(result)) => {
                info!("Found direct site search result: {} - {}", result.title, result.snippet);
                
                // Extract potential search terms from the result
                let mut search_terms = Vec::new();
                
                // Look for quotes in the title and snippet
                if let Some(quote) = self.extract_quote_from_text(&result.title) {
                    search_terms.push(quote);
                }
                
                if let Some(quote) = self.extract_quote_from_text(&result.snippet) {
                    search_terms.push(quote);
                }
                
                // Extract episode information
                if let Some(episode_info) = self.extract_episode_info(&format!("{} {}", result.title, result.snippet)) {
                    search_terms.push(episode_info);
                }
                
                // Extract potential phrases
                let potential_phrases = self.extract_potential_quotes(&result.title, &result.snippet);
                search_terms.extend(potential_phrases);
                
                // Add the original query
                search_terms.push(query.to_string());
                
                // Try each search term with Morbotron
                let mut results = Vec::new();
                
                for term in &search_terms {
                    info!("Trying search term from direct site search: {}", term);
                    match self.morbotron_client.search(term).await {
                        Ok(Some(result)) => {
                            // Verify that the result actually contains the search terms
                            if self.result_contains_search_terms(&result, query) {
                                let score = self.calculate_total_score(&result, query, term);
                                results.push((result, score));
                            } else {
                                info!("Result doesn't contain search terms, skipping");
                            }
                        },
                        _ => {}
                    }
                }
                
                // If we have results, return the best one
                if !results.is_empty() {
                    // Sort by score (descending)
                    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                    info!("Found {} results from direct site search, returning best match with score {:.2}", 
                          results.len(), results[0].1);
                    return Ok(Some(results[0].0.clone()));
                }
            },
            Ok(None) => {
                info!("No results from direct site search, trying Gemini API");
            },
            Err(e) => {
                error!("Error with direct site search: {}, trying Gemini API", e);
            }
        }
        
        // Try Gemini API as a fallback
        info!("Trying Gemini API for enhanced search");
        let gemini_prompt = GEMINI_MORBOTRON_PROMPT.replace("{}", query);
        
        match self.gemini_client.generate_content(&gemini_prompt).await {
            Ok(response) => {
                info!("Received Gemini API response: {}", response);
                
                // Skip if Gemini returned "pass"
                if response.trim().to_lowercase().contains("pass") {
                    info!("Gemini API returned 'pass', falling back to direct search");
                } else {
                    // Clean up the response - remove markdown code blocks and any other formatting
                    let cleaned_response = response
                        .replace("```json", "")
                        .replace("```", "")
                        .trim()
                        .to_string();
                    
                    info!("Cleaned Gemini response: {}", cleaned_response);
                    
                    // Try to parse the response as JSON
                    let json_result = serde_json::from_str::<serde_json::Value>(&cleaned_response);
                    
                    match json_result {
                        Ok(json) => {
                            info!("Successfully parsed Gemini response as JSON");
                            
                            // Extract quote, episode, and character information
                            let quote = json.get("quote").and_then(|q| q.as_str()).unwrap_or("");
                            let episode = json.get("episode").and_then(|e| e.as_str()).unwrap_or("");
                            let character = json.get("character").and_then(|c| c.as_str()).unwrap_or("");
                            
                            info!("Extracted quote: {}", quote);
                            info!("Extracted episode: {}", episode);
                            info!("Extracted character: {}", character);
                            
                            if !quote.is_empty() {
                                // Try to extract season and episode numbers
                                let mut search_terms = Vec::new();
                                
                                // Try to extract season and episode numbers from the episode string
                                if let Some(season_episode) = extract_season_episode_from_text(episode) {
                                    info!("Extracted season/episode: {}", season_episode);
                                    
                                    // First, try searching by season and episode if available
                                    info!("Trying search by season/episode: {}", season_episode);
                                    match self.morbotron_client.search(&season_episode).await {
                                        Ok(Some(result)) => {
                                            // For season/episode searches, we'll be more lenient
                                            let score = 0.9; // High score for episode match
                                            info!("Found result by season/episode with score {:.2}", score);
                                            
                                            // Return this result immediately - it's the most reliable method
                                            return Ok(Some(result));
                                        },
                                        _ => {
                                            info!("No results found for season/episode search");
                                        }
                                    }
                                    
                                    // Add the season/episode as a search term for later
                                    search_terms.push(season_episode);
                                }
                                
                                // Add the quote as a search term
                                search_terms.push(quote.to_string());
                                
                                // Add individual words from the quote as search terms
                                let quote_words: Vec<&str> = quote.split_whitespace().collect();
                                if quote_words.len() >= 3 {
                                    // Try combinations of 3 consecutive words
                                    for i in 0..quote_words.len() - 2 {
                                        let three_word_term = format!("{} {} {}", 
                                            quote_words[i], quote_words[i+1], quote_words[i+2]);
                                        search_terms.push(three_word_term);
                                    }
                                }
                                
                                // Try each search term with Morbotron
                                let mut results = Vec::new();
                                
                                for term in &search_terms {
                                    info!("Trying search term from Gemini API: {}", term);
                                    match self.morbotron_client.search(term).await {
                                        Ok(Some(result)) => {
                                            // For Gemini-provided quotes, we'll be more lenient with filtering
                                            let contains_terms = self.result_contains_search_terms(&result, query);
                                            let score = self.calculate_total_score(&result, query, term);
                                            
                                            if contains_terms {
                                                info!("Result contains search terms, adding with score {:.2}", score);
                                                results.push((result, score));
                                            } else {
                                                // For the primary Gemini response, accept it even if it doesn't contain
                                                // all search terms, but with a lower score
                                                let adjusted_score = score * 0.8;
                                                info!("Result doesn't contain all search terms, but accepting with adjusted score {:.2}", adjusted_score);
                                                results.push((result, adjusted_score));
                                            }
                                        },
                                        _ => {}
                                    }
                                }
                                
                                // If we have results, return the best one
                                if !results.is_empty() {
                                    // Sort by score (descending)
                                    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                                    info!("Found {} results from Gemini API, returning best match with score {:.2}", 
                                          results.len(), results[0].1);
                                    return Ok(Some(results[0].0.clone()));
                                }
                            }
                        },
                        Err(e) => {
                            // If JSON parsing fails, try to extract the quote directly
                            info!("Failed to parse Gemini response as JSON: {}", e);
                            
                            // Try to extract a quote from the response
                            let mut search_terms = Vec::new();
                            
                            // Look for quotes in the response
                            if let Some(quote) = self.extract_quote_from_text(&response) {
                                search_terms.push(quote);
                            }
                            
                            // If we couldn't extract a quote, use the whole response
                            if search_terms.is_empty() {
                                // Clean up the response
                                let cleaned = response
                                    .replace("```json", "")
                                    .replace("```", "")
                                    .trim()
                                    .to_string();
                                
                                search_terms.push(cleaned);
                            }
                            
                            // Try each search term with Morbotron
                            let mut results = Vec::new();
                            
                            for term in &search_terms {
                                info!("Trying search term from Gemini API (non-JSON): {}", term);
                                match self.morbotron_client.search(term).await {
                                    Ok(Some(result)) => {
                                        // For Gemini-provided quotes, we'll be more lenient with filtering
                                        let contains_terms = self.result_contains_search_terms(&result, query);
                                        let score = self.calculate_total_score(&result, query, term);
                                        
                                        if contains_terms {
                                            info!("Result contains search terms, adding with score {:.2}", score);
                                            results.push((result, score));
                                        } else {
                                            // For the primary Gemini response, accept it even if it doesn't contain
                                            // all search terms, but with a lower score
                                            if term == &search_terms[0] {
                                                let adjusted_score = score * 0.8;
                                                info!("Result doesn't contain all search terms, but accepting Gemini's primary response with adjusted score {:.2}", adjusted_score);
                                                results.push((result, adjusted_score));
                                            } else {
                                                info!("Result doesn't contain search terms, skipping");
                                            }
                                        }
                                    },
                                    _ => {}
                                }
                            }
                            
                            // If we have results, return the best one
                            if !results.is_empty() {
                                // Sort by score (descending)
                                results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                                info!("Found {} results from Gemini API (non-JSON), returning best match with score {:.2}", 
                                      results.len(), results[0].1);
                                return Ok(Some(results[0].0.clone()));
                            }
                        }
                    }
                }
            },
            Err(e) => {
                error!("Error with Gemini API: {}, falling back to direct search", e);
            }
        }
        
        // If we still have no results, try a direct search with the original query as a last resort
        info!("No results from enhanced search, falling back to direct search with filtering");
        match self.morbotron_client.search(query).await {
            Ok(Some(result)) => {
                if self.result_contains_search_terms(&result, query) {
                    Ok(Some(result))
                } else {
                    info!("Direct search result doesn't contain search terms, returning None");
                    Ok(None)
                }
            },
            other => other
        }
    }

    // Calculate a total score for a result based on multiple factors
    fn calculate_total_score(&self, result: &MorbotronResult, query: &str, search_term: &str) -> f32 {
        // Calculate relevance score based on how well the caption matches the original query
        let relevance_score = self.calculate_relevance_score(result, query);
        
        // Calculate popularity score
        let popularity_score = self.calculate_popularity_score(result);
        
        // Calculate quote match score - how well the caption matches the search term
        let quote_match_score = self.calculate_quote_match_score(result, search_term);
        
        // Calculate exact word match score - how many words from the original query are in the caption
        let exact_word_match_score = self.calculate_exact_word_match_score(result, query);
        
        // Calculate total score (weighted combination of all scores)
        let total_score = (relevance_score * 0.25) + 
                         (popularity_score * 0.15) + 
                         (quote_match_score * 0.25) + 
                         (exact_word_match_score * 0.35);
        
        total_score
    }
    
    // Check if a result contains the search terms
    fn result_contains_search_terms(&self, result: &MorbotronResult, query: &str) -> bool {
        let query_lower = query.to_lowercase();
        let caption_lower = result.caption.to_lowercase();
        let episode_title_lower = result.episode_title.to_lowercase();
        
        // Split query into words
        let query_words: Vec<&str> = query_lower.split_whitespace().collect();
        if query_words.is_empty() {
            return true; // Empty query matches everything
        }
        
        // Check if all words from the query appear in either the caption or episode title
        let all_words_in_caption = query_words.iter()
            .all(|&word| caption_lower.contains(word));
            
        let all_words_in_title = query_words.iter()
            .all(|&word| episode_title_lower.contains(word));
            
        // Return true if all words are found in either the caption or title
        all_words_in_caption || all_words_in_title
    }
    
    // Extract a quote from text (text between quotation marks)
    fn extract_quote_from_text(&self, text: &str) -> Option<String> {
        let re = Regex::new(r#""([^"]+)""#).ok()?;
        if let Some(caps) = re.captures(text) {
            if let Some(quote) = caps.get(1) {
                return Some(quote.as_str().to_string());
            }
        }
        None
    }
    
    // Extract episode information from text
    fn extract_episode_info(&self, text: &str) -> Option<String> {
        // Look for season and episode information
        let re = Regex::new(r"(?i)season\s+(\d+)\s+episode\s+(\d+)").ok()?;
        if let Some(caps) = re.captures(text) {
            if let (Some(season), Some(episode)) = (caps.get(1), caps.get(2)) {
                return Some(format!("S{:02}E{:02}", 
                    season.as_str().parse::<u32>().unwrap_or(0),
                    episode.as_str().parse::<u32>().unwrap_or(0)));
            }
        }
        None
    }
    
    // Extract potential quotes from title and snippet
    fn extract_potential_quotes(&self, title: &str, snippet: &str) -> Vec<String> {
        let mut quotes = Vec::new();
        
        // Extract sentences that might be quotes
        let combined_text = format!("{} {}", title, snippet);
        let sentences = combined_text.split(|c| c == '.' || c == '!' || c == '?')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<&str>>();
            
        for sentence in sentences {
            if sentence.len() > 5 && sentence.len() < 100 {
                quotes.push(sentence.to_string());
            }
        }
        
        quotes
    }

    // Calculate relevance score based on how well the caption matches the original query
    fn calculate_relevance_score(&self, result: &MorbotronResult, query: &str) -> f32 {
        // Use Jaro-Winkler similarity for fuzzy matching
        let similarity = jaro_winkler(&result.caption.to_lowercase(), &query.to_lowercase());
        similarity as f32
    }
    
    // Calculate popularity score (placeholder implementation)
    fn calculate_popularity_score(&self, _result: &MorbotronResult) -> f32 {
        // In a real implementation, this would use popularity data
        // For now, return a default value
        0.5
    }
    
    // Calculate quote match score - how well the caption matches the search term
    fn calculate_quote_match_score(&self, result: &MorbotronResult, search_term: &str) -> f32 {
        // Use Jaro-Winkler similarity for fuzzy matching
        let similarity = jaro_winkler(&result.caption.to_lowercase(), &search_term.to_lowercase());
        similarity as f32
    }
    
    // Calculate a score based on how many exact words from the query are in the caption
    fn calculate_exact_word_match_score(&self, result: &MorbotronResult, query: &str) -> f32 {
        let query_lower = query.to_lowercase();
        let caption_lower = result.caption.to_lowercase();
        let episode_title_lower = result.episode_title.to_lowercase();
        
        // Split query into words
        let query_words: Vec<&str> = query_lower.split_whitespace().collect();
        if query_words.is_empty() {
            return 0.0;
        }
        
        // Count how many words from the query appear in the caption
        let caption_matching_words = query_words.iter()
            .filter(|&word| caption_lower.contains(word))
            .count();
            
        // Count how many words from the query appear in the episode title
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
