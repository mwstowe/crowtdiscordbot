use anyhow::Result;
use reqwest;
use scraper::{Html, Selector};
use tracing::{error, info};

pub struct GoogleSearchClient {}

pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

impl GoogleSearchClient {
    pub fn new() -> Self {
        Self {}
    }

    pub async fn search(&self, query: &str) -> Result<Option<SearchResult>> {
        info!("Performing Google search for: {}", query);
        
        // Create the client with custom user agent to avoid being blocked
        let client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36")
            .build()?;
        
        // Build the URL with query parameters
        let url = format!(
            "https://www.google.com/search?q={}",
            urlencoding::encode(query)
        );
        
        // Make the request
        let response = client.get(&url)
            .send()
            .await?;
        
        // Check if the request was successful
        if !response.status().is_success() {
            let error_text = response.text().await?;
            error!("Google search request failed: {}", error_text);
            return Err(anyhow::anyhow!("Google search request failed: {}", error_text));
        }
        
        // Get the HTML content
        let html_content = response.text().await?;
        
        // Parse the HTML
        let document = Html::parse_document(&html_content);
        
        // Try to find search results
        // These selectors might need adjustment based on Google's current HTML structure
        let search_result_selector = Selector::parse("div.g").unwrap_or_else(|_| Selector::parse("div.tF2Cxc").unwrap());
        let title_selector = Selector::parse("h3").unwrap();
        let link_selector = Selector::parse("a").unwrap();
        let snippet_selector = Selector::parse("div.VwiC3b").unwrap_or_else(|_| Selector::parse("span.aCOpRe").unwrap());
        
        // Find the first search result
        if let Some(result) = document.select(&search_result_selector).next() {
            // Extract title
            let title = result.select(&title_selector)
                .next()
                .map(|el| el.text().collect::<Vec<_>>().join(""))
                .unwrap_or_else(|| "No title".to_string());
            
            // Extract URL
            let url = result.select(&link_selector)
                .next()
                .and_then(|el| el.value().attr("href"))
                .map(|href| {
                    if href.starts_with("/url?q=") {
                        // Extract the actual URL from Google's redirect URL
                        if let Some(end_idx) = href.find("&sa=") {
                            return href[7..end_idx].to_string();
                        }
                    }
                    href.to_string()
                })
                .unwrap_or_else(|| "No URL".to_string());
            
            // Extract snippet
            let snippet = result.select(&snippet_selector)
                .next()
                .map(|el| el.text().collect::<Vec<_>>().join(""))
                .unwrap_or_else(|| "No description available".to_string());
            
            return Ok(Some(SearchResult {
                title,
                url,
                snippet,
            }));
        }
        
        // No results found
        info!("No search results found for query: {}", query);
        Ok(None)
    }
}
