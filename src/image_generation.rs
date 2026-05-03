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

    let encoded_prompt = urlencoding::encode(prompt);
    let url = format!(
        "https://image.pollinations.ai/prompt/{encoded_prompt}?width=1024&height=1024&nologo=true"
    );

    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .timeout(Duration::from_secs(60))
        .send()
        .await;

    match response {
        Ok(resp) if resp.status().is_success() => {
            let image_bytes = resp.bytes().await?;

            let temp_dir = std::env::temp_dir();
            let file_path = temp_dir.join(format!(
                "pollinations_image_{}.png",
                chrono::Utc::now().timestamp()
            ));
            std::fs::write(&file_path, &image_bytes)?;

            let files = vec![CreateAttachment::path(&file_path).await?];
            let message_content = format!("Here's what I imagine for: {prompt}");
            let builder = files
                .into_iter()
                .fold(CreateMessage::default().content(message_content), |b, f| {
                    b.add_file(f)
                });

            if let Err(e) = msg.channel_id.send_message(&ctx.http, builder).await {
                error!("Failed to send generated image: {:?}", e);
                msg.reply(&ctx.http, "Sorry, I couldn't send the generated image.")
                    .await?;
            }

            if let Err(e) = std::fs::remove_file(file_path) {
                error!("Failed to clean up temporary image file: {:?}", e);
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
