use anyhow::Result;
use scraper::{Html, Selector};
use std::time::Duration;
use tracing::{error, info};

pub struct DuckDuckGoSearchClient {}

pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

impl DuckDuckGoSearchClient {
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
        let response = client.get(&url).send().await?;

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

        // Debug: Save the HTML content to a file for inspection
        if query.contains("site:frinkiac.com") {
            info!("Saving DuckDuckGo HTML response for debugging");
            std::fs::write("/tmp/duckduckgo_response.html", &html_content)
                .map_err(|e| anyhow::anyhow!("Failed to save debug HTML: {}", e))?;
        }

        // Parse the HTML
        let document = Html::parse_document(&html_content);

        // Try different selectors for DuckDuckGo results
        let result_selectors = [
            ".result",                // Standard result class
            ".web-result",            // Alternative result class
            ".results_links",         // Another possible class
            ".results__body .result", // Nested results
            "article",                // Generic article element
        ];

        for selector_str in &result_selectors {
            if let Ok(result_selector) = Selector::parse(selector_str) {
                for result in document.select(&result_selector) {
                    // Skip sponsored results
                    let has_sponsored_class = result
                        .value()
                        .has_class("result--ad", scraper::CaseSensitivity::CaseSensitive)
                        || result
                            .value()
                            .has_class("sponsored", scraper::CaseSensitivity::CaseSensitive);
                    if has_sponsored_class {
                        info!("Skipping sponsored result");
                        continue;
                    }

                    // Try different title selectors
                    let title_selectors = [".result__title", ".result__a", "h2", "h3", "a"];
                    let mut title = String::new();

                    for title_sel in &title_selectors {
                        if let Ok(title_selector) = Selector::parse(title_sel) {
                            if let Some(title_element) = result.select(&title_selector).next() {
                                title = title_element.text().collect::<Vec<_>>().join("");
                                if !title.is_empty() {
                                    break;
                                }
                            }
                        }
                    }

                    if title.is_empty() {
                        continue; // Skip if we couldn't find a title
                    }

                    // Try different URL selectors
                    let link_selectors = [".result__title a", "a", ".result__a"];
                    let mut href = String::new();

                    for link_sel in &link_selectors {
                        if let Ok(link_selector) = Selector::parse(link_sel) {
                            if let Some(link_element) = result.select(&link_selector).next() {
                                if let Some(link_href) = link_element.value().attr("href") {
                                    href = link_href.to_string();
                                    if !href.is_empty() {
                                        break;
                                    }
                                }
                            }
                        }
                    }

                    if href.is_empty() {
                        continue; // Skip if we couldn't find a URL
                    }

                    // Try different snippet selectors
                    let snippet_selectors =
                        [".result__snippet", ".result__snippet-link", ".snippet", "p"];
                    let mut snippet = String::new();

                    for snippet_sel in &snippet_selectors {
                        if let Ok(snippet_selector) = Selector::parse(snippet_sel) {
                            if let Some(snippet_element) = result.select(&snippet_selector).next() {
                                snippet = snippet_element.text().collect::<Vec<_>>().join("");
                                if !snippet.is_empty() {
                                    break;
                                }
                            }
                        }
                    }

                    if snippet.is_empty() {
                        snippet = "No description available".to_string();
                    }

                    // Extract the actual URL from DuckDuckGo's redirect URL
                    let actual_url = if href.contains("//duckduckgo.com/l/?uddg=") {
                        if let Some(encoded_url) = href.split("uddg=").nth(1) {
                            if let Some(end_idx) = encoded_url.find("&") {
                                let encoded_part = &encoded_url[..end_idx];
                                match urlencoding::decode(encoded_part) {
                                    Ok(decoded) => decoded.to_string(),
                                    Err(_) => href.to_string(),
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

                    // Special handling for frinkiac.com results
                    if query.contains("site:frinkiac.com")
                        && (title.contains("Frinkiac") || actual_url.contains("frinkiac.com"))
                    {
                        info!("Found Frinkiac result: {} - {}", title, snippet);

                        // Extract episode information if available
                        let mut enhanced_snippet = snippet.clone();

                        // Look for season/episode information in the URL or snippet
                        if actual_url.contains("S") && actual_url.contains("E") {
                            if let Some(season_ep) = extract_season_episode(&actual_url) {
                                enhanced_snippet = format!("{} [{}]", enhanced_snippet, season_ep);
                            }
                        }

                        return Ok(Some(SearchResult {
                            title,
                            url: actual_url,
                            snippet: enhanced_snippet,
                        }));
                    }

                    return Ok(Some(SearchResult {
                        title,
                        url: actual_url,
                        snippet,
                    }));
                }
            }
        }

        // No results found
        info!("No search results found for query: {}", query);
        Ok(None)
    }
}

// Helper function to extract season and episode information from a URL
fn extract_season_episode(url: &str) -> Option<String> {
    // Look for patterns like S07E05
    let re = regex::Regex::new(r"S(\d+)E(\d+)").ok()?;
    if let Some(caps) = re.captures(url) {
        let season = caps.get(1)?.as_str();
        let episode = caps.get(2)?.as_str();
        return Some(format!("Season {} Episode {}", season, episode));
    }
    None
}
