use anyhow::Result;
use reqwest;
use scraper::{Html, Selector};
use tracing::{error, info};
use std::time::Duration;

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

    // Helper method to fetch raw HTML for debugging
    pub async fn fetch_raw_html(&self, query: &str) -> Result<String> {
        // Create the client with custom user agent to avoid being blocked
        let client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36")
            .timeout(Duration::from_secs(10))
            .build()?;
        
        // Build the URL with query parameters
        let url = format!(
            "https://duckduckgo.com/html/?q={}",
            urlencoding::encode(query)
        );
        
        info!("Fetching search results from: {}", url);
        
        // Make the request
        let response = client.get(&url)
            .send()
            .await?;
        
        // Check if the request was successful
        if !response.status().is_success() {
            let error_text = response.text().await?;
            error!("Search request failed: {}", error_text);
            return Err(anyhow::anyhow!("Search request failed: {}", error_text));
        }
        
        // Get the HTML content
        let html_content = response.text().await?;
        Ok(html_content)
    }

    pub async fn search(&self, query: &str) -> Result<Option<SearchResult>> {
        info!("Performing search for: {}", query);
        
        // Get the HTML content from DuckDuckGo
        let html_content = self.fetch_raw_html(query).await?;
        
        // Parse the HTML
        let document = Html::parse_document(&html_content);
        
        // DuckDuckGo search results are in elements with class "result"
        if let Ok(result_selector) = Selector::parse(".result") {
            for result in document.select(&result_selector) {
                // Extract title
                if let Ok(title_selector) = Selector::parse(".result__title") {
                    if let Some(title_element) = result.select(&title_selector).next() {
                        let title = title_element.text().collect::<Vec<_>>().join("");
                        
                        // Extract URL
                        if let Ok(link_selector) = Selector::parse(".result__title a") {
                            if let Some(link_element) = result.select(&link_selector).next() {
                                if let Some(href) = link_element.value().attr("href") {
                                    // Extract snippet
                                    let snippet = if let Ok(snippet_selector) = Selector::parse(".result__snippet") {
                                        if let Some(snippet_element) = result.select(&snippet_selector).next() {
                                            snippet_element.text().collect::<Vec<_>>().join("")
                                        } else {
                                            "No description available".to_string()
                                        }
                                    } else {
                                        "No description available".to_string()
                                    };
                                    
                                    // Extract the actual URL from DuckDuckGo's redirect URL
                                    let actual_url = if href.contains("//duckduckgo.com/l/?uddg=") {
                                        if let Some(encoded_url) = href.split("uddg=").nth(1) {
                                            if let Some(end_idx) = encoded_url.find("&") {
                                                let encoded_part = &encoded_url[..end_idx];
                                                match urlencoding::decode(encoded_part) {
                                                    Ok(decoded) => decoded.to_string(),
                                                    Err(_) => href.to_string()
                                                }
                                            } else {
                                                href.to_string()
                                            }
                                        } else {
                                            href.to_string()
                                        }
                                    } else {
                                        href.to_string()
                                    };
                                    
                                    return Ok(Some(SearchResult {
                                        title,
                                        url: actual_url,
                                        snippet,
                                    }));
                                }
                            }
                        }
                    }
                }
            }
        }
        
        // No results found
        info!("No search results found for query: {}", query);
        Ok(None)
    }
}
