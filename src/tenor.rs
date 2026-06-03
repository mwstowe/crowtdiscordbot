use anyhow::Result;
use tracing::info;

pub struct TenorClient {
    api_key: String,
}

impl TenorClient {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }

    /// Search Tenor for a GIF and return the URL of the top result
    pub async fn search_gif(&self, query: &str) -> Result<Option<String>> {
        info!("Searching Tenor for GIF: {}", query);

        let url = format!(
            "https://tenor.googleapis.com/v2/search?q={}&key={}&limit=1&media_filter=gif",
            urlencoding::encode(query),
            self.api_key
        );

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?;

        let json: serde_json::Value = response.json().await?;

        if let Some(gif_url) = json
            .pointer("/results/0/media_formats/gif/url")
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
