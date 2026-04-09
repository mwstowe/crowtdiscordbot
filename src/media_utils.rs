use anyhow::Result;
use base64::Engine;
use regex::Regex;
use serenity::model::channel::Message;
use tracing::{error, info};

/// A media item extracted from a Discord message
#[derive(Debug, Clone)]
pub struct MediaItem {
    pub mime_type: String,
    pub data: String, // base64-encoded
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
pub async fn extract_media_from_message(msg: &Message) -> Vec<MediaItem> {
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

        match download_and_encode(&attachment.url).await {
            Ok(data) => {
                items.push(MediaItem {
                    mime_type: content_type.to_string(),
                    data,
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

            match download_and_encode(&attachment.url).await {
                Ok(data) => {
                    items.push(MediaItem {
                        mime_type: content_type.to_string(),
                        data,
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
    let re = Regex::new(
        r"(?:https?://)?(?:www\.)?(?:youtube\.com/watch\?v=|youtu\.be/|youtube\.com/shorts/)[\w\-]+"
    ).unwrap();

    re.find_iter(text)
        .map(|m| YouTubeUrl {
            url: m.as_str().to_string(),
        })
        .collect()
}

/// Describe attachments as text tags for context storage (includes URL for later retrieval)
pub fn describe_attachments(msg: &Message) -> String {
    let mut tags = Vec::new();
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
    tags.join(" ")
}

/// Extract image/video URLs from context text, returning media metadata.
/// Returns up to `max_items` most recent items (from end of text).
pub fn extract_media_urls_from_context(text: &str, max_items: usize) -> Vec<(String, String)> {
    // Match [Image: name | mime | url] and [Video: name | mime | url]
    let re = Regex::new(r"\[(Image|Video): [^|]+ \| ([^|]+) \| (https?://[^\]]+)\]").unwrap();
    let mut items: Vec<(String, String)> = re
        .captures_iter(text)
        .map(|cap| {
            let mime = cap[2].trim().to_string();
            let url = cap[3].trim().to_string();
            (mime, url)
        })
        .collect();
    // Keep only the most recent items
    if items.len() > max_items {
        items = items.split_off(items.len() - max_items);
    }
    items
}

/// Strip media URLs from context text for display (keep just filename)
pub fn strip_media_urls_from_context(text: &str) -> String {
    let re = Regex::new(r"\[(Image|Video): ([^|]+) \| [^|]+ \| https?://[^\]]+\]").unwrap();
    re.replace_all(text, |caps: &regex::Captures| {
        let kind = &caps[1];
        let name = caps[2].trim();
        format!("[{kind}: {name}]")
    })
    .to_string()
}

/// Download media items from URLs found in context text.
/// Returns up to max_items MediaItems, silently skipping failures.
pub async fn fetch_media_from_context(text: &str, max_items: usize) -> Vec<MediaItem> {
    let urls = extract_media_urls_from_context(text, max_items);
    let mut items = Vec::new();
    for (mime, url) in urls {
        match download_and_encode(&url).await {
            Ok(data) => {
                info!("Fetched context media: {} ({})", url, mime);
                items.push(MediaItem {
                    mime_type: mime,
                    data,
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
async fn download_and_encode(url: &str) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        return Err(anyhow::anyhow!("HTTP {}", response.status()));
    }
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_INLINE_SIZE {
        return Err(anyhow::anyhow!("Too large: {} bytes", bytes.len()));
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
}
