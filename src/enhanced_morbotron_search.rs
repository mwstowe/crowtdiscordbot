use anyhow::Result;
use tracing::{info, error};
use crate::gemini_api::GeminiClient;
use crate::morbotron::{MorbotronClient, MorbotronResult};
use crate::google_search::GoogleSearchClient;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use strsim::jaro_winkler;
use regex::Regex;

// A struct to hold search results with metadata for ranking
#[derive(Debug)]
struct RankedMorbotronResult {
    result: MorbotronResult,
    relevance_score: f32,
    popularity_score: f32,
    total_score: f32,
}

// A struct to hold the popularity data for quotes
#[derive(Debug, Deserialize, Serialize)]
struct QuotePopularity {
    quotes: HashMap<String, u32>, // Maps quote text to popularity score
}

// Constants for the Gemini prompt
const GEMINI_MORBOTRON_PROMPT: &str = r#"
You are helping to search for Futurama quotes and scenes. Given a user's search query, generate 3-5 possible exact phrases or quotes from Futurama that best match what the user is looking for.

Focus on famous, memorable, and popular quotes that match the semantic meaning of the query, not just the exact words. Consider these guidelines:

1. Include character names if they're relevant (e.g., "Fry", "Leela", "Bender", "Professor Farnsworth")
2. Focus on shorter, more iconic quotes rather than long dialogue
3. Include the exact quote as it appears in the show, not paraphrased versions
4. If the user is clearly referencing a specific scene or episode, provide quotes from that scene

Examples:
- Query: "bite my shiny metal" → "Bite my shiny metal ass!"
- Query: "good news everyone" → "Good news, everyone!"
- Query: "death by snu snu" → "Death by snu-snu!"
- Query: "i don't want to live on this planet" → "I don't want to live on this planet anymore"
- Query: "blackjack and hookers" → "I'll make my own theme park with blackjack and hookers"
- Query: "hypnotoad" → "ALL GLORY TO THE HYPNOTOAD"
- Query: "shut up and take my money" → "Shut up and take my money!"

Return ONLY the quotes, one per line, without any explanations or additional text. Prioritize exact quotes that are well-known and popular.

