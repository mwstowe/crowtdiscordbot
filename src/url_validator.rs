use lazy_static::lazy_static;
use regex::Regex;
use tracing::{error, info};

lazy_static! {
    static ref URL_REGEX: Regex = Regex::new(
        r"https?://(?:www\.)?[-a-zA-Z0-9@:%._\+~#=]{1,256}\.[a-zA-Z0-9()]{1,6}\b(?:[-a-zA-Z0-9()@:%_\+.~#?&//=]*)"
    ).unwrap();

    static ref SEARCH_ENGINE_REGEX: Regex = Regex::new(
        r"https?://(?:www\.)?(?:google\.com/search|bing\.com/search|search\.yahoo\.com|duckduckgo\.com/\?q=)"
    ).unwrap();

    static ref VALID_NEWS_DOMAINS: Regex = Regex::new(
        r"https?://(?:www\.)?(?:techcrunch\.com|arstechnica\.com|wired\.com|theverge\.com|bbc\.com|reuters\.com|cnn\.com|nytimes\.com|washingtonpost\.com|wsj\.com|bloomberg\.com|engadget\.com|gizmodo\.com|zdnet\.com|cnet\.com|venturebeat\.com|thenextweb\.com|mashable\.com|slashdot\.org|vice\.com|fastcompany\.com|forbes\.com|businessinsider\.com|theregister\.com)"
    ).unwrap();
}

/// Validates a URL for fact and news interjections
pub fn validate_url(text: &str) -> bool {
    // Extract URLs from the text
    if let Some(url_match) = URL_REGEX.find(text) {
        let url = url_match.as_str();

        // Check if it's a search engine URL
        if SEARCH_ENGINE_REGEX.is_match(url) {
            error!("Invalid URL: Search engine URL detected: {}", url);
            return false;
        }

        // For news interjections, check if it's from a valid news domain
        if text.contains("Article title:") && !VALID_NEWS_DOMAINS.is_match(url) {
            error!("Invalid news URL domain: {}", url);
            return false;
        }

        // Check if the URL contains a significant portion of the message text
        // This catches cases where the bot puts its entire response in the URL
        let url_without_prefix = url.replace("https://", "").replace("http://", "");
        let text_without_url = text.replace(url, "");

        if url_without_prefix.len() > 100 || url_without_prefix.contains(" ") {
            error!("Invalid URL: URL is too long or contains spaces: {}", url);
            return false;
        }

        // Check if the URL contains a significant portion of the remaining text
        let text_words: Vec<&str> = text_without_url.split_whitespace().collect();
        if text_words.len() >= 5 {
            let significant_words = text_words
                .iter()
                .filter(|word| word.len() > 4)
                .take(3)
                .collect::<Vec<_>>();

            for word in significant_words {
                if url_without_prefix.contains(word) {
                    error!("Invalid URL: URL contains message text: {}", url);
                    return false;
                }
            }
        }

        info!("URL validation passed: {}", url);
        return true;
    }

    error!("No URL found in text");
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_urls() {
        assert!(validate_url("Check out this fact: https://www.nasa.gov/feature/goddard/2016/carbon-dioxide-fertilization-greening-earth"));
        assert!(validate_url("Article title: New AI Breakthrough https://techcrunch.com/2025/07/new-ai-breakthrough-changes-everything/"));
    }

    #[test]
    fn test_search_engine_urls() {
        assert!(!validate_url(
            "Check out this fact: https://www.google.com/search?q=carbon+dioxide+fertilization"
        ));
        assert!(!validate_url(
            "Check out this fact: https://duckduckgo.com/?q=carbon+dioxide+fertilization"
        ));
        assert!(!validate_url(
            "Article title: New AI Breakthrough https://www.bing.com/search?q=new+ai+breakthrough"
        ));
    }

    #[test]
    fn test_invalid_news_domains() {
        assert!(!validate_url(
            "Article title: New AI Breakthrough https://example.com/2025/07/new-ai-breakthrough"
        ));
    }

    #[test]
    fn test_url_with_message_text() {
        assert!(!validate_url("Oh, I'm Crow. That's... mostly right, I guess? You forgot to mention I'm incredibly handsome. And modest. Very, very modest. https://www.google.com/search?hl=en&q=Oh%2C%20I%27m%20Crow.%20That%27s...%20mostly%20right%2C%20I%20guess%3F%20You%20forgot%20to%20mention%20I%27m%20incredibly%20handsome.%20And%20modest.%20Very%2C%20very%20modest."));
        assert!(!validate_url("Oh, I'm Crow. That's... mostly right, I guess? You forgot to mention I'm incredibly handsome. And modest. Very, very modest. https://duckduckgo.com/?q=Oh%2C%20I%27m%20Crow.%20That%27s...%20mostly%20right%2C%20I%20guess%3F%20You%20forgot%20to%20mention%20I%27m%20incredibly%20handsome.%20And%20modest.%20Very%2C%20very%20modest."));
    }
}
