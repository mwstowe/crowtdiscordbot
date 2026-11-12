use anyhow::Result;
use base64::Engine;
use lazy_static::lazy_static;
use regex::Regex;
use serenity::model::channel::Message;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{error, info};

/// Media in conversation context older than this is not attached to Gemini
/// requests. Without a limit, an image posted hours ago (e.g. a command
/// response) gets re-sent and commented on as if it were current.
pub const CONTEXT_MEDIA_MAX_AGE: Duration = Duration::from_secs(30 * 60);

/// Discord snowflake epoch (2015-01-01T00:00:00Z) in Unix milliseconds.
const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;

/// Upper bound on base64 data kept in the download cache.
const MAX_CACHE_BYTES: usize = 48_000_000;

lazy_static! {
    /// Discord CDN attachment URLs embed the attachment's snowflake ID, which
    /// encodes its creation time: .../attachments/<channel>/<attachment>/<file>
    static ref ATTACHMENT_ID_REGEX: Regex = Regex::new(r"/attachments/\d+/(\d+)/").unwrap();
    static ref YOUTUBE_URL_REGEX: Regex = Regex::new(
        r"(?:https?://)?(?:www\.)?(?:youtube\.com/watch\?v=|youtu\.be/|youtube\.com/shorts/)[\w\-]+"
    )
    .unwrap();
    static ref MEDIA_URL_REGEX: Regex =
        Regex::new(r"\[(Image|Video|GIF): [^|]+ \| ([^|]+) \| (https?://[^\]]+)\]").unwrap();
    static ref MEDIA_STRIP_REGEX: Regex =
        Regex::new(r"\[(Image|Video|GIF): ([^|]+) \| [^|]+ \| https?://[^\]]+\]").unwrap();
}

/// A media item extracted from a Discord message
#[derive(Debug, Clone)]
pub struct MediaItem {
    pub mime_type: String,
    pub data: String, // base64-encoded
    /// Display name of the person who posted this media, if known from context.
    pub author: Option<String>,
}

/// A YouTube URL found in message text
#[derive(Debug, Clone)]
pub struct YouTubeUrl {
    pub url: String,
}

const MAX_INLINE_SIZE: usize = 15_000_000; // ~15MB before base64

const IMAGE_TYPES: &[&str] = &["image/png", "image/jpeg", "image/webp", "image/gif"];
const VIDEO_TYPES: &[&str] = &[
    "video/mp4",
    "video/webm",
    "video/quicktime",
    "video/mpeg",
    "video/x-flv",
];

/// Extract downloadable image/video attachments from a message
pub async fn extract_media_from_message(
    http_client: &reqwest::Client,
    msg: &Message,
) -> Vec<MediaItem> {
    let mut items = Vec::new();

    for attachment in &msg.attachments {
        let content_type = attachment.content_type.as_deref().unwrap_or("");

        let is_image = IMAGE_TYPES.iter().any(|t| content_type.starts_with(t));
        let is_video = VIDEO_TYPES.iter().any(|t| content_type.starts_with(t));

        if !is_image && !is_video {
            continue;
        }

        if attachment.size as usize > MAX_INLINE_SIZE {
            info!(
                "Skipping attachment {} ({} bytes) - too large for inline",
                attachment.filename, attachment.size
            );
            continue;
        }

        match download_and_encode(http_client, &attachment.url).await {
            Ok(data) => {
                items.push(MediaItem {
                    mime_type: content_type.to_string(),
                    data,
                    author: None,
                });
                info!(
                    "Extracted media: {} ({}, {} bytes)",
                    attachment.filename, content_type, attachment.size
                );
            }
            Err(e) => {
                error!(
                    "Failed to download attachment {}: {:?}",
                    attachment.filename, e
                );
            }
        }
    }

    // Also check referenced message (reply-to) for attachments
    if let Some(ref referenced) = msg.referenced_message {
        for attachment in &referenced.attachments {
            let content_type = attachment.content_type.as_deref().unwrap_or("");

            let is_image = IMAGE_TYPES.iter().any(|t| content_type.starts_with(t));
            let is_video = VIDEO_TYPES.iter().any(|t| content_type.starts_with(t));

            if !is_image && !is_video {
                continue;
            }

            if attachment.size as usize > MAX_INLINE_SIZE {
                continue;
            }

            match download_and_encode(http_client, &attachment.url).await {
                Ok(data) => {
                    items.push(MediaItem {
                        mime_type: content_type.to_string(),
                        data,
                        author: None,
                    });
                    info!(
                        "Extracted media from referenced message: {} ({})",
                        attachment.filename, content_type
                    );
                }
                Err(e) => {
                    error!(
                        "Failed to download referenced attachment {}: {:?}",
                        attachment.filename, e
                    );
                }
            }
        }
    }

    items
}

