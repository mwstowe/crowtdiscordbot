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

    let encoded_prompt = urlencoding::encode(prompt);
    let (url, has_auth) = if let Some(key) = pollinations_api_key {
        (
            format!("https://gen.pollinations.ai/image/{encoded_prompt}?model=zimage&width=1024&height=1024&nologo=true"),
            Some(key),
        )
    } else {
        info!("No Pollinations API key configured, using legacy endpoint");
        (
            format!("https://image.pollinations.ai/prompt/{encoded_prompt}?width=1024&height=1024&nologo=true"),
            None,
        )
    };

    let mut request = http_client
        .get(&url)
        .timeout(Duration::from_secs(60));

    if let Some(key) = has_auth {
        request = request.header("Authorization", format!("Bearer {key}"));
    }

    let response = request.send().await;

    match response {
        Ok(resp) if resp.status().is_success() => {
            let image_bytes = resp.bytes().await?;

            let attachment = CreateAttachment::bytes(image_bytes, "imagine.jpg");
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
        Ok(resp) => {
            error!("Pollinations API error: HTTP {}", resp.status());
            msg.reply(
                &ctx.http,
                "Sorry, I couldn't generate that image. Please try again.",
            )
            .await?;
        }
        Err(e) => {
            error!("Pollinations API request failed: {:?}", e);
            msg.reply(
                &ctx.http,
                "Sorry, image generation timed out. Please try again.",
            )
            .await?;
        }
    }

    Ok(())
}
