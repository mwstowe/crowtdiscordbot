use anyhow::Result;
use tracing::{error, info};
use crate::gemini_api::GeminiClient;

/// Verify that a news article title and summary match the content at the URL
pub async fn verify_news_article(
    gemini_client: &GeminiClient,
    article_title: &str,
    article_url: &str,
    article_summary: &str,
) -> Result<bool> {
    // Create a prompt for Gemini to verify the article
    let prompt = format!(
        "You are a fact-checking assistant. Your task is to determine if the provided article title and summary match the content at the URL.\n\n\
        Article Title: {}\n\
        Article URL: {}\n\
        Article Summary: {}\n\n\
        Based on the URL and your knowledge, determine if the title and summary are likely to match the actual content at the URL.\n\
        Consider the following:\n\
        1. Does the URL domain match a reputable news source?\n\
        2. Does the URL path contain keywords related to the title or summary?\n\
        3. Does the title seem appropriate for the news source?\n\
        4. Does the summary contain information that would likely be in an article with this title?\n\
        5. Are there any obvious mismatches or inconsistencies?\n\n\
        Respond with ONLY ONE of these exact words:\n\
        - \"MATCH\" - if the title and summary likely match the URL\n\
        - \"MISMATCH\" - if there's a clear mismatch between the title/summary and the URL\n\
        - \"UNCERTAIN\" - if you cannot determine with confidence\n\
        Do not include any other text in your response.",
        article_title,
        article_url,
        article_summary
    );

    // Send the prompt to Gemini
    match gemini_client.generate_content(&prompt).await {
        Ok(response) => {
            let response = response.trim().to_uppercase();
            info!("News verification response: {}", response);
            
            match response.as_str() {
                "MATCH" => Ok(true),
                "MISMATCH" => {
                    error!("News verification failed: Title/summary mismatch with URL");
                    Ok(false)
                },
                "UNCERTAIN" => {
                    error!("News verification uncertain: Cannot determine if title/summary match URL");
                    Ok(false) // Treat uncertainty as a failure to be safe
                },
                _ => {
                    error!("Unexpected news verification response: {}", response);
                    Ok(false) // Treat unexpected responses as failures
                }
            }
        },
        Err(e) => {
            error!("Error verifying news article: {:?}", e);
            Ok(false) // Treat errors as failures
        }
    }
}

/// Verify that a URL is properly formatted for a news article
pub fn verify_url_format(url: &str) -> bool {
    // Check if the URL is from a known news source
    let known_domains = [
        "arstechnica.com", "techcrunch.com", "wired.com", "theverge.com", 
        "bbc.com", "reuters.com", "nytimes.com", "washingtonpost.com",
        "cnn.com", "apnews.com", "npr.org", "smithsonianmag.com",
        "scientificamerican.com", "nature.com", "science.org", "newscientist.com",
        "technologyreview.com", "engadget.com", "gizmodo.com", "zdnet.com"
    ];
    
    // Check if the URL contains a known domain
    let contains_known_domain = known_domains.iter().any(|domain| url.contains(domain));
    if !contains_known_domain {
        error!("URL verification failed: Unknown domain in URL: {}", url);
        return false;
    }
    
    // Check if the URL is properly formatted
    // Should be something like: https://domain.com/section/YYYY/MM/specific-article-title-with-multiple-words/
    // or https://domain.com/section/specific-article-title-with-multiple-words/
    
    // Check if the URL has at least 4 path segments (domain/section/something/something)
    let segments: Vec<&str> = url.split('/').collect();
    if segments.len() < 6 {
        error!("URL verification failed: Not enough path segments in URL: {}", url);
        return false;
    }
    
    // Check if the URL ends with a specific article title (at least 3 words)
    let last_segment = segments.last().unwrap_or(&"");
    let words_in_last_segment = last_segment.split('-').count();
    if words_in_last_segment < 3 && !last_segment.is_empty() {
        error!("URL verification failed: Last segment doesn't contain enough words: {}", last_segment);
        return false;
    }
    
    // Check if the URL contains a year and month (YYYY/MM)
    let has_year_month = segments.iter().enumerate().any(|(i, segment)| {
        if i > 0 && i < segments.len() - 1 {
            // Check if this segment is a 4-digit year
            if segment.len() == 4 && segment.chars().all(|c| c.is_digit(10)) {
                // Check if the next segment is a 2-digit month
                let next = segments.get(i + 1).unwrap_or(&"");
                return next.len() == 2 && next.chars().all(|c| c.is_digit(10));
            }
        }
        false
    });
    
    // If the URL doesn't have a year/month pattern, it should at least have a substantial path
    if !has_year_month && segments.len() < 5 {
        error!("URL verification failed: URL doesn't contain year/month and doesn't have enough path segments: {}", url);
        return false;
    }
    
    // Check if the URL is a generic category or date-only URL
    let last_non_empty_segment = segments.iter().rev().find(|s| !s.is_empty()).unwrap_or(&"");
    if last_non_empty_segment.len() <= 2 && last_non_empty_segment.chars().all(|c| c.is_digit(10)) {
        error!("URL verification failed: URL ends with a date segment: {}", url);
        return false;
    }
    
    true
}

/// Extract the article title and URL from a formatted news interjection
pub fn extract_article_info(text: &str) -> Option<(String, String)> {
    // Look for the pattern "Article title: https://..."
    if let Some(colon_pos) = text.find(": http") {
        let title = text[0..colon_pos].trim().to_string();
        
        // Find the end of the URL (space, newline, or end of string)
        let url_start = colon_pos + 2; // Skip the ": "
        let url_end = text[url_start..].find(|c: char| c.is_whitespace())
            .map_or(text.len(), |pos| url_start + pos);
        
        let url = text[url_start..url_end].trim().to_string();
        
        // Extract the summary (everything after the URL)
        let _summary = if url_end < text.len() {
            text[url_end..].trim().to_string()
        } else {
            String::new()
        };
        
        Some((title, url))
    } else {
        None
    }
}
