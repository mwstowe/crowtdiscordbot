use crate::rate_limiter::RateLimiter;
use anyhow::Result;
use base64::Engine;
use serenity::all::{Channel, CreateMessage};
use serenity::builder::CreateAttachment;
use serenity::model::channel::Message;
use serenity::prelude::*;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Guard that cancels the typing indicator when dropped, ensuring it's always
/// stopped regardless of how the function exits (early return, error propagation, etc.)
struct TypingGuard {
    token: CancellationToken,
}

impl Drop for TypingGuard {
    fn drop(&mut self) {
        self.token.cancel();
    }
}

pub async fn handle_imagine_command(
    ctx: &Context,
    msg: &Message,
    prompt: &str,
    imagine_channels: &[String],
    pollinations_api_key: Option<&str>,
    together_api_key: Option<&str>,
    cloudflare_account_id: Option<&str>,
    cloudflare_api_token: Option<&str>,
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

    // Note: Even without API keys, the free Pollinations endpoint (image.pollinations.ai)
    // is available. But if we have neither key, warn that quality may be limited.
    if pollinations_api_key.is_none() && together_api_key.is_none() {
        info!(
            "No paid image generation API keys configured, using free Pollinations endpoint only"
        );
    }

    // Start typing indicator and keep refreshing it until generation completes.
    // The TypingGuard ensures the indicator is always cancelled when it goes out of scope,
    // regardless of how the function exits (early return, error propagation, etc.)
    let typing_channel_id = msg.channel_id;
    let typing_http = ctx.http.clone();
    let typing_cancel = CancellationToken::new();
    let typing_cancel_clone = typing_cancel.clone();
    let _typing_guard = TypingGuard {
        token: typing_cancel,
    };
    tokio::spawn(async move {
        loop {
            if let Err(e) = typing_channel_id.broadcast_typing(&typing_http).await {
                error!(
                    "Failed to send typing indicator for image generation: {:?}",
                    e
                );
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(8)) => {}
                _ = typing_cancel_clone.cancelled() => break,
            }
        }
    });

    info!("Generating image for prompt: {}", prompt);

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

    // Try Pollinations first (paid then free), then Cloudflare Workers AI, then Together.ai
    let image_bytes = if let Some(key) = pollinations_api_key {
        // Paid Pollinations → Cloudflare → free Pollinations → Together.ai
        let pollinations_result = try_pollinations(http_client, truncated_prompt, key).await;
        match pollinations_result {
            Some(bytes) => Some(bytes),
            None => {
                // Try Cloudflare Workers AI (free tier: ~173 images/day, FLUX.1 schnell)
                if let Some(bytes) = try_cloudflare(
                    http_client,
                    truncated_prompt,
                    cloudflare_account_id,
                    cloudflare_api_token,
                )
                .await
                {
                    Some(bytes)
                } else if let Some(bytes) =
                    try_pollinations_free(http_client, truncated_prompt).await
                {
                    Some(bytes)
                } else if let Some(key) = together_api_key {
                    info!("All free providers failed, falling back to Together.ai");
                    try_together(http_client, truncated_prompt, key).await
                } else {
                    None
                }
            }
        }
    } else {
        // No paid Pollinations key — try Cloudflare first, then free Pollinations, then Together.ai
        if let Some(bytes) = try_cloudflare(
            http_client,
            truncated_prompt,
            cloudflare_account_id,
            cloudflare_api_token,
        )
        .await
        {
            Some(bytes)
        } else if let Some(bytes) = try_pollinations_free(http_client, truncated_prompt).await {
            Some(bytes)
        } else if let Some(key) = together_api_key {
            info!("All free providers failed, falling back to Together.ai");
            try_together(http_client, truncated_prompt, key).await
        } else {
            None
        }
    };

    match image_bytes {
        Some(bytes) => {
            let attachment = CreateAttachment::bytes(bytes, "imagine.png");
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

/// Try generating an image via Pollinations API.
/// First tries the paid gen.pollinations.ai endpoint (if API key is provided),
/// then falls back to the free image.pollinations.ai endpoint (no key required).
/// Returns Some(bytes) on success, None if all models fail.
async fn try_pollinations(
    http_client: &reqwest::Client,
    prompt: &str,
    api_key: &str,
) -> Option<Vec<u8>> {
    let encoded_prompt = urlencoding::encode(prompt);
    let timeout = Duration::from_secs(90);

    // First try the paid gen.pollinations.ai endpoint with API key
    let paid_models = ["gptimage", "zimage", "flux"];
    let mut all_paid_402 = true;

    for model in paid_models {
        let url = format!(
            "https://gen.pollinations.ai/image/{encoded_prompt}?model={model}&width=1024&height=1024&nologo=true"
        );
        let resp = http_client
            .get(&url)
            .header("Authorization", format!("Bearer {api_key}"))
            .timeout(timeout)
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                info!(
                    "Image generated successfully via Pollinations (paid) with model: {}",
                    model
                );
                match r.bytes().await {
                    Ok(bytes) => return Some(bytes.to_vec()),
                    Err(e) => {
                        error!("Failed to read Pollinations response bytes: {:?}", e);
                        continue;
                    }
                }
            }
            Ok(r) if r.status().as_u16() == 402 => {
                info!(
                    "Pollinations model {} returned 402 (payment required), trying next model",
                    model
                );
                continue;
            }
            Ok(r) if r.status().as_u16() == 422 => {
                all_paid_402 = false;
                info!(
                    "Pollinations model {} returned 422 (content policy or invalid request), trying next model",
                    model
                );
                continue;
            }
            Ok(r) => {
                all_paid_402 = false;
                error!(
                    "Pollinations API error with model {}: HTTP {}",
                    model,
                    r.status()
                );
                continue;
            }
            Err(e) => {
                all_paid_402 = false;
                error!(
                    "Pollinations API request failed with model {}: {:?}",
                    model, e
                );
                continue;
            }
        }
    }

    // If all paid models returned 402, try the free image.pollinations.ai endpoint
    // This endpoint requires no API key and uses the flux model
    if all_paid_402 {
        info!(
            "All paid Pollinations models returned 402, trying free image.pollinations.ai endpoint"
        );
        if let Some(bytes) = try_pollinations_free(http_client, prompt).await {
            return Some(bytes);
        }
    }

    warn!("All Pollinations models failed");
    None
}