/// Extract YouTube URLs from message text
pub fn extract_youtube_urls(text: &str) -> Vec<YouTubeUrl> {
    YOUTUBE_URL_REGEX
        .find_iter(text)
        .map(|m| YouTubeUrl {
            url: m.as_str().to_string(),
        })
        .collect()
}

/// Describe attachments, stickers, and embeds as text tags for context storage
/// (image/video attachments include the URL for later retrieval).
pub fn describe_attachments(msg: &Message) -> String {
    let mut tags = Vec::new();

    // File attachments (images, videos, other files)
    for attachment in &msg.attachments {
        let content_type = attachment.content_type.as_deref().unwrap_or("unknown");
        if IMAGE_TYPES.iter().any(|t| content_type.starts_with(t)) {
            tags.push(format!(
                "[Image: {} | {} | {}]",
                attachment.filename, content_type, attachment.url
            ));
        } else if VIDEO_TYPES.iter().any(|t| content_type.starts_with(t)) {
            tags.push(format!(
                "[Video: {} | {} | {}]",
                attachment.filename, content_type, attachment.url
            ));
        } else {
            tags.push(format!("[File: {}]", attachment.filename));
        }
    }

    // Stickers (posted with no text, otherwise stored as empty content)
    for sticker in &msg.sticker_items {
        tags.push(format!("[Sticker: {}]", sticker.name));
    }

    // Embeds (GIFs from Tenor/Giphy, unfurled links, etc.)
    for embed in &msg.embeds {
        // An embed that just mirrors an attachment we already tagged adds no value;
        // but standalone GIF/image/link embeds are the common "empty message" source.
        let kind = embed.kind.as_deref().unwrap_or("");
        if let Some(url) = &embed.url {
            if kind == "gifv" || kind == "video" {
                tags.push(format!("[GIF: animated | image/gif | {url}]"));
            } else if kind == "image" {
                tags.push(format!("[Image: embed | image/gif | {url}]"));
            } else {
                // Link/article embed - include title if available for context
                match &embed.title {
                    Some(title) => tags.push(format!("[Link: {title} | {url}]")),
                    None => tags.push(format!("[Link: {url}]")),
                }
            }
        }
    }

    tags.join(" ")
}

/// Rules for which media in the conversation context may be attached to a request.
#[derive(Debug, Clone, Default)]
pub struct MediaContextPolicy {
    /// The bot's own name. Media posted by the bot (including command
    /// responses like `!frinkiac`) is never attached from context: the model
    /// ends up commenting on the bot's own output as though it were new.
    pub bot_name: String,
    /// Skip media older than this. Only applies to media whose age can be
    /// determined (Discord CDN attachments); other URLs are left eligible.
    pub max_age: Option<Duration>,
}

impl MediaContextPolicy {
    /// Standard policy: exclude the bot's own media and anything stale.
    pub fn standard(bot_name: &str) -> Self {
        Self {
            bot_name: bot_name.to_string(),
            max_age: Some(CONTEXT_MEDIA_MAX_AGE),
        }
    }
}

/// True if a context-line author is the bot itself. Context lines label the
/// bot as "Crow", "Crow (you)" or "Crow (you) [command response]".
fn is_bot_author(author: &str, bot_name: &str) -> bool {
    let bot = bot_name.trim().to_lowercase();
    if bot.is_empty() {
        return false;
    }
    let author = author.trim().to_lowercase();
    author == bot || author.starts_with(&format!("{bot} (you)"))
}

