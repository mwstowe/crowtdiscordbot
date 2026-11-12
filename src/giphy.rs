use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use tracing::{error, info};

/// Results requested per search. We pick the best-matching one rather than
/// trusting Giphy's top hit, which is often unrelated for specific queries.
const RESULTS_PER_SEARCH: usize = 5;

/// Resolved queries remembered (including "no match"), so resolving the same
/// directive more than once doesn't repeat network requests.
const MEMO_CAPACITY: usize = 32;

/// Filler words ignored when judging whether a GIF matches a query.
const STOPWORDS: &[&str] = &[
    "a", "an", "the", "in", "on", "at", "of", "to", "and", "or", "for", "with", "by", "from", "is",
    "are", "it", "its", "this", "that", "his", "her", "their",
];

/// A search result with the text we can use to judge its relevance.
#[derive(Debug, Clone, PartialEq)]
struct GifCandidate {
    url: String,
    title: String,
    slug: String,
}

pub struct GiphyClient {
    api_key: String,
    http_client: reqwest::Client,
    /// query (lowercased) -> resolved URL, or None for "no relevant GIF"
    memo: Mutex<HashMap<String, Option<String>>>,
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
            memo: Mutex::new(HashMap::new()),
        }
    }

    /// Search Giphy for a GIF that actually matches the query.
    ///
    /// Each search term is tried in turn (the full query, then a keywords-only
    /// form). From each batch of results, a GIF is accepted only if its title
    /// matches enough of the *original* query's keywords. Returns `Ok(None)`
    /// when nothing relevant is found: a missing GIF is better than a random
    /// one, and callers can fall back to a text response.
    pub async fn search_gif(&self, query: &str) -> Result<Option<String>> {
        let memo_key = query.trim().to_lowercase();
        if let Some(cached) = self.memo_get(&memo_key) {
            return Ok(cached);
        }

        let wanted = keywords(query);
        let mut had_error = false;

        for term in Self::search_terms(query) {
            info!("Searching Giphy for GIF: {}", term);
            match self.fetch_candidates(&term).await {
                Ok(candidates) => {
                    if let Some((candidate, matched)) = best_candidate(&wanted, candidates) {
                        info!(
                            "Found GIF for '{}' (matched {}/{} keywords, title: {:?}): {}",
                            term,
                            matched,
                            wanted.len(),
                            candidate.title,
                            candidate.url
                        );
                        self.memo_put(memo_key, Some(candidate.url.clone()));
                        return Ok(Some(candidate.url));
                    }
                    info!("No sufficiently relevant GIF among results for '{}'", term);
                }
                Err(e) => {
                    // Network/API error: log and try the next term.
                    had_error = true;
                    error!("Giphy search error for '{}': {:?}", term, e);
                }
            }
        }

        info!("No relevant GIF found for '{}'", query);
        // Don't remember a miss that might just have been a transient failure.
        if !had_error {
            self.memo_put(memo_key, None);
        }
        Ok(None)
    }

    fn memo_get(&self, key: &str) -> Option<Option<String>> {
        let memo = self.memo.lock().unwrap_or_else(|e| e.into_inner());
        memo.get(key).cloned()
    }

    fn memo_put(&self, key: String, value: Option<String>) {
        let mut memo = self.memo.lock().unwrap_or_else(|e| e.into_inner());
        if memo.len() >= MEMO_CAPACITY {
            memo.clear();
        }
        memo.insert(key, value);
    }

    /// Search terms to try, most specific first: the full query, then its
    /// keywords alone (filler words removed), then just the leading keywords
    /// for long queries. There is deliberately no generic or random fallback.
    fn search_terms(query: &str) -> Vec<String> {
        let cleaned = query.trim();
        let mut terms: Vec<String> = Vec::new();
        if !cleaned.is_empty() {
            terms.push(cleaned.to_string());
        }

        let kw = keywords(cleaned);
        let mut push_unique = |term: String| {
            if !terms.iter().any(|t| t.eq_ignore_ascii_case(&term)) {
                terms.push(term);
            }
        };
        if kw.len() >= 2 {
            push_unique(kw.join(" "));
        }
        if kw.len() > 3 {
            push_unique(kw[..3].join(" "));
        }
        terms
    }

    /// Fetch a batch of results for an exact query.
    async fn fetch_candidates(&self, query: &str) -> Result<Vec<GifCandidate>> {
        let url = format!(
            "https://api.giphy.com/v1/gifs/search?api_key={}&q={}&limit={}&rating=pg-13",
            self.api_key,
            urlencoding::encode(query),
            RESULTS_PER_SEARCH,
        );

        let response = self.http_client.get(&url).send().await?;
        let json: serde_json::Value = response.json().await?;
        Ok(parse_candidates(&json))
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

    /// Return `selected` if it can be sent as-is: either it isn't a GIF
    /// request, or its GIF request resolves to a relevant GIF. Otherwise return
    /// the first text alternative, so a GIF with no good match becomes a normal
    /// reply instead of a bad GIF or silence. If there is no usable
    /// alternative, `selected` is returned unchanged.
    pub async fn choose_resolvable(&self, selected: String, alternatives: &[String]) -> String {
        if !selected.contains("GIF:") {
            return selected;
        }
        // Resolution is memoized, so the caller resolving it again is cheap.
        if self.try_resolve_gif(&selected).await.is_some()
            || self.try_resolve_embedded_gif(&selected).await.is_some()
        {
            return selected;
        }
        match alternatives
            .iter()
            .find(|alt| !alt.contains("GIF:") && !alt.trim().is_empty())
        {
            Some(alt) => {
                info!("GIF request had no relevant match; using a text alternative instead");
                alt.clone()
            }
            None => selected,
        }
    }
}