/// Try generating an image via the free Pollinations endpoint (image.pollinations.ai).
/// This endpoint requires no API key but may be slower or have lower priority.
/// Returns Some(bytes) on success, None on failure.
async fn try_pollinations_free(http_client: &reqwest::Client, prompt: &str) -> Option<Vec<u8>> {
    let encoded_prompt = urlencoding::encode(prompt);
    let timeout = Duration::from_secs(120); // Free tier can be slower

    let url = format!(
        "https://image.pollinations.ai/prompt/{encoded_prompt}?width=1024&height=1024&nologo=true&model=flux"
    );

    info!("Trying free Pollinations endpoint (image.pollinations.ai)");

    let resp = http_client.get(&url).timeout(timeout).send().await;

    match resp {
        Ok(r) if r.status().is_success() => {
            match r.bytes().await {
                Ok(bytes) => {
                    // Verify we got actual image data (not an error page)
                    if bytes.len() > 1000 {
                        info!(
                            "Image generated successfully via free Pollinations endpoint ({} bytes)",
                            bytes.len()
                        );
                        return Some(bytes.to_vec());
                    } else {
                        warn!(
                            "Free Pollinations returned suspiciously small response ({} bytes), skipping",
                            bytes.len()
                        );
                    }
                }
                Err(e) => {
                    error!("Failed to read free Pollinations response bytes: {:?}", e);
                }
            }
        }
        Ok(r) => {
            error!("Free Pollinations endpoint returned HTTP {}", r.status());
        }
        Err(e) => {
            error!("Free Pollinations endpoint request failed: {:?}", e);
        }
    }

    None
}

/// Try generating an image via Cloudflare Workers AI (free tier: ~173 images/day).
/// Uses the flux-1-schnell model. Requires a free Cloudflare account with Account ID and API token.
/// Returns Some(bytes) on success, None on failure or if not configured.
async fn try_cloudflare(
    http_client: &reqwest::Client,
    prompt: &str,
    account_id: Option<&str>,
    api_token: Option<&str>,
) -> Option<Vec<u8>> {
    let (account_id, api_token) = match (account_id, api_token) {
        (Some(id), Some(token)) if !id.is_empty() && !token.is_empty() => (id, token),
        _ => return None, // Not configured, skip silently
    };

    let timeout = Duration::from_secs(60);
    let model = "@cf/black-forest-labs/flux-1-schnell";
    let url = format!("https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/run/{model}");

    info!("Trying Cloudflare Workers AI (flux-1-schnell)");

    let body = serde_json::json!({
        "prompt": prompt,
        "steps": 4
    });

    let resp = http_client
        .post(&url)
        .header("Authorization", format!("Bearer {api_token}"))
        .header("Content-Type", "application/json")
        .timeout(timeout)
        .json(&body)
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => match r.json::<serde_json::Value>().await {
            Ok(json) => {
                if let Some(b64_image) = json["result"]["image"].as_str() {
                    match base64::engine::general_purpose::STANDARD.decode(b64_image) {
                        Ok(bytes) => {
                            info!(
                                "Image generated successfully via Cloudflare Workers AI ({} bytes)",
                                bytes.len()
                            );
                            return Some(bytes);
                        }
                        Err(e) => {
                            error!("Failed to decode Cloudflare base64 image: {:?}", e);
                        }
                    }
                } else {
                    error!("Cloudflare response missing image data: {:?}", json);
                }
            }
            Err(e) => {
                error!("Failed to parse Cloudflare response: {:?}", e);
            }
        },
        Ok(r) if r.status().as_u16() == 429 => {
            warn!("Cloudflare Workers AI daily limit reached (429)");
        }
        Ok(r) => {
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            error!("Cloudflare Workers AI error: HTTP {} - {}", status, body);
        }
        Err(e) => {
            error!("Cloudflare Workers AI request failed: {:?}", e);
        }
    }

    None
}