/// Creation time (Unix ms) of a Discord CDN attachment, decoded from the
/// snowflake ID in its URL. None for URLs that aren't Discord attachments.
fn attachment_timestamp_ms(url: &str) -> Option<u64> {
    let id: u64 = ATTACHMENT_ID_REGEX
        .captures(url)?
        .get(1)?
        .as_str()
        .parse()
        .ok()?;
    Some((id >> 22) + DISCORD_EPOCH_MS)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Extract image/video URLs from context text along with the posting author.
/// Context lines are formatted "DisplayName: content...[Image: ...]", so the
/// author is the text before the first ": " on the line containing the media tag.
/// Media excluded by `policy` is dropped first, then the `max_items` most
/// recent remaining items are returned.
pub fn extract_media_urls_from_context(
    text: &str,
    max_items: usize,
    policy: &MediaContextPolicy,
    now_ms: u64,
) -> Vec<(String, String, Option<String>)> {
    let mut items: Vec<(String, String, Option<String>)> = Vec::new();

    for line in text.lines() {
        // Author is the segment before the first ": " on the line, if present.
        let author = line
            .find(": ")
            .map(|idx| line[..idx].trim().to_string())
            .filter(|a| !a.is_empty());

        if let Some(a) = &author {
            if is_bot_author(a, &policy.bot_name) {
                continue;
            }
        }

        for cap in MEDIA_URL_REGEX.captures_iter(line) {
            let mime = cap[2].trim().to_string();
            let url = cap[3].trim().to_string();

            if let (Some(max_age), Some(posted_ms)) =
                (policy.max_age, attachment_timestamp_ms(&url))
            {
                if now_ms.saturating_sub(posted_ms) > max_age.as_millis() as u64 {
                    continue;
                }
            }

            items.push((mime, url, author.clone()));
        }
    }

    // Keep only the most recent items
    if items.len() > max_items {
        items = items.split_off(items.len() - max_items);
    }
    items
}

/// Strip media URLs from context text for display (keep just filename)
pub fn strip_media_urls_from_context(text: &str) -> String {
    MEDIA_STRIP_REGEX
        .replace_all(text, |caps: &regex::Captures| {
            let kind = &caps[1];
            let name = caps[2].trim();
            format!("[{kind}: {name}]")
        })
        .to_string()
}

/// Download media items from URLs found in context text.
/// Returns up to max_items MediaItems that pass `policy`, silently skipping
/// failures. Downloads are served from `cache` when possible, so the same
/// image isn't re-fetched on every request while it stays in context.
pub async fn fetch_media_from_context(
    http_client: &reqwest::Client,
    cache: &MediaCache,
    text: &str,
    max_items: usize,
    policy: &MediaContextPolicy,
) -> Vec<MediaItem> {
    let urls = extract_media_urls_from_context(text, max_items, policy, now_ms());
    let mut items = Vec::new();
    for (mime, url, author) in urls {
        if let Some(data) = cache.get(&url) {
            info!("Context media cache hit: {} posted by {:?}", url, author);
            items.push(MediaItem {
                mime_type: mime,
                data,
                author,
            });
            continue;
        }
        match download_and_encode(http_client, &url).await {
            Ok(data) => {
                info!(
                    "Fetched context media: {} ({}) posted by {:?}",
                    url, mime, author
                );
                cache.insert(&url, data.clone());
                items.push(MediaItem {
                    mime_type: mime,
                    data,
                    author,
                });
            }
            Err(e) => {
                info!(
                    "Failed to fetch context media {}: {:?} (may be expired)",
                    url, e
                );
            }
        }
    }
    items
}

/// Small in-memory cache of downloaded (base64) media, bounded by total size.
/// Keyed by URL without its query string: Discord's signed CDN parameters
/// change when links are refreshed, but the attachment ID in the path doesn't.
#[derive(Clone, Default)]
pub struct MediaCache {
    inner: Arc<Mutex<CacheState>>,
}

#[derive(Default)]
struct CacheState {
    entries: VecDeque<(String, String)>,
    bytes: usize,
}

impl MediaCache {
    fn key(url: &str) -> &str {
        url.split('?').next().unwrap_or(url)
    }

    pub fn get(&self, url: &str) -> Option<String> {
        let key = Self::key(url);
        let state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state
            .entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, data)| data.clone())
    }

    pub fn insert(&self, url: &str, data: String) {
        if data.len() > MAX_CACHE_BYTES {
            return;
        }
        let key = Self::key(url).to_string();
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());

        if let Some(pos) = state.entries.iter().position(|(k, _)| *k == key) {
            if let Some((_, old)) = state.entries.remove(pos) {
                state.bytes -= old.len();
            }
        }
        state.bytes += data.len();
        state.entries.push_back((key, data));

        // Evict oldest entries until back under the size cap.
        while state.bytes > MAX_CACHE_BYTES {
            match state.entries.pop_front() {
                Some((_, evicted)) => state.bytes -= evicted.len(),
                None => break,
            }
        }
    }
}

