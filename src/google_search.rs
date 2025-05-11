use anyhow::Result;
use reqwest;
use serde_json;
use tracing::{error, info};

pub struct GoogleSearchClient {
    api_key: String,
    search_engine_id: String,
}

pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

impl GoogleSearchClient {
    pub fn new(api_key: String, search_engine_id: String) -> Self {
        Self {
            api_key,
            search_engine_id,
        }
    }

    pub async fn search(&self, query: &str) -> Result<Option<SearchResult>> {
        info!("Performing Google search for: {}", query);
        
        // Create the client
        let client = reqwest::Client::new();
        
        // Build the URL with query parameters
        let url = format!(
            "https://www.googleapis.com/customsearch/v1?key={}&cx={}&q={}",
            self.api_key,
            self.search_engine_id,
            urlencoding::encode(query)
        );
        
        // Make the request
        let response = client.get(&url)
            .send()
            .await?;
        
        // Check if the request was successful
        if !response.status().is_success() {
            let error_text = response.text().await?;
            error!("Google search API request failed: {}", error_text);
            return Err(anyhow::anyhow!("Google search API request failed: {}", error_text));
        }
        
        // Parse the response
        let response_json: serde_json::Value = response.json().await?;
        
        // Check if we have search results
        if let Some(items) = response_json["items"].as_array() {
            if let Some(first_result) = items.first() {
                // Extract the title, URL, and snippet
                let title = first_result["title"].as_str()
                    .unwrap_or("No title")
                    .to_string();
                    
                let url = first_result["link"].as_str()
                    .unwrap_or("No URL")
                    .to_string();
                    
                let snippet = first_result["snippet"].as_str()
                    .unwrap_or("No description available")
                    .to_string();
                
                return Ok(Some(SearchResult {
                    title,
                    url,
                    snippet,
                }));
            }
        }
        
        // No results found
        info!("No search results found for query: {}", query);
        Ok(None)
    }
}
