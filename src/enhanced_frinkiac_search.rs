use anyhow::Result;
use tracing::{info, error};
use crate::gemini_api::GeminiClient;
use crate::frinkiac::{FrinkiacClient, FrinkiacResult};
use crate::google_search::GoogleSearchClient;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use strsim::jaro_winkler;
use regex::Regex;

// A struct to hold search results with metadata for ranking
#[derive(Debug)]
struct RankedFrinkiacResult {
    result: FrinkiacResult,
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
const GEMINI_FRINKIAC_PROMPT: &str = r#"
You are helping to search for Simpsons quotes and scenes. Given a user's search query, generate 3-5 possible exact phrases or quotes from The Simpsons that best match what the user is looking for.

Focus on famous, memorable, and popular quotes that match the semantic meaning of the query, not just the exact words. Consider these guidelines:

1. Prioritize quotes from seasons 3-8 (the "golden era" of The Simpsons)
2. Include character names if they're relevant (e.g., "Homer", "Bart", "Mr. Burns")
3. Focus on shorter, more iconic quotes rather than long dialogue
4. Include the exact quote as it appears in the show, not paraphrased versions
5. If the user is clearly referencing a specific scene or episode, provide quotes from that scene

Examples:
- Query: "extra b typo" → "What's that extra B for?", "That's a typo.", "BBQ"
- Query: "stupid sexy flanders" → "Stupid sexy Flanders!", "Feels like I'm wearing nothing at all!"
- Query: "everything's coming up milhouse" → "Everything's coming up Milhouse!"
- Query: "dental plan" → "Dental plan", "Lisa needs braces"
- Query: "steamed hams" → "Steamed hams", "Aurora Borealis", "Superintendent Chalmers"
- Query: "i'm in danger" → "I'm in danger", "Ralph Wiggum chuckles"
- Query: "spider pig" → "Spider pig, spider pig", "Does whatever a spider pig does"

Return ONLY the quotes, one per line, without any explanations or additional text. Prioritize exact quotes that are well-known and popular.

User query: {query}
"#;

pub struct EnhancedFrinkiacSearch {
    gemini_client: GeminiClient,
    frinkiac_client: FrinkiacClient,
    google_client: GoogleSearchClient,
}

impl EnhancedFrinkiacSearch {
    pub fn new(gemini_client: GeminiClient, frinkiac_client: FrinkiacClient, google_client: GoogleSearchClient) -> Self {
        Self {
            gemini_client,
            frinkiac_client,
            google_client,
        }
    }