/// Lowercased alphanumeric words of a string.
fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect()
}

/// Significant words of a query: filler words removed, duplicates dropped,
/// original order kept.
fn keywords(query: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    tokenize(query)
        .into_iter()
        .filter(|w| w.len() >= 2 && !STOPWORDS.contains(&w.as_str()))
        .filter(|w| seen.insert(w.clone()))
        .collect()
}

/// Crude stemmer so "spinning" matches "spin" and "dancing" matches "dance".
/// Only needs to be consistent between query words and title words.
fn stem(word: &str) -> String {
    let word = word.to_lowercase();
    let mut base = word.as_str();
    let mut verb_suffix = false;
    for suffix in ["ing", "ed", "es", "s"] {
        if let Some(stripped) = base.strip_suffix(suffix) {
            if stripped.len() >= 3 {
                base = stripped;
                verb_suffix = suffix == "ing" || suffix == "ed";
                break;
            }
        }
    }
    let mut out = base.to_string();
    // Undouble a final consonant added before -ing/-ed: spinn -> spin
    if verb_suffix {
        let bytes = out.as_bytes();
        let n = bytes.len();
        if n >= 2 && bytes[n - 1] == bytes[n - 2] && b"bdgmnprt".contains(&bytes[n - 1]) {
            out.pop();
        }
    }
    // dance / dancing / dances -> danc
    if out.len() > 3 && out.ends_with('e') {
        out.pop();
    }
    out
}

/// How many keywords a GIF's title must match to count as relevant: all of
/// them for short queries, about half for longer ones.
fn required_matches(keyword_count: usize) -> usize {
    if keyword_count <= 2 {
        keyword_count
    } else {
        keyword_count.div_ceil(2)
    }
}

/// Number of query keywords found in a candidate's title or slug.
fn score(keyword_stems: &[String], candidate: &GifCandidate) -> usize {
    let words: HashSet<String> = tokenize(&candidate.title)
        .into_iter()
        .chain(tokenize(&candidate.slug))
        .map(|w| stem(&w))
        .collect();
    keyword_stems.iter().filter(|k| words.contains(*k)).count()
}

/// The most relevant candidate (ties go to Giphy's own ranking), or None if
/// none matches enough of the query's keywords.
fn best_candidate(
    wanted: &[String],
    candidates: Vec<GifCandidate>,
) -> Option<(GifCandidate, usize)> {
    let needed = required_matches(wanted.len());
    let wanted_stems: Vec<String> = wanted.iter().map(|w| stem(w)).collect();

    let mut best: Option<(GifCandidate, usize)> = None;
    for candidate in candidates {
        let matched = score(&wanted_stems, &candidate);
        if matched < needed {
            continue;
        }
        if best.as_ref().is_none_or(|(_, b)| matched > *b) {
            best = Some((candidate, matched));
        }
    }
    best
}

