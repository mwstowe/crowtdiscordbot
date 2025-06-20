use anyhow::Result;
use tracing::{info, error};
use crate::gemini_api::GeminiClient;
use crate::frinkiac::{FrinkiacClient, FrinkiacResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

Focus on famous, memorable, and popular quotes that match the semantic meaning of the query, not just the exact words.

For example:
- If the user searches for "extra b typo", you might suggest "What's that extra B for? That's a typo."
- If the user searches for "stupid sexy flanders", you might suggest "Stupid sexy Flanders!", "Feels like I'm wearing nothing at all!"
- If the user searches for "everything's coming up milhouse", you might suggest "Everything's coming up Milhouse!"

Return ONLY the quotes, one per line, without any explanations or additional text. Prioritize exact quotes that are well-known and popular.

User query: {query}
"#;

pub struct EnhancedFrinkiacSearch {
    gemini_client: GeminiClient,
    frinkiac_client: FrinkiacClient,
    // We could add a cache here for popular quotes
}

impl EnhancedFrinkiacSearch {
    pub fn new(gemini_client: GeminiClient, frinkiac_client: FrinkiacClient) -> Self {
        Self {
            gemini_client,
            frinkiac_client,
        }
    }

    // Main search function that uses Gemini to enhance the search
    pub async fn search(&self, query: &str) -> Result<Option<FrinkiacResult>> {
        info!("Enhanced Frinkiac search for: {}", query);
        
        // First, try a direct search with the original query
        // This is for cases where the user's query is already a good match
        match self.frinkiac_client.search(query).await {
            Ok(Some(result)) => {
                // Check if the result is relevant by comparing the caption with the query
                if self.is_relevant_result(&result, query) {
                    info!("Found relevant result with direct search");
                    return Ok(Some(result));
                }
                info!("Direct search result wasn't relevant enough, trying enhanced search");
            },
            Ok(None) => {
                info!("No results from direct search, trying enhanced search");
            },
            Err(e) => {
                error!("Error in direct search: {}, trying enhanced search", e);
            }
        }
        
        // Use Gemini to generate better search terms
        let enhanced_terms = self.generate_search_terms(query).await?;
        info!("Generated {} enhanced search terms", enhanced_terms.len());
        
        if enhanced_terms.is_empty() {
            // If Gemini couldn't generate any terms, fall back to the original query
            info!("No enhanced terms generated, falling back to original query");
            return self.frinkiac_client.search(query).await;
        }
        
        // Try each enhanced term and collect results
        let mut results = Vec::new();
        for term in &enhanced_terms {
            info!("Trying enhanced search term: {}", term);
            match self.frinkiac_client.search(term).await {
                Ok(Some(result)) => {
                    // Calculate relevance score based on how well the caption matches the query
                    let relevance_score = self.calculate_relevance_score(&result, query);
                    
                    // Calculate popularity score (could be based on a predefined list of popular quotes)
                    let popularity_score = self.calculate_popularity_score(&result);
                    
                    // Calculate total score (weighted combination of relevance and popularity)
                    let total_score = (relevance_score * 0.7) + (popularity_score * 0.3);
                    
                    results.push(RankedFrinkiacResult {
                        result,
                        relevance_score,
                        popularity_score,
                        total_score,
                    });
                },
                Ok(None) => {
                    info!("No results for enhanced term: {}", term);
                },
                Err(e) => {
                    error!("Error searching with enhanced term '{}': {}", term, e);
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
        
        // If we still have no results, try the original query as a last resort
        info!("No results from enhanced search, falling back to original query");
        self.frinkiac_client.search(query).await
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
        
        // Combine scores (capped at 1.0)
        (word_match_score + exact_phrase_bonus).min(1.0)
    }
    
    // Calculate a popularity score for a result
    // This could be based on a predefined list of popular quotes or scenes
    fn calculate_popularity_score(&self, result: &FrinkiacResult) -> f32 {
        // For now, use a simple heuristic based on the length of the caption
        // Shorter captions are often more iconic/memorable
        // This is just a placeholder - ideally we'd have actual popularity data
        
        let caption_length = result.caption.len();
        
        // Score inversely proportional to length, with some bounds
        if caption_length <= 30 {
            0.9 // Short quotes (likely more iconic)
        } else if caption_length <= 60 {
            0.7 // Medium quotes
        } else if caption_length <= 100 {
            0.5 // Longer quotes
        } else {
            0.3 // Very long quotes (likely less iconic)
        }
        
        // In a more sophisticated implementation, we could:
        // 1. Have a database of popular quotes with popularity scores
        // 2. Use episode ratings as a proxy for popularity
        // 3. Use season number as a proxy (earlier seasons tend to have more iconic quotes)
    }
}
