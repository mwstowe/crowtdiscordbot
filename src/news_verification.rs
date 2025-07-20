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
    
    // Parse URL segments
    let segments: Vec<&str> = url.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() < 4 {
        error!("URL verification failed: Not enough path segments in URL: {}", url);
        return false;
    }
    
    // Skip protocol and domain (https:, domain.com)
    let path_segments = &segments[2..];
    
    // Check for archive/category page patterns that should be excluded
    let is_archive_page = is_archive_or_category_url(path_segments);
    if is_archive_page {
        error!("URL verification failed: URL appears to be an archive/category page: {}", url);
        return false;
    }
    
    // Check if the URL ends with a specific article title (at least 3 words)
    let last_segment = path_segments.last().unwrap_or(&"");
    if !last_segment.is_empty() {
        let words_in_last_segment = last_segment.split('-').filter(|s| !s.is_empty()).count();
        if words_in_last_segment < 3 {
            error!("URL verification failed: Last segment doesn't contain enough words for an article title: {}", last_segment);
            return false;
        }
        
        // Check if the last segment is just a date (like "2024-04-15")
        if is_date_segment(last_segment) {
            error!("URL verification failed: Last segment appears to be a date: {}", last_segment);
            return false;
        }
    }
    
    // Ensure the URL has enough depth to be a specific article
    if path_segments.len() < 2 {
        error!("URL verification failed: URL path too shallow for a specific article: {}", url);
        return false;
    }
    
    true
}

/// Check if the URL path segments indicate an archive or category page
fn is_archive_or_category_url(path_segments: &[&str]) -> bool {
    // Pattern 1: Ends with just year/month (e.g., /ai/2024/04/ or /ai/2024/04)
    if path_segments.len() >= 3 {
        let last_three = &path_segments[path_segments.len()-3..];
        if last_three.len() == 3 {
            // Check if it's category/year/month pattern
            if is_year(last_three[1]) && is_month(last_three[2]) {
                info!("Detected archive pattern: category/year/month");
                return true;
            }
        }
    }
    
    // Pattern 2: Ends with just year (e.g., /news/2024/ or /news/2024)
    if path_segments.len() >= 2 {
        let last_two = &path_segments[path_segments.len()-2..];
        if last_two.len() == 2 && is_year(last_two[1]) {
            info!("Detected archive pattern: category/year");
            return true;
        }
    }
    
    // Pattern 3: Ends with just a category (e.g., /ai/ or /technology/)
    if path_segments.len() == 1 {
        let category_indicators = [
            "ai", "tech", "technology", "science", "news", "politics", "business",
            "sports", "entertainment", "health", "world", "opinion", "lifestyle",
            "culture", "gaming", "security", "privacy", "mobile", "software",
            "hardware", "internet", "social", "media", "startups", "gadgets"
        ];
        
        let last_segment = path_segments[0].to_lowercase();
        if category_indicators.contains(&last_segment.as_str()) {
            info!("Detected category page: {}", last_segment);
            return true;
        }
    }
    
    // Pattern 4: Contains common archive indicators
    let archive_indicators = ["archive", "category", "tag", "page"];
    for segment in path_segments {
        if archive_indicators.contains(&segment.to_lowercase().as_str()) {
            info!("Detected archive indicator in path: {}", segment);
            return true;
        }
    }
    
    false
}

/// Check if a segment represents a 4-digit year
fn is_year(segment: &str) -> bool {
    segment.len() == 4 && segment.chars().all(|c| c.is_ascii_digit()) && 
    segment.parse::<i32>().map_or(false, |year| year >= 1990 && year <= 2030)
}

/// Check if a segment represents a 2-digit month
fn is_month(segment: &str) -> bool {
    segment.len() == 2 && segment.chars().all(|c| c.is_ascii_digit()) &&
    segment.parse::<i32>().map_or(false, |month| month >= 1 && month <= 12)
}

/// Check if a segment looks like a date (YYYY-MM-DD format)
fn is_date_segment(segment: &str) -> bool {
    let parts: Vec<&str> = segment.split('-').collect();
    if parts.len() == 3 {
        return is_year(parts[0]) && is_month(parts[1]) && 
               parts[2].len() == 2 && parts[2].chars().all(|c| c.is_ascii_digit()) &&
               parts[2].parse::<i32>().map_or(false, |day| day >= 1 && day <= 31);
    }
    false
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
