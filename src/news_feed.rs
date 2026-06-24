use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::info;

/// A headline from a news feed
#[derive(Debug, Clone)]
pub struct Headline {
    pub title: String,
    pub url: String,
    pub source: String,
}

/// Shared cache of recent headlines
pub type HeadlineCache = Arc<RwLock<Vec<Headline>>>;

/// Create a new headline cache
pub fn new_cache() -> HeadlineCache {
    Arc::new(RwLock::new(Vec::new()))
}

/// Default feeds used when none are configured
const DEFAULT_FEEDS: &[(&str, &str)] = &[
    (
        "https://feeds.arstechnica.com/arstechnica/index",
        "Ars Technica",
    ),
    (
        "https://feeds.bbci.co.uk/news/technology/rss.xml",
        "BBC News",
    ),
    ("https://rss.slashdot.org/Slashdot/slashdotMain", "Slashdot"),
    ("https://gizmodo.com/feed", "Gizmodo"),
    (
        "https://rss.nytimes.com/services/xml/rss/nyt/HomePage.xml",
        "NYT",
    ),
    ("https://www.them.us/feed/rss", "them."),
    ("https://www.odditycentral.com/feed", "Oddity Central"),
];

/// Parse feed config string. Format: "url|name, url|name, ..."
/// If prefixed with "+", appends to defaults. Otherwise replaces them.
fn parse_feed_config(config: &str) -> (bool, Vec<(String, String)>) {
    let (append, config) = if let Some(rest) = config.strip_prefix('+') {
        (true, rest)
    } else {
        (false, config)
    };

    let feeds: Vec<(String, String)> = config
        .split(',')
        .filter_map(|entry| {
            let entry = entry.trim();
            if entry.is_empty() {
                return None;
            }
            if let Some((url, name)) = entry.split_once('|') {
                Some((url.trim().to_string(), name.trim().to_string()))
            } else {
                // If no name provided, derive from URL
                let url = entry.trim();
                let name = url
                    .replace("https://", "")
                    .replace("http://", "")
                    .split('/')
                    .next()
                    .unwrap_or(url)
                    .replace("www.", "")
                    .replace("feeds.", "")
                    .replace("rss.", "");
                Some((url.to_string(), name))
            }
        })
        .collect();

    (append, feeds)
}

/// Spawn a background task that refreshes headlines every `interval` seconds
pub fn spawn_fetcher(cache: HeadlineCache, interval_secs: u64, feed_config: Option<String>) {
    // Build the feed list
    let feeds: Vec<(String, String)> = match feed_config {
        Some(ref config) => {
            let (append, custom) = parse_feed_config(config);
            if append {
                let mut all: Vec<(String, String)> = DEFAULT_FEEDS
                    .iter()
                    .map(|(u, n)| (u.to_string(), n.to_string()))
                    .collect();
                all.extend(custom);
                all
            } else {
                custom
            }
        }
        None => DEFAULT_FEEDS
            .iter()
            .map(|(u, n)| (u.to_string(), n.to_string()))
            .collect(),
    };

    info!("News feeds configured: {} sources", feeds.len());
    for (url, name) in &feeds {
        info!("  - {} ({})", name, url);
    }

    tokio::spawn(async move {
        loop {
            let headlines = fetch_feeds(&feeds).await;
            if !headlines.is_empty() {
                info!("Fetched {} headlines from news feeds", headlines.len());
                let mut c = cache.write().await;
                *c = headlines;
            }
            tokio::time::sleep(Duration::from_secs(interval_secs)).await;
        }
    });
}

/// Fetch headlines from the configured feeds
async fn fetch_feeds(feeds: &[(String, String)]) -> Vec<Headline> {
    let mut all = Vec::new();

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("Mozilla/5.0 (compatible; CrowBot/1.0)")
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .unwrap_or_default();

    for (url, name) in feeds {
        if let Some(mut items) = fetch_rss(&client, url, name).await {
            items.truncate(15);
            all.append(&mut items);
        }
    }

    all
}

/// Parse an RSS feed and extract headlines
async fn fetch_rss(client: &reqwest::Client, url: &str, source: &str) -> Option<Vec<Headline>> {
    let body = client.get(url).send().await.ok()?.text().await.ok()?;
    let mut headlines = Vec::new();

    // Simple XML parsing for <item><title>...</title><link>...</link></item>
    for item in body.split("<item>").skip(1) {
        let title = extract_xml_tag(item, "title")?;
        let link = extract_xml_tag(item, "link").or_else(|| extract_xml_tag(item, "guid"))?;

        if !title.is_empty() && link.starts_with("http") {
            headlines.push(Headline {
                title: clean_cdata(&title),
                url: link,
                source: source.to_string(),
            });
        }
    }

    Some(headlines)
}

/// Extract content between XML tags (handles CDATA)
fn extract_xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let start = xml.find(&open)?;
    let after_open = &xml[start..];
    let content_start = after_open.find('>')? + 1;
    let content = &after_open[content_start..];
    let end = content.find(&close)?;
    Some(content[..end].trim().to_string())
}

/// Strip CDATA wrappers and HTML entities
fn clean_cdata(s: &str) -> String {
    s.replace("<![CDATA[", "")
        .replace("]]>", "")
        .replace("&#8217;", "'")
        .replace("&#8216;", "'")
        .replace("&#8220;", "\"")
        .replace("&#8221;", "\"")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .trim()
        .to_string()
}
