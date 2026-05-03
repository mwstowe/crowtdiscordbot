use crate::rate_limiter::RateLimiter;
use anyhow::Result;
use serenity::all::{Channel, CreateMessage};
use serenity::builder::CreateAttachment;
use serenity::model::channel::Message;
use serenity::prelude::*;
use std::time::Duration;
use tracing::{error, info};

pub async fn handle_imagine_command(
    ctx: &Context,
    msg: &Message,
    prompt: &str,
    imagine_channels: &[String],
    pollinations_api_key: Option<&str>,
    rate_limiter: &RateLimiter,
    http_client: &reqwest::Client,
) -> Result<()> {
    // Check if the command is being used in an allowed channel
    let channel_name = match msg.channel_id.to_channel(&ctx.http).await {
        Ok(channel) => match channel {
            Channel::Guild(guild_channel) => guild_channel.name,
            _ => String::new(),
        },
        Err(_) => String::new(),
    };

    if !imagine_channels.is_empty() && !imagine_channels.contains(&channel_name) {
        let channel_list = if imagine_channels.len() == 1 {
            format!("the #{} channel", imagine_channels[0])
        } else {
            let channels = imagine_channels
                .iter()
                .map(|c| format!("#{c}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("one of these channels: {channels}")
        };
        msg.reply(
            &ctx.http,
            format!("Image generation is only available in {channel_list}. Please try your command there."),
        )
        .await?;
        return Ok(());
    }

    // Start typing indicator
    if let Err(e) = msg.channel_id.broadcast_typing(&ctx.http).await {
        error!(
            "Failed to send typing indicator for image generation: {:?}",
            e
        );
    }

    info!("Generating image via Pollinations for prompt: {}", prompt);

    // Check rate limits before making the request
    if let Err(e) = rate_limiter.acquire().await {
        error!("Image generation rate limited: {:?}", e);
        msg.reply(
            &ctx.http,
            "Image generation is currently rate limited. Please try again in a moment.",
        )
        .await?;
        return Ok(());
    }

    // Truncate very long prompts — image models don't benefit from extremely detailed text
    // and long URL-encoded prompts can cause timeouts
    let truncated_prompt = if prompt.len() > 500 {
        info!("Truncating image prompt from {} to 500 chars", prompt.len());
        &prompt[..prompt.rfind(' ').unwrap_or(500).min(500)]
    } else {
        prompt
    };

    let encoded_prompt = urlencoding::encode(truncated_prompt);
    let timeout = Duration::from_secs(90);

    let image_bytes = if let Some(key) = pollinations_api_key {
        // Try models in order of quality, falling back on 402 (payment required)
        let models = ["zimage", "flux"];
        let mut result = None;
        let mut all_402 = true;

        for model in models {
            let url = format!(
                "https://gen.pollinations.ai/image/{encoded_prompt}?model={model}&width=1024&height=1024&nologo=true"
            );
            let resp = http_client
                .get(&url)
                .header("Authorization", format!("Bearer {key}"))
                .timeout(timeout)
                .send()
                .await;

            match resp {
                Ok(r) if r.status().is_success() => {
                    info!("Image generated successfully with model: {}", model);
                    all_402 = false;
                    result = Some(r.bytes().await?);
                    break;
                }
                Ok(r) if r.status().as_u16() == 402 => {
                    info!("Model {} returned 402, trying next model", model);
                    continue;
                }
                Ok(r) => {
                    error!("Pollinations API error with model {}: HTTP {}", model, r.status());
                    all_402 = false;
                    break;
                }
                Err(e) => {
                    error!("Pollinations API request failed: {:?}", e);
                    all_402 = false;
                    break;
                }
            }
        }

        // If all models returned 402, fall back to legacy endpoint (no auth, no pollen cost)
        if result.is_none() && all_402 {
            info!("All models returned 402, falling back to legacy endpoint");
            let url = format!(
                "https://image.pollinations.ai/prompt/{encoded_prompt}?width=1024&height=1024&nologo=true"
            );
            for attempt in 1..=3 {
                match http_client.get(&url).timeout(timeout).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        result = Some(resp.bytes().await?);
                        break;
                    }
                    Ok(resp) if resp.status().as_u16() == 429 => {
                        info!("Legacy endpoint returned 429, retry {}/3 after {}s", attempt, attempt * 10);
                        tokio::time::sleep(Duration::from_secs(attempt as u64 * 10)).await;
                        let _ = msg.channel_id.broadcast_typing(&ctx.http).await;
                        continue;
                    }
                    Ok(resp) => {
                        error!("Legacy endpoint failed: HTTP {}", resp.status());
                        break;
                    }
                    Err(_) if attempt < 3 => {
                        info!("Legacy endpoint timed out, retry {}/3 after {}s", attempt, attempt * 10);
                        tokio::time::sleep(Duration::from_secs(attempt as u64 * 10)).await;
                        let _ = msg.channel_id.broadcast_typing(&ctx.http).await;
                        continue;
                    }
                    Err(e) => {
                        error!("Legacy endpoint request failed after retries: {:?}", e);
                        break;
                    }
                }
            }
        }

        result
    } else {
        info!("No Pollinations API key configured, using legacy endpoint");
        let url = format!(
            "https://image.pollinations.ai/prompt/{encoded_prompt}?width=1024&height=1024&nologo=true"
        );
        match http_client.get(&url).timeout(timeout).send().await {
            Ok(resp) if resp.status().is_success() => Some(resp.bytes().await?),
            Ok(resp) => {
                error!("Pollinations legacy API error: HTTP {}", resp.status());
                None
            }
            Err(e) => {
                error!("Pollinations legacy API request failed: {:?}", e);
                None
            }
        }
    };

    match image_bytes {
        Some(bytes) => {
            let attachment = CreateAttachment::bytes(bytes, "imagine.jpg");
            let message_content = format!("Here's what I imagine for: {prompt}");
            let builder = CreateMessage::default()
                .content(message_content)
                .add_file(attachment);

            if let Err(e) = msg.channel_id.send_message(&ctx.http, builder).await {
                error!("Failed to send generated image: {:?}", e);
                msg.reply(&ctx.http, "Sorry, I couldn't send the generated image.")
                    .await?;
            }
        }
        None => {
            msg.reply(
                &ctx.http,
                "Sorry, I couldn't generate that image. Please try again.",
            )
            .await?;
        }
    }

    Ok(())
}