/// Try generating an image via Together.ai API.
/// Uses cheap serverless models as a last-resort fallback.
/// Returns Some(bytes) on success, None on failure.
async fn try_together(
    http_client: &reqwest::Client,
    prompt: &str,
    api_key: &str,
) -> Option<Vec<u8>> {
    let timeout = Duration::from_secs(60);

    // Cheap serverless image models that are confirmed working (Aug 2026)
    // Juggernaut Lightning: $0.0017/MP, SDXL: $0.0019/MP
    let models = [
        "Rundiffusion/Juggernaut-Lightning-Flux",
        "stabilityai/stable-diffusion-xl-base-1.0",
    ];

    for model in models {
        let body = serde_json::json!({
            "model": model,
            "prompt": prompt,
            "width": 1024,
            "height": 1024,
            "steps": 4,
            "n": 1,
            "response_format": "b64_json"
        });

        let resp = http_client
            .post("https://api.together.xyz/v1/images/generations")
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json")
            .timeout(timeout)
            .json(&body)
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                match r.json::<serde_json::Value>().await {
                    Ok(json) => {
                        if let Some(b64_data) = json["data"][0]["b64_json"].as_str() {
                            match base64::engine::general_purpose::STANDARD.decode(b64_data) {
                                Ok(bytes) => {
                                    info!(
                                        "Image generated successfully via Together.ai with model: {}",
                                        model
                                    );
                                    return Some(bytes);
                                }
                                Err(e) => {
                                    error!("Failed to decode Together.ai base64 image: {:?}", e);
                                }
                            }
                        } else if let Some(url) = json["data"][0]["url"].as_str() {
                            // Fallback: download from URL if b64_json not present
                            match http_client
                                .get(url)
                                .header("User-Agent", "crow-bot/1.0")
                                .timeout(Duration::from_secs(30))
                                .send()
                                .await
                            {
                                Ok(img_resp) if img_resp.status().is_success() => {
                                    match img_resp.bytes().await {
                                        Ok(bytes) => {
                                            info!(
                                                "Image generated successfully via Together.ai (URL download) with model: {}",
                                                model
                                            );
                                            return Some(bytes.to_vec());
                                        }
                                        Err(e) => {
                                            error!("Failed to download Together.ai image: {:?}", e);
                                        }
                                    }
                                }
                                Ok(img_resp) => {
                                    error!(
                                        "Failed to download Together.ai image: HTTP {}",
                                        img_resp.status()
                                    );
                                }
                                Err(e) => {
                                    error!("Failed to download Together.ai image: {:?}", e);
                                }
                            }
                        } else {
                            error!("Together.ai response missing image data: {:?}", json);
                        }
                    }
                    Err(e) => {
                        error!("Failed to parse Together.ai response: {:?}", e);
                    }
                }
            }
            Ok(r) if r.status().as_u16() == 402 || r.status().as_u16() == 429 => {
                warn!(
                    "Together.ai model {} returned {} (rate limited or payment required), trying next model",
                    model,
                    r.status()
                );
                continue;
            }
            Ok(r) => {
                let status = r.status();
                let body = r.text().await.unwrap_or_default();
                error!(
                    "Together.ai API error with model {}: HTTP {} - {}",
                    model, status, body
                );
                continue;
            }
            Err(e) => {
                error!(
                    "Together.ai API request failed with model {}: {:?}",
                    model, e
                );
                continue;
            }
        }
    }

    warn!("All Together.ai models failed");
    None
}
