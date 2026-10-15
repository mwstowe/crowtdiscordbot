use anyhow::Result;
use tracing::{error, info};

pub struct GiphyClient {
    api_key: String,
    http_client: reqwest::Client,
}

impl GiphyClient {
    pub fn new(api_key: String) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to create Giphy HTTP client");
        Self {
            api_key,
            http_client,
        }
    }

    /// Search Giphy for a GIF and return the URL of the top result.
    /// Tries the full query first, then progressively simpler variants, then a
    /// generic reaction term, and finally a truly random GIF — so a GIF
    /// directive essentially always resolves to a usable URL (a random GIF is
    /// better than posting the raw "GIF:" text).
    pub async fn search_gif(&self, query: &str) -> Result<Option<String>> {
        for candidate in Self::search_candidates(query) {
            info!("Searching Giphy for GIF: {}", candidate);
            match self.search_once(&candidate).await {
                Ok(Some(url)) => {
                    info!("Found GIF: {}", url);
                    return Ok(Some(url));
                }
                Ok(None) => {
                    info!("No GIF results for '{}', trying a simpler term", candidate);
                    continue;
                }
                Err(e) => {
                    // Network/API error: log and try the next candidate rather than
                    // giving up (which would cause the raw "GIF:" text to be posted).
                    error!("Giphy search error for '{}': {:?}", candidate, e);
                    continue;
                }
            }
        }

        // Last resort: a random GIF is better than nothing (and far better than
        // posting the literal "GIF: ..." text to the channel).
        info!("No GIF matched '{}'; falling back to a random GIF", query);
        match self.random_gif().await {
            Ok(Some(url)) => {
                info!("Random fallback GIF: {}", url);
                Ok(Some(url))
            }
            Ok(None) => {
                info!("Random GIF fallback also returned nothing for '{}'", query);
                Ok(None)
            }
            Err(e) => {
                error!("Random GIF fallback error: {:?}", e);
                Ok(None)
            }
        }
    }

    /// Fetch a random GIF from Giphy (used as the ultimate fallback).
    async fn random_gif(&self) -> Result<Option<String>> {
        let url = format!(
            "https://api.giphy.com/v1/gifs/random?api_key={}&rating=pg-13",
            self.api_key,
        );
        let response = self.http_client.get(&url).send().await?;
        let json: serde_json::Value = response.json().await?;

        // The /random endpoint nests images under "data" (not an array).
        let gif_url = json
            .pointer("/data/images/downsized/url")
            .and_then(|u| u.as_str())
            .or_else(|| {
                json.pointer("/data/images/original/url")
                    .and_then(|u| u.as_str())
            })
            .or_else(|| json.pointer("/data/image_url").and_then(|u| u.as_str()));

        Ok(gif_url.map(|s| s.to_string()))
    }

    /// Build an ordered list of search terms to try: the full term, a trimmed
    /// first-few-keywords version, and finally a generic reaction fallback.
    fn search_candidates(query: &str) -> Vec<String> {
        let mut candidates: Vec<String> = Vec::new();
        let cleaned = query.trim();
        if !cleaned.is_empty() {
            candidates.push(cleaned.to_string());
        }

        // Simplified: keep the first 3 significant words (Giphy matches short
        // common phrases far better than long specific ones).
        let words: Vec<&str> = cleaned.split_whitespace().collect();
        if words.len() > 3 {
            let short = words[..3].join(" ");
            if !candidates.contains(&short) {
                candidates.push(short);
            }
        }

        // First keyword alone, if we have one and it isn't already present.
        if let Some(first) = words.first() {
            let first = first.to_string();
            if first.len() >= 3 && !candidates.contains(&first) {
                candidates.push(first);
            }
        }

        // Generic reaction fallback so a GIF directive still yields *a* GIF.
        candidates.push("shrug".to_string());

        candidates
    }

    /// Perform a single Giphy search request for an exact query.
    async fn search_once(&self, query: &str) -> Result<Option<String>> {
        let url = format!(
            "https://api.giphy.com/v1/gifs/search?api_key={}&q={}&limit=1&rating=pg-13",
            self.api_key,
            urlencoding::encode(query),
        );

        let response = self.http_client.get(&url).send().await?;
        let json: serde_json::Value = response.json().await?;

        // Prefer the downsized URL (smaller, embeds reliably), fall back to original
        let gif_url = json
            .pointer("/data/0/images/downsized/url")
            .and_then(|u| u.as_str())
            .or_else(|| {
                json.pointer("/data/0/images/original/url")
                    .and_then(|u| u.as_str())
            });

        Ok(gif_url.map(|s| s.to_string()))
    }

    /// Check if a response is a GIF request (starts with "GIF:") and resolve it.
    /// Returns Some(gif_url) if it was a GIF request and we found one, None otherwise.
    pub async fn try_resolve_gif(&self, response: &str) -> Option<String> {
        let trimmed = response.trim();
        if !trimmed.starts_with("GIF:") {
            return None;
        }
        // Strip any number of leading "GIF:" prefixes (the model/multi-response
        // path occasionally double-wraps, e.g. "GIF: GIF: paper cutter").
        let search_term = strip_gif_prefixes(trimmed);
        if search_term.is_empty() {
            return None;
        }
        match self.search_gif(&search_term).await {
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
            // Strip repeated "GIF:" prefixes from the remainder.
            let search_term = strip_gif_prefixes(&trimmed[gif_pos..]);
            if search_term.is_empty() {
                return None;
            }
            match self.search_gif(&search_term).await {
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

/// Strip one or more leading "GIF:" prefixes (and surrounding whitespace) from a
/// string, returning the underlying search term. Handles the occasional
/// double-wrap like "GIF: GIF: paper cutter accident".
pub fn strip_gif_prefixes(s: &str) -> String {
    let mut cur = s.trim();
    while let Some(rest) = cur.strip_prefix("GIF:") {
        cur = rest.trim();
    }
    cur.to_string()
}

/// Returns true if the response is purely a GIF directive (starts with "GIF:").
pub fn is_gif_directive(response: &str) -> bool {
    response.trim().starts_with("GIF:")
}

/// The GIF instruction text to append to prompts
pub const GIF_INSTRUCTION: &str = r#"

GIF OPTION: If a reaction GIF would be funnier or more expressive than words, you may respond with ONLY "GIF: [search term]" where [search term] is a short phrase describing the perfect reaction GIF. Only use this when a GIF would genuinely be the best response. Example: "GIF: mind blown explosion""#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_single_and_double_gif_prefixes() {
        assert_eq!(
            strip_gif_prefixes("GIF: paper cutter accident"),
            "paper cutter accident"
        );
        // The occasional double-wrap must collapse to the real search term
        assert_eq!(
            strip_gif_prefixes("GIF: GIF: glowing angel chorus"),
            "glowing angel chorus"
        );
        assert_eq!(strip_gif_prefixes("  GIF:   shrug  "), "shrug");
    }

    #[test]
    fn detects_gif_directive() {
        assert!(is_gif_directive("GIF: mind blown"));
        assert!(is_gif_directive("  GIF: tumbleweed"));
        assert!(!is_gif_directive("That was wild. GIF: mind blown"));
        assert!(!is_gif_directive("just a normal sentence"));
    }

    #[test]
    fn search_candidates_simplify_and_fallback() {
        let c = GiphyClient::search_candidates("a very obscure paper cutter accident scene");
        // Full term first
        assert_eq!(c[0], "a very obscure paper cutter accident scene");
        // A 3-word simplified variant is included
        assert!(c.iter().any(|s| s == "a very obscure"));
        // Always ends with the generic reaction fallback
        assert_eq!(c.last().unwrap(), "shrug");
    }

    #[test]
    fn search_candidates_short_query() {
        let c = GiphyClient::search_candidates("shrug");
        assert_eq!(c[0], "shrug");
        assert_eq!(c.last().unwrap(), "shrug");
    }
}
