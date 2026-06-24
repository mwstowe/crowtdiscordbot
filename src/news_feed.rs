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

/// Spawn a background task that refreshes headlines every `interval` seconds
pub fn spawn_fetcher(cache: HeadlineCache, interval_secs: u64) {
    tokio::spawn(async move {
        loop {
            let headlines = fetch_all_feeds().await;
            if !headlines.is_empty() {
                info!("Fetched {} headlines from news feeds", headlines.len());
                let mut c = cache.write().await;
                *c = headlines;
            }
            tokio::time::sleep(Duration::from_secs(interval_secs)).await;
        }
    });
}

/// Fetch headlines from all configured feeds
async fn fetch_all_feeds() -> Vec<Headline> {
    let mut all = Vec::new();

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_default();

    // Ars Technica RSS
    if let Some(mut items) = fetch_rss(
        &client,
        "https://feeds.arstechnica.com/arstechnica/index",
        "Ars Technica",
    )
    .await
    {
        items.truncate(15);
        all.append(&mut items);
    }

    // BBC News Technology RSS
    if let Some(mut items) = fetch_rss(
        &client,
        "https://feeds.bbci.co.uk/news/technology/rss.xml",
        "BBC News",
    )
    .await
    {
        items.truncate(15);
        all.append(&mut items);
    }

    // Slashdot RSS (tech/nerd/weird news)
    if let Some(mut items) = fetch_rss(
        &client,
        "https://rss.slashdot.org/Slashdot/slashdotMain",
        "Slashdot",
    )
    .await
    {
        items.truncate(15);
        all.append(&mut items);
    }

    // Gizmodo RSS (tech/science/pop culture)
    if let Some(mut items) = fetch_rss(&client, "https://gizmodo.com/feed", "Gizmodo").await {
        items.truncate(10);
        all.append(&mut items);
    }

    // NYT Top Stories RSS (general/breaking news)
    if let Some(mut items) = fetch_rss(
        &client,
        "https://rss.nytimes.com/services/xml/rss/nyt/HomePage.xml",
        "NYT",
    )
    .await
    {
        items.truncate(10);
        all.append(&mut items);
    }

    // them.us RSS (LGBTQ+ news)
    if let Some(mut items) = fetch_rss(&client, "https://www.them.us/feed/rss", "them.").await {
        items.truncate(10);
        all.append(&mut items);
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

/// Fetch top stories from Hacker News API
/// Extract content between XML tags (handles CDATA)
fn extract_xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let start = xml.find(&open)?;
    let after_open = &xml[start..];
    // Skip past the opening tag (handles attributes)
    let content_start = after_open.find('>')? + 1;
    let content = &after_open[content_start..];
    let end = content.find(&close)?;
    Some(content[..end].trim().to_string())
}

/// Strip CDATA wrappers
fn clean_cdata(s: &str) -> String {
    s.replace("<![CDATA[", "")
        .replace("]]>", "")
        .trim()
        .to_string()
}
