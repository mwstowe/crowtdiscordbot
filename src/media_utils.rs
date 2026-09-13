use anyhow::Result;
use base64::Engine;
use lazy_static::lazy_static;
use regex::Regex;
use serenity::model::channel::Message;
use tracing::{error, info};

lazy_static! {
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

/// Extract image/video URLs from context text along with the posting author.
/// Context lines are formatted "DisplayName: content...[Image: ...]", so the
/// author is the text before the first ": " on the line containing the media tag.
/// Returns up to `max_items` most recent items (from end of text).
pub fn extract_media_urls_from_context(
    text: &str,
    max_items: usize,
) -> Vec<(String, String, Option<String>)> {
    let mut items: Vec<(String, String, Option<String>)> = Vec::new();

    for line in text.lines() {
        // Author is the segment before the first ": " on the line, if present.
        let author = line
            .find(": ")
            .map(|idx| line[..idx].trim().to_string())
            .filter(|a| !a.is_empty());

        for cap in MEDIA_URL_REGEX.captures_iter(line) {
            let mime = cap[2].trim().to_string();
            let url = cap[3].trim().to_string();
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
/// Returns up to max_items MediaItems, silently skipping failures.
pub async fn fetch_media_from_context(
    http_client: &reqwest::Client,
    text: &str,
    max_items: usize,
) -> Vec<MediaItem> {
    let urls = extract_media_urls_from_context(text, max_items);
    let mut items = Vec::new();
    for (mime, url, author) in urls {
        match download_and_encode(http_client, &url).await {
            Ok(data) => {
                info!(
                    "Fetched context media: {} ({}) posted by {:?}",
                    url, mime, author
                );
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

    #[test]
    fn attributes_image_to_the_line_author() {
        // Mirrors the Chuck E. Cheese incident: Rumm posts the image, Raccoon
        // posts an empty message afterward.
        let context = "Rumm: I liked this one: [Image: img.png | image/png | https://cdn.example.com/a.png]\n\
                       del23: i knew she looked familiar\n\
                       Raccoon: ";
        let items = extract_media_urls_from_context(context, 3);
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
        let items = extract_media_urls_from_context(context, 2);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].2.as_deref(), Some("c"));
        assert_eq!(items[1].2.as_deref(), Some("d"));
    }

    #[test]
    fn gif_tag_is_extracted_and_stripped() {
        let context = "Rumm: [GIF: animated | image/gif | https://tenor.com/x.gif]";
        let items = extract_media_urls_from_context(context, 3);
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
}