/// Pull candidates out of a Giphy search response. Prefers the downsized
/// rendition (smaller, embeds reliably) and falls back to the original.
fn parse_candidates(json: &serde_json::Value) -> Vec<GifCandidate> {
    let Some(items) = json.get("data").and_then(|d| d.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let url = item
                .pointer("/images/downsized/url")
                .and_then(|u| u.as_str())
                .or_else(|| {
                    item.pointer("/images/original/url")
                        .and_then(|u| u.as_str())
                })?;
            Some(GifCandidate {
                url: url.to_string(),
                title: item
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string(),
                slug: item
                    .get("slug")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect()
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

    fn candidate(title: &str) -> GifCandidate {
        GifCandidate {
            url: format!("https://media.giphy.com/{}.gif", title.replace(' ', "-")),
            title: title.to_string(),
            slug: title.to_lowercase().replace(' ', "-"),
        }
    }

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
    fn keywords_drop_filler_and_duplicates() {
        assert_eq!(
            keywords("Sandra Bullock spinning in space"),
            vec!["sandra", "bullock", "spinning", "space"]
        );
        assert_eq!(keywords("the the cat and the cat"), vec!["cat"]);
        assert!(keywords("in the").is_empty());
    }

    #[test]
    fn stemming_unifies_common_word_forms() {
        assert_eq!(stem("spinning"), stem("spin"));
        assert_eq!(stem("dancing"), stem("dance"));
        assert_eq!(stem("dances"), stem("dance"));
        assert_eq!(stem("falling"), stem("fall"));
        assert_eq!(stem("hugged"), stem("hug"));
        assert_ne!(stem("space"), stem("spin"));
    }

    #[test]
    fn required_matches_scales_with_query_length() {
        assert_eq!(required_matches(0), 0);
        assert_eq!(required_matches(1), 1);
        assert_eq!(required_matches(2), 2);
        assert_eq!(required_matches(3), 2);
        assert_eq!(required_matches(4), 2);
        assert_eq!(required_matches(5), 3);
    }

    #[test]
    fn search_terms_have_no_generic_or_random_fallback() {
        let terms = GiphyClient::search_terms("Sandra Bullock spinning in space");
        assert_eq!(
            terms,
            vec![
                "Sandra Bullock spinning in space",
                "sandra bullock spinning space",
                "sandra bullock spinning",
            ]
        );
        assert!(!terms.iter().any(|t| t == "shrug"));

        // A short query is just itself.
        assert_eq!(GiphyClient::search_terms("shrug"), vec!["shrug"]);
    }

    #[test]
    fn rejects_irrelevant_top_result() {
        // Giphy's top hit for a specific query is often unrelated; it must not
        // be accepted just for ranking first.
        let wanted = keywords("Sandra Bullock spinning in space");
        let results = vec![candidate("Cat Dancing GIF"), candidate("Mind Blown GIF")];
        assert!(best_candidate(&wanted, results).is_none());
    }

    #[test]
    fn picks_the_most_relevant_result_not_the_first() {
        let wanted = keywords("Sandra Bullock spinning in space");
        let results = vec![
            candidate("Space Cat GIF"),
            candidate("Sandra Bullock GIF"),
            candidate("Sandra Bullock Spinning In Space GIF"),
        ];
        let (best, matched) = best_candidate(&wanted, results).unwrap();
        assert_eq!(best.title, "Sandra Bullock Spinning In Space GIF");
        assert_eq!(matched, 4);
    }

    #[test]
    fn ties_go_to_giphys_own_ranking() {
        let wanted = keywords("mind blown");
        let results = vec![candidate("Mind Blown GIF"), candidate("Blown Mind Wow GIF")];
        let (best, _) = best_candidate(&wanted, results).unwrap();
        assert_eq!(best.title, "Mind Blown GIF");
    }

    #[test]
    fn short_queries_require_every_keyword() {
        let wanted = keywords("mind blown");
        assert!(best_candidate(&wanted, vec![candidate("Blown Away GIF")]).is_none());
    }

    #[test]
    fn parses_candidates_and_skips_entries_without_images() {
        let json = serde_json::json!({
            "data": [
                {"title": "A GIF", "slug": "a-gif",
                 "images": {"downsized": {"url": "https://x/d.gif"}, "original": {"url": "https://x/o.gif"}}},
                {"title": "Only original", "slug": "only-original",
                 "images": {"original": {"url": "https://x/o2.gif"}}},
                {"title": "No images", "slug": "none"}
            ]
        });
        let parsed = parse_candidates(&json);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].url, "https://x/d.gif");
        assert_eq!(parsed[1].url, "https://x/o2.gif");
        assert!(parse_candidates(&serde_json::json!({"meta": {}})).is_empty());
    }

    #[tokio::test]
    async fn non_gif_responses_pass_through_untouched() {
        let client = GiphyClient::new("test-key".to_string());
        let chosen = client
            .choose_resolvable("Just a text reply".to_string(), &["other".to_string()])
            .await;
        assert_eq!(chosen, "Just a text reply");
    }

    #[tokio::test]
    async fn unresolvable_gif_falls_back_to_first_text_alternative() {
        let client = GiphyClient::new("test-key".to_string());
        // Pre-seed a known miss so no network is needed.
        client.memo_put("sandra bullock spinning in space".to_string(), None);

        let chosen = client
            .choose_resolvable(
                "GIF: Sandra Bullock spinning in space".to_string(),
                &[
                    "GIF: another gif".to_string(),
                    "A text reply".to_string(),
                    "A second text reply".to_string(),
                ],
            )
            .await;
        assert_eq!(chosen, "A text reply");
    }

    #[tokio::test]
    async fn unresolvable_gif_with_no_text_alternative_is_left_for_caller() {
        let client = GiphyClient::new("test-key".to_string());
        client.memo_put("nothing matches".to_string(), None);
        let chosen = client
            .choose_resolvable("GIF: nothing matches".to_string(), &[])
            .await;
        assert_eq!(chosen, "GIF: nothing matches");
    }

    #[tokio::test]
    async fn resolvable_gif_is_kept() {
        let client = GiphyClient::new("test-key".to_string());
        client.memo_put(
            "mind blown".to_string(),
            Some("https://media.giphy.com/mind.gif".to_string()),
        );
        let chosen = client
            .choose_resolvable("GIF: mind blown".to_string(), &["A text reply".to_string()])
            .await;
        assert_eq!(chosen, "GIF: mind blown");
    }
}
