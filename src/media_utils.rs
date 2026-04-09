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

/// Describe attachments as text tags for context storage
pub fn describe_attachments(msg: &Message) -> String {
    let mut tags = Vec::new();
    for attachment in &msg.attachments {
        let content_type = attachment.content_type.as_deref().unwrap_or("unknown");
        if IMAGE_TYPES.iter().any(|t| content_type.starts_with(t)) {
            tags.push(format!("[Image: {}]", attachment.filename));
        } else if VIDEO_TYPES.iter().any(|t| content_type.starts_with(t)) {
            tags.push(format!("[Video: {}]", attachment.filename));
        } else {
            tags.push(format!("[File: {}]", attachment.filename));
        }
    }
    tags.join(" ")
}

/// Download a URL and return base64-encoded content
async fn download_and_encode(url: &str) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let response = client.get(url).send().await?;
    let bytes = response.bytes().await?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
}
