use anyhow::Result;
use tracing::info;

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
}