/// Download a URL and return base64-encoded content
async fn download_and_encode(http_client: &reqwest::Client, url: &str) -> Result<String> {
    let response = http_client.get(url).send().await?;
    if !response.status().is_success() {
        return Err(anyhow::anyhow!("HTTP {}", response.status()));
    }
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_INLINE_SIZE {
        return Err(anyhow::anyhow!("Too large: {} bytes", bytes.len()));
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No filtering: isolates attribution/capping behavior from the policy.
    fn open() -> MediaContextPolicy {
        MediaContextPolicy::default()
    }

    /// Build a Discord attachment snowflake for a given creation time.
    fn snowflake_at(ms: u64) -> u64 {
        (ms - DISCORD_EPOCH_MS) << 22
    }

    fn cdn_url(ms: u64, file: &str) -> String {
        format!(
            "https://cdn.discordapp.com/attachments/390158211082354691/{}/{}?ex=1&is=2&hm=3&",
            snowflake_at(ms),
            file
        )
    }

    #[test]
    fn attributes_image_to_the_line_author() {
        // Mirrors the Chuck E. Cheese incident: Rumm posts the image, Raccoon
        // posts an empty message afterward.
        let context = "Rumm: I liked this one: [Image: img.png | image/png | https://cdn.example.com/a.png]\n\
                       del23: i knew she looked familiar\n\
                       Raccoon: ";
        let items = extract_media_urls_from_context(context, 3, &open(), 0);
        assert_eq!(items.len(), 1);
        let (mime, url, author) = &items[0];
        assert_eq!(mime, "image/png");
        assert_eq!(url, "https://cdn.example.com/a.png");
        assert_eq!(author.as_deref(), Some("Rumm"));
    }

    #[test]
    fn caps_to_most_recent_items() {
        let context = "a: [Image: 1 | image/png | https://e.com/1.png]\n\
                       b: [Image: 2 | image/png | https://e.com/2.png]\n\
                       c: [Image: 3 | image/png | https://e.com/3.png]\n\
                       d: [Image: 4 | image/png | https://e.com/4.png]";
        let items = extract_media_urls_from_context(context, 2, &open(), 0);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].2.as_deref(), Some("c"));
        assert_eq!(items[1].2.as_deref(), Some("d"));
    }

    #[test]
    fn gif_tag_is_extracted_and_stripped() {
        let context = "Rumm: [GIF: animated | image/gif | https://tenor.com/x.gif]";
        let items = extract_media_urls_from_context(context, 3, &open(), 0);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].2.as_deref(), Some("Rumm"));

        let stripped = strip_media_urls_from_context(context);
        assert!(!stripped.contains("https://"));
        assert!(stripped.contains("[GIF:"));
    }

    #[test]
    fn strip_removes_url_keeps_name() {
        let context = "x: [Image: photo.jpg | image/jpeg | https://cdn.example.com/p.jpg]";
        let stripped = strip_media_urls_from_context(context);
        assert_eq!(stripped, "x: [Image: photo.jpg]");
    }

    #[test]
    fn bot_authors_are_recognized_in_all_context_labels() {
        assert!(is_bot_author("Crow", "Crow"));
        assert!(is_bot_author("crow", "Crow"));
        assert!(is_bot_author("Crow (you)", "Crow"));
        assert!(is_bot_author("Crow (you) [command response]", "Crow"));
        assert!(!is_bot_author("Crowbar", "Crow"));
        assert!(!is_bot_author("Rumm", "Crow"));
        assert!(!is_bot_author("Crow", ""));
    }

    #[test]
    fn bot_posted_media_is_excluded() {
        // The 2026-11-12 incident: the bot's own !frinkiac GIF was re-attached
        // and commented on after hours of silence.
        let now = 1_800_000_000_000;
        let context = format!(
            "Queued: !frinkiac first you get the sugar\n\
             Crow: Lisa's Rival (Season 6, Episode 2) [Image: frinkiac.gif | image/gif | {}]\n\
             Rumm: look at this [Image: p.png | image/png | {}]",
            cdn_url(now - 60_000, "frinkiac.gif"),
            cdn_url(now - 60_000, "p.png"),
        );
        let policy = MediaContextPolicy::standard("Crow");
        let items = extract_media_urls_from_context(&context, 3, &policy, now);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].2.as_deref(), Some("Rumm"));
    }

    #[test]
    fn stale_discord_attachments_are_excluded() {
        let now = 1_800_000_000_000;
        let two_hours = 2 * 60 * 60 * 1000;
        let context = format!(
            "Rumm: old [Image: old.png | image/png | {}]\n\
             Rumm: new [Image: new.png | image/png | {}]",
            cdn_url(now - two_hours, "old.png"),
            cdn_url(now - 5 * 60 * 1000, "new.png"),
        );
        let policy = MediaContextPolicy::standard("Crow");
        let items = extract_media_urls_from_context(&context, 3, &policy, now);
        assert_eq!(items.len(), 1);
        assert!(items[0].1.contains("new.png"));
    }

    #[test]
    fn media_with_unknown_age_stays_eligible() {
        // Non-Discord URLs carry no timestamp; behavior is unchanged for them.
        let context = "Rumm: [Image: embed | image/gif | https://i.imgur.com/x.gif]";
        let policy = MediaContextPolicy::standard("Crow");
        let items = extract_media_urls_from_context(context, 3, &policy, 1_800_000_000_000);
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn cap_applies_after_filtering() {
        // Excluded items must not use up the cap and crowd out eligible ones.
        let now = 1_800_000_000_000;
        let context = format!(
            "Rumm: [Image: a.png | image/png | {}]\n\
             Crow: [Image: b.png | image/png | {}]\n\
             Crow: [Image: c.png | image/png | {}]",
            cdn_url(now - 1000, "a.png"),
            cdn_url(now - 1000, "b.png"),
            cdn_url(now - 1000, "c.png"),
        );
        let policy = MediaContextPolicy::standard("Crow");
        let items = extract_media_urls_from_context(&context, 1, &policy, now);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].2.as_deref(), Some("Rumm"));
    }

    #[test]
    fn attachment_age_is_decoded_from_snowflake() {
        let ms = 1_794_000_000_000;
        assert_eq!(attachment_timestamp_ms(&cdn_url(ms, "x.gif")), Some(ms));
        assert_eq!(attachment_timestamp_ms("https://i.imgur.com/x.gif"), None);
    }

    #[test]
    fn cache_ignores_query_string_and_evicts_oldest() {
        let cache = MediaCache::default();
        cache.insert(
            "https://cdn.example.com/attachments/1/2/a.png?ex=1",
            "AAA".into(),
        );
        assert_eq!(
            cache.get("https://cdn.example.com/attachments/1/2/a.png?ex=999&hm=z"),
            Some("AAA".to_string())
        );
        assert_eq!(
            cache.get("https://cdn.example.com/attachments/1/3/b.png"),
            None
        );

        // Fill past the cap with large entries; the oldest must go first.
        let big = "x".repeat(MAX_CACHE_BYTES / 2 + 1);
        cache.insert("https://e.com/1", big.clone());
        cache.insert("https://e.com/2", big);
        assert_eq!(cache.get("https://e.com/1"), None);
        assert!(cache.get("https://e.com/2").is_some());
    }
}