User query: {query}
"#;

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
        
        // First, use Google search to find potential quotes - this is our best approach
        let search_terms = self.find_quotes_via_search(query).await?;
        info!("Found {} potential quotes via search", search_terms.len());
        
        // If search engine didn't return useful results, try Gemini
        let enhanced_terms = if search_terms.is_empty() {
            info!("No quotes found via search, trying Gemini");
            self.generate_search_terms(query).await?
        } else {
            search_terms
        };
        
        // Add the original query as a fallback, but with lower priority
        let mut all_terms = enhanced_terms.clone();
        all_terms.push(query.to_string());
        
        // Try each term and collect ALL results
        let mut results = Vec::new();
        
        for term in &all_terms {
            info!("Trying search term: {}", term);
            match self.morbotron_client.search(term).await {
                Ok(Some(result)) => {
                    // Calculate relevance score based on how well the caption matches the query
                    let relevance_score = self.calculate_relevance_score(&result, query);
                    
                    // Calculate popularity score
                    let popularity_score = self.calculate_popularity_score(&result);
                    
                    // Calculate quote match score - how well the caption matches the search term
                    let quote_match_score = self.calculate_quote_match_score(&result, term);
                    
                    // Calculate total score (weighted combination of relevance, popularity, and quote match)
                    // Give higher weight to search engine and Gemini terms
                    let priority_bonus = if term == query { 0.0 } else { 0.2 };
                    let total_score = (relevance_score * 0.3) + 
                                     (popularity_score * 0.2) + 
                                     (quote_match_score * 0.3) + 
                                     priority_bonus;
                    
                    info!("Found result for '{}' with scores - relevance: {:.2}, popularity: {:.2}, quote match: {:.2}, total: {:.2}", 
                          term, relevance_score, popularity_score, quote_match_score, total_score);
                    
                    results.push(RankedMorbotronResult {
                        result,
                        relevance_score,
                        popularity_score,
                        total_score,
                    });
                },
                Ok(None) => {
                    info!("No results for term: {}", term);
                },
                Err(e) => {
                    error!("Error searching with term '{}': {}", term, e);
                }
            }
        }
        
        // If we have results, sort them by total score and return the best one
        if !results.is_empty() {
            // Sort by total score (descending)
            results.sort_by(|a, b| b.total_score.partial_cmp(&a.total_score).unwrap_or(std::cmp::Ordering::Equal));
            
            info!("Found {} results, returning best match with score {:.2}", 
                  results.len(), results[0].total_score);
            
            return Ok(Some(results[0].result.clone()));
        }
        
        // If we still have no results, try a direct search with the original query as a last resort
        info!("No results from enhanced search, falling back to direct search");
        self.morbotron_client.search(query).await
    }
    
    // Use Google search to find Futurama quotes related to the query
    async fn find_quotes_via_search(&self, query: &str) -> Result<Vec<String>> {
        // Try multiple search queries to increase chances of finding good quotes
        let search_queries = [
            format!("futurama quote \"{}\"", query),
            format!("futurama scene \"{}\"", query),
            format!("famous futurama quote {}", query),
        ];
        
        let mut all_quotes = Vec::new();
        
        // Try each search query
        for search_query in &search_queries {
            info!("Searching for Futurama quotes with query: {}", search_query);
            
            // Use the existing Google search client
            match self.google_client.search(search_query).await {
                Ok(Some(result)) => {
                    info!("Found search result: {} - {}", result.title, result.snippet);
                    
                    // Extract potential quotes from the search result
                    let mut quotes = Vec::new();
                    
                    // Look for quotes in the title
                    if let Some(quote) = self.extract_quote_from_text(&result.title) {
                        quotes.push(quote);
                    }
                    
                    // Look for quotes in the snippet
                    if let Some(quote) = self.extract_quote_from_text(&result.snippet) {
                        quotes.push(quote);
                    }
                    
                    // If we couldn't find quotes in quotation marks, extract potential phrases
                    if quotes.is_empty() {
                        // Extract phrases that might be quotes
                        let potential_quotes = self.extract_potential_quotes(&result.title, &result.snippet);
                        quotes.extend(potential_quotes);
                    }
                    
                    all_quotes.extend(quotes);
                },
                Ok(None) => {
                    info!("No search results found for query: {}", search_query);
                },
                Err(e) => {
                    error!("Error searching for quotes: {}", e);
                }
            }
            
            // If we found some quotes, no need to try more search queries
            if !all_quotes.is_empty() {
                break;
            }
        }
        
        // Add the original query as a fallback
        if !query.is_empty() {
            all_quotes.push(query.to_string());
        }
        
        // Remove duplicates
        all_quotes.sort();
        all_quotes.dedup();
        
        info!("Extracted {} quotes from search results", all_quotes.len());
        Ok(all_quotes)
    }
    
    // Extract potential quotes from text
    fn extract_potential_quotes(&self, title: &str, snippet: &str) -> Vec<String> {
        let mut quotes = Vec::new();
        
        // Combine title and snippet for analysis
        let combined_text = format!("{} {}", title, snippet);
        
        // Check if the text contains Futurama-related keywords
        if combined_text.to_lowercase().contains("futurama") {
            // Split by sentence endings and other punctuation
            let potential_sentences: Vec<&str> = combined_text
                .split(&['.', '!', '?', ';'])
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();
            
            for sentence in potential_sentences {
                // Skip very long sentences (unlikely to be quotes)
                if sentence.len() > 150 {
                    continue;
                }
                
                // Skip sentences that are likely not quotes
                if sentence.to_lowercase().contains("episode") || 
                   sentence.to_lowercase().contains("season") ||
                   sentence.to_lowercase().contains("wikipedia") ||
                   sentence.to_lowercase().contains("click") {
                    continue;
                }
                
                // Check for character names to identify potential quotes
                let character_names = ["fry", "leela", "bender", "professor", "zoidberg", "amy", "hermes", "zapp", "kif"];
                let contains_character = character_names.iter().any(|&name| sentence.to_lowercase().contains(name));
                
                // Check for quote indicators
                let quote_indicators = ["says", "said", "quote", "quotes", "line", "scene"];
                let contains_indicator = quote_indicators.iter().any(|&ind| sentence.to_lowercase().contains(ind));
                
                // If it contains a character name or quote indicator, it might be a quote or description of a quote
                if contains_character || contains_indicator {
                    quotes.push(sentence.to_string());
                }
            }
        }
        
        quotes
    }
    
    // Extract a quote from text (looking for quotation marks)
    fn extract_quote_from_text(&self, text: &str) -> Option<String> {
        // Try different quote patterns
        let patterns = [
            r#""([^"]+)""#,           // Standard double quotes
            r#"'([^']+)'"#,           // Single quotes
            r#""([^"]+)""#,           // Curly double quotes
            r#"'([^']+)'"#,           // Curly single quotes
            r#"«([^»]+)»"#,           // Guillemets
            r#"„([^"]+)""#,           // German quotes
            r#"「([^」]+)」"#,         // Japanese quotes
        ];
        
        for pattern in &patterns {
            if let Ok(re) = Regex::new(pattern) {
                for caps in re.captures_iter(text) {
                    if let Some(quote) = caps.get(1) {
                        let quote_text = quote.as_str().trim();
                        
                        // Skip very short quotes
                        if quote_text.len() < 3 {
                            continue;
                        }
                        
                        // Skip quotes that are likely not from Futurama
                        if quote_text.to_lowercase().contains("episode") || 
                           quote_text.to_lowercase().contains("season") ||
                           quote_text.to_lowercase().contains("wikipedia") {
                            continue;
                        }
                        
                        return Some(quote_text.to_string());
                    }
                }
            }
        }
        
        None
    }
    
    // Generate better search terms using Gemini API
    async fn generate_search_terms(&self, query: &str) -> Result<Vec<String>> {
        let prompt = GEMINI_MORBOTRON_PROMPT.replace("{query}", query);
        
        match self.gemini_client.generate_response(&prompt).await {
            Ok(response) => {
                // Parse the response into individual search terms
                let terms: Vec<String> = response
                    .lines()
                    .map(|line| line.trim().to_string())
                    .filter(|line| !line.is_empty())
                    .collect();
                
                Ok(terms)
            },
            Err(e) => {
                error!("Error generating search terms with Gemini: {}", e);
                // Return an empty vector, which will cause the search to fall back to the original query
                Ok(Vec::new())
            }
        }
    }
    
    // Calculate a relevance score for a result based on how well it matches the query
    fn calculate_relevance_score(&self, result: &MorbotronResult, query: &str) -> f32 {
        let query_lower = query.to_lowercase();
        let caption_lower = result.caption.to_lowercase();
        let episode_title_lower = result.episode_title.to_lowercase();
        
        // Count how many words from the query appear in the caption
        let query_words: Vec<&str> = query_lower.split_whitespace().collect();
        let matching_words = query_words.iter()
            .filter(|word| caption_lower.contains(*word))
            .count();
        
        // Calculate a score based on the proportion of matching words
        if query_words.is_empty() {
            return 0.5; // Default score if query is empty
        }
        
        // Base score on matching words
        let word_match_score = matching_words as f32 / query_words.len() as f32;
        
        // Bonus for exact phrase match
        let exact_phrase_bonus = if caption_lower.contains(&query_lower) {
            0.5
        } else {
            0.0
        };
        
        // Bonus for consecutive words matching
        let consecutive_words_bonus = self.calculate_consecutive_words_bonus(&query_lower, &caption_lower);
        
        // Bonus if the query appears in the episode title
        let episode_title_bonus = if episode_title_lower.contains(&query_lower) {
            0.3
        } else if query_words.iter().any(|word| episode_title_lower.contains(*word)) {
            0.1
        } else {
            0.0
        };
        
        // Combine scores (capped at 1.0)
        (word_match_score + exact_phrase_bonus + consecutive_words_bonus + episode_title_bonus).min(1.0)
    }
    
    // Calculate bonus for consecutive words matching
    fn calculate_consecutive_words_bonus(&self, query: &str, caption: &str) -> f32 {
        let query_words: Vec<&str> = query.split_whitespace().collect();
        
        // If query has only one word, no consecutive bonus applies
        if query_words.len() <= 1 {
            return 0.0;
        }
        
        // Check for consecutive pairs of words
        let mut max_consecutive = 0;
        let mut current_consecutive = 0;
        
        for i in 0..query_words.len() - 1 {
            let pair = format!("{} {}", query_words[i], query_words[i + 1]);
            if caption.contains(&pair) {
                current_consecutive += 1;
                max_consecutive = max_consecutive.max(current_consecutive);
            } else {
                current_consecutive = 0;
            }
        }
        
        // Scale bonus based on number of consecutive pairs found
        // and the total possible consecutive pairs
        let max_possible_consecutive = query_words.len() - 1;
        
        if max_consecutive > 0 {
            0.3 * (max_consecutive as f32 / max_possible_consecutive as f32)
        } else {
            0.0
        }
    }
    
    // Calculate a popularity score for a result
    fn calculate_popularity_score(&self, result: &MorbotronResult) -> f32 {
        // Multiple factors contribute to popularity score
        
        // 1. Caption length (shorter captions are often more iconic/memorable)
        let caption_length = result.caption.len();
        let length_score = if caption_length <= 30 {
            0.9 // Short quotes (likely more iconic)
        } else if caption_length <= 60 {
            0.7 // Medium quotes
        } else if caption_length <= 100 {
            0.5 // Longer quotes
        } else {
            0.3 // Very long quotes (likely less iconic)
        };
        
        // 2. Check for iconic characters in the caption
        let iconic_characters = [
            "fry", "leela", "bender", "professor", "zoidberg", "amy", "hermes", "zapp", "kif",
            "nibbler", "mom", "hypnotoad", "scruffy", "nixon", "calculon"
        ];
        
        let caption_lower = result.caption.to_lowercase();
        let character_score = if iconic_characters.iter().any(|&c| caption_lower.contains(c)) {
            0.8
        } else {
            0.5
        };
        
        // 3. Check for iconic phrases
        let iconic_phrases = [
            "bite my shiny", "good news everyone", "death by snu", "shut up and take my money",
            "hypnotoad", "blackjack and hookers", "i'm 40% ", "woop woop woop", "why not zoidberg",
            "to shreds you say", "i don't want to live on this planet", "futurama"
        ];
        
        let phrase_score = if iconic_phrases.iter().any(|&p| caption_lower.contains(p)) {
            0.9
        } else {
            0.5
        };
        
        // Combine scores with different weights
        (length_score * 0.4) + 
        (character_score * 0.3) + 
        (phrase_score * 0.3)
    }
    
    // Calculate how well the caption matches the search term
    fn calculate_quote_match_score(&self, result: &MorbotronResult, search_term: &str) -> f32 {
        let term_lower = search_term.to_lowercase();
        let caption_lower = result.caption.to_lowercase();
        
        // Use Jaro-Winkler similarity for fuzzy matching
        let similarity = jaro_winkler(&term_lower, &caption_lower) as f32;
        
        // Check for exact substring match
        let contains_term = caption_lower.contains(&term_lower);
        let contains_bonus = if contains_term { 0.3 } else { 0.0 };
        
        // Check for word-by-word matches
        let term_words: Vec<&str> = term_lower.split_whitespace().collect();
        let caption_words: Vec<&str> = caption_lower.split_whitespace().collect();
        
        let matching_words = term_words.iter()
            .filter(|&word| caption_words.contains(word))
            .count();
            
        let word_match_score = if term_words.is_empty() {
            0.0
        } else {
            matching_words as f32 / term_words.len() as f32
        };
        
        // Combine scores (capped at 1.0)
        (similarity * 0.5 + contains_bonus + word_match_score * 0.2).min(1.0)
    }
}
