use anyhow::Result;
use tracing::{error, info};

pub struct GiphyClient {
    api_key: String,
}

impl GiphyClient {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }

    /// Search Giphy for a GIF and return the URL of the top result
    pub async fn search_gif(&self, query: &str) -> Result<Option<String>> {
        info!("Searching Giphy for GIF: {}", query);

        let url = format!(
            "https://api.giphy.com/v1/gifs/search?api_key={}&q={}&limit=1&rating=pg-13",
            self.api_key,
            urlencoding::encode(query),
        );

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?;

        let json: serde_json::Value = response.json().await?;

        // Extract the GIF URL - use the regular size for Discord embedding
        if let Some(gif_url) = json
            .pointer("/data/0/images/original/url")
            .and_then(|u| u.as_str())
        {
            info!("Found GIF: {}", gif_url);
            Ok(Some(gif_url.to_string()))
        } else {
            info!("No GIF results found for: {}", query);
            Ok(None)
        }
    }

    /// Check if a response is a GIF request (starts with "GIF:") and resolve it.
    /// Returns Some(gif_url) if it was a GIF request and we found one, None otherwise.
    pub async fn try_resolve_gif(&self, response: &str) -> Option<String> {
        let trimmed = response.trim();
        if !trimmed.starts_with("GIF:") {
            return None;
        }
        let search_term = trimmed[4..].trim();
        if search_term.is_empty() {
            return None;
        }
        match self.search_gif(search_term).await {
            Ok(Some(url)) => {
                info!("GIF resolved: {} -> {}", search_term, url);
                Some(url)
            }
            Ok(None) => {
                info!("No GIF found for: {}", search_term);
                None
            }
            Err(e) => {
                error!("Error searching for GIF: {:?}", e);
                None
            }
        }
    }

    /// Check if a response contains "GIF:" anywhere (even after text).
    /// Returns Some((text_before, gif_url)) if found and resolved, None otherwise.
    pub async fn try_resolve_embedded_gif(&self, response: &str) -> Option<(String, String)> {
        let trimmed = response.trim();
        if let Some(gif_pos) = trimmed.find("GIF:") {
            let text_before = trimmed[..gif_pos].trim().to_string();
            let search_term = trimmed[gif_pos + 4..].trim();
            if search_term.is_empty() {
                return None;
            }
            match self.search_gif(search_term).await {
                Ok(Some(url)) => {
                    info!("Embedded GIF resolved: {} -> {}", search_term, url);
                    Some((text_before, url))
                }
                Ok(None) => None,
                Err(e) => {
                    error!("Error searching for embedded GIF: {:?}", e);
                    None
                }
            }
        } else {
            None
        }
    }
}

/// The GIF instruction text to append to prompts
pub const GIF_INSTRUCTION: &str = r#"

GIF OPTION: If a reaction GIF would be funnier or more expressive than words, you may respond with ONLY "GIF: [search term]" where [search term] is a short phrase describing the perfect reaction GIF. Only use this when a GIF would genuinely be the best response. Example: "GIF: mind blown explosion""#;