    // Main search function that uses search engine to enhance the search
    pub async fn search(&self, query: &str) -> Result<Option<FrinkiacResult>> {
        info!("Enhanced Frinkiac search for: {}", query);
        
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
        if !all_terms.contains(&query.to_string()) {
            all_terms.push(query.to_string());
        }
        
        // Try each term and collect ALL results
        let mut results = Vec::new();
        
        for term in &all_terms {
            info!("Trying search term: {}", term);
            match self.frinkiac_client.search(term).await {
                Ok(Some(result)) => {
                    // Calculate relevance score based on how well the caption matches the original query
                    let relevance_score = self.calculate_relevance_score(&result, query);
                    
                    // Calculate popularity score
                    let popularity_score = self.calculate_popularity_score(&result);
                    
                    // Calculate quote match score - how well the caption matches the search term
                    let quote_match_score = self.calculate_quote_match_score(&result, term);
                    
                    // Calculate exact word match score - how many words from the original query are in the caption
                    let exact_word_match_score = self.calculate_exact_word_match_score(&result, query);
                    
                    // Calculate total score (weighted combination of all scores)
                    // Give higher weight to search engine and Gemini terms
                    let priority_bonus = if term == query { 0.0 } else { 0.1 };
                    let total_score = (relevance_score * 0.25) + 
                                     (popularity_score * 0.15) + 
                                     (quote_match_score * 0.25) + 
                                     (exact_word_match_score * 0.35) +
                                     priority_bonus;
                    
                    info!("Found result for '{}' with scores - relevance: {:.2}, popularity: {:.2}, quote match: {:.2}, exact word match: {:.2}, total: {:.2}", 
                          term, relevance_score, popularity_score, quote_match_score, exact_word_match_score, total_score);
                    
                    results.push(RankedFrinkiacResult {
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
        self.frinkiac_client.search(query).await
    }
    
    // Calculate a score based on how many exact words from the query are in the caption
    fn calculate_exact_word_match_score(&self, result: &FrinkiacResult, query: &str) -> f32 {
        let query_lower = query.to_lowercase();
        let caption_lower = result.caption.to_lowercase();
        
        // Split query into words
        let query_words: Vec<&str> = query_lower.split_whitespace().collect();
        if query_words.is_empty() {
            return 0.0;
        }
        
        // Count how many words from the query appear in the caption
        let matching_words = query_words.iter()
            .filter(|&word| caption_lower.contains(word))
            .count();
        
        // Calculate score based on proportion of matching words
        let proportion = matching_words as f32 / query_words.len() as f32;
        
        // Give a bonus if all words match
        if matching_words == query_words.len() {
            proportion * 1.5
        } else {
            proportion
        }
    }
    
    // Use Google search to find Simpsons quotes related to the query
    async fn find_quotes_via_search(&self, query: &str) -> Result<Vec<String>> {
        // Try multiple search queries to increase chances of finding good quotes
        let search_queries = [
            format!("simpsons quote \"{}\"", query),
            format!("simpsons scene \"{}\"", query),
            format!("famous simpsons quote {}", query),
            format!("simpsons {} quote", query),  // Added this variation
            format!("\"{}\" simpsons episode", query),  // Added this variation
        ];
        
        let mut all_quotes = Vec::new();
        
        // Try each search query
        for search_query in &search_queries {
            info!("Searching for Simpsons quotes with query: {}", search_query);
            
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
                    
                    // Also add the exact search query as a potential search term
                    // This helps with specific phrases like "extra b typo"
                    if !quotes.contains(&query.to_string()) {
                        quotes.push(query.to_string());
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
        if !query.is_empty() && !all_quotes.contains(&query.to_string()) {
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
        
        // Check if the text contains Simpsons-related keywords
        if combined_text.to_lowercase().contains("simpsons") {
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
                let character_names = ["homer", "bart", "lisa", "marge", "burns", "flanders", "ralph", "milhouse"];
                let contains_character = character_names.iter().any(|&name| sentence.to_lowercase().contains(name));
                
                // Check for quote indicators
                let quote_indicators = ["says", "said", "quote", "quotes", "line", "scene"];
                let contains_indicator = quote_indicators.iter().any(|&ind| sentence.to_lowercase().contains(ind));
                
                // If it contains a character name or quote indicator, it might be a quote or description of a quote
                if contains_character || contains_indicator {
                    quotes.push(sentence.to_string());
                }
            }
            
            // If we couldn't find any quotes using the above methods, try to extract key phrases
            if quotes.is_empty() {
                // Look for phrases that might be quotes based on context
                if let Some(key_phrase) = self.extract_key_phrase_from_text(&combined_text) {
                    quotes.push(key_phrase);
                }
            }
        }
        
        quotes
    }
    
    // Extract a key phrase that might be a quote based on context
    fn extract_key_phrase_from_text(&self, text: &str) -> Option<String> {
        let text_lower = text.to_lowercase();
        
        // Look for phrases that are likely to be quotes
        let indicators = [
            "famous quote", "memorable quote", "popular quote", "iconic quote",
            "famous line", "memorable line", "popular line", "iconic line",
            "says", "said", "utters", "uttered", "exclaims", "exclaimed"
        ];
        
        for indicator in &indicators {
            if let Some(index) = text_lower.find(indicator) {
                // Look for a reasonable phrase after the indicator
                let after_indicator = &text[index + indicator.len()..];
                let phrase = after_indicator.trim_start_matches(|c: char| !c.is_alphanumeric())
                                          .split(&['.', '!', '?', ';', '\n'])
                                          .next()
                                          .unwrap_or("")
                                          .trim();
                
                // If we found a reasonable phrase, return it
                if phrase.len() > 5 && phrase.len() < 100 {
                    return Some(phrase.to_string());
                }
            }
        }
        
        // If we couldn't find a phrase using indicators, look for phrases in quotes
        // (This is a fallback if the extract_quote_from_text method didn't find anything)
        let quote_patterns = [
            ("\"", "\""), ("'", "'")
        ];
        
        for (start_quote, end_quote) in &quote_patterns {
            if let Some(start_idx) = text.find(start_quote) {
                if let Some(end_idx) = text[start_idx + start_quote.len()..].find(end_quote) {
                    let quote = &text[start_idx + start_quote.len()..start_idx + start_quote.len() + end_idx];
                    if quote.len() > 3 && quote.len() < 100 {
                        return Some(quote.to_string());
                    }
                }
            }
        }
        
        None
    }
    
    // Extract a quote from text (looking for quotation marks)
    fn extract_quote_from_text(&self, text: &str) -> Option<String> {
        // Try different quote patterns
        let patterns = [
            r#""([^"]+)""#,           // Standard double quotes
            r#"'([^']+)'"#,           // Single quotes
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
                        
                        // Skip quotes that are likely not from The Simpsons
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
        let prompt = GEMINI_FRINKIAC_PROMPT.replace("{query}", query);
        
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
    
    // Check if a result is relevant to the query
    fn is_relevant_result(&self, result: &FrinkiacResult, query: &str) -> bool {
        // Simple relevance check - could be more sophisticated
        let query_lower = query.to_lowercase();
        let caption_lower = result.caption.to_lowercase();
        
        // Check if any word from the query appears in the caption
        query_lower.split_whitespace().any(|word| caption_lower.contains(word))
    }
    
    // Calculate a relevance score for a result based on how well it matches the query
    fn calculate_relevance_score(&self, result: &FrinkiacResult, query: &str) -> f32 {
        let query_lower = query.to_lowercase();
        let caption_lower = result.caption.to_lowercase();
        let episode_title_lower = result.episode_title.to_lowercase();
        
        // Count how many words from the query appear in the caption
        let query_words: Vec<&str> = query_lower.split_whitespace().collect();
        
        let matching_words = query_words.iter()
            .filter(|&word| caption_lower.contains(word))
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
        
        // Bonus if ALL words in the query are found in the caption
        let all_words_bonus = if matching_words == query_words.len() && !query_words.is_empty() {
            0.4 // Significant bonus for matching all words
        } else {
            0.0
        };
        
        // Combine scores (capped at 1.0)
        (word_match_score * 0.4 + exact_phrase_bonus + consecutive_words_bonus + episode_title_bonus + all_words_bonus).min(1.0)
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
    fn calculate_popularity_score(&self, result: &FrinkiacResult) -> f32 {
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
        
        // 2. Season number (earlier seasons tend to have more iconic quotes)
        // Seasons 3-8 are generally considered the "golden era"
        let season_score = match result.season {
            3..=8 => 0.9,  // Golden era
            9..=12 => 0.7, // Still good
            1..=2 => 0.6,  // Early seasons
            13..=15 => 0.5, // Later seasons
            _ => 0.3       // Much later seasons
        };
        
        // 3. Check for iconic characters in the caption
        let iconic_characters = [
            "homer", "bart", "lisa", "marge", "burns", "flanders", 
            "troy mcclure", "ralph", "comic book guy", "nelson", "milhouse"
        ];
        
        let caption_lower = result.caption.to_lowercase();
        let character_score = if iconic_characters.iter().any(|&c| caption_lower.contains(c)) {
            0.8
        } else {
            0.5
        };
        
        // 4. Check for iconic phrases
        let iconic_phrases = [
            "d'oh", "eat my shorts", "don't have a cow", "excellent", 
            "ha ha", "stupid", "why you little", "mmm", "hi diddly ho",
            "worst", "ever", "perfectly cromulent", "embiggen"
        ];
        
        let phrase_score = if iconic_phrases.iter().any(|&p| caption_lower.contains(p)) {
            0.9
        } else {
            0.5
        };
        
        // Combine scores with different weights
        (length_score * 0.3) + 
        (season_score * 0.3) + 
        (character_score * 0.2) + 
        (phrase_score * 0.2)
    }
    
    // Calculate how well the caption matches the search term
    fn calculate_quote_match_score(&self, result: &FrinkiacResult, search_term: &str) -> f32 {
        let term_lower = search_term.to_lowercase();
        let caption_lower = result.caption.to_lowercase();
        
        // Use Jaro-Winkler similarity for fuzzy matching
        let similarity = jaro_winkler(&term_lower, &caption_lower) as f32;
        
        // Check for exact substring match
        let contains_term = caption_lower.contains(&term_lower);
        let contains_bonus = if contains_term { 0.5 } else { 0.0 }; // Increased from 0.3 to 0.5
        
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
        
        // Add a bonus if ALL words in the search term are found in the caption
        let all_words_bonus = if matching_words == term_words.len() && !term_words.is_empty() {
            0.4 // Significant bonus for matching all words
        } else {
            0.0
        };
        
        // Combine scores (capped at 1.0)
        (similarity * 0.3 + contains_bonus + word_match_score * 0.4 + all_words_bonus).min(1.0)
    }
}
