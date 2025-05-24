use anyhow::Result;
use serenity::model::channel::Message;
use serenity::prelude::*;
use tracing::{error, info, warn};
use crate::gemini_api::GeminiClient;
use serenity::builder::CreateAttachment;
use serenity::all::CreateMessage;
use tokio::time::sleep;
use std::time::Duration;

pub async fn handle_imagine_command(ctx: &Context, msg: &Message, gemini_client: &GeminiClient, prompt: &str) -> Result<()> {
    // Start typing indicator
    if let Err(e) = msg.channel_id.broadcast_typing(&ctx.http).await {
        error!("Failed to send typing indicator for image generation: {:?}", e);
    }

    info!("Generating image for prompt: {}", prompt);

    // Try up to 3 times (initial attempt + 2 retries)
    let mut attempt = 0;
    let max_attempts = 3;
    let retry_delays = [15, 30]; // Seconds to wait before retries

    loop {
        attempt += 1;
        info!("Image generation attempt {} of {}", attempt, max_attempts);

        // Generate the image with a timeout
        match tokio::time::timeout(
            Duration::from_secs(60), // 60 second timeout
            gemini_client.generate_image(prompt)
        ).await {
            // Successful API call (may be success or error)
            Ok(api_result) => {
                match api_result {
                    // Successful image generation
                    Ok((image_data, description)) => {
                        // Create a temporary file for the image
                        let temp_dir = std::env::temp_dir();
                        let file_path = temp_dir.join(format!("gemini_image_{}.png", chrono::Utc::now().timestamp()));
                        
                        // Write the image data to the file
                        std::fs::write(&file_path, &image_data)?;

                        // Create the attachment
                        let files = vec![CreateAttachment::path(&file_path).await?];

                        // Format the message with both the prompt and the AI's description
                        let message_content = if description.is_empty() {
                            format!("Here's what I imagine for: {}", prompt)
                        } else {
                            format!("Here's what I imagine for: {}\n\n{}", prompt, description)
                        };

                        // Send the image file with the description
                        let builder = files.into_iter().fold(
                            CreateMessage::default().content(message_content),
                            |b, f| b.add_file(f)
                        );

                        // Send the message
                        if let Err(e) = msg.channel_id.send_message(&ctx.http, builder).await {
                            error!("Failed to send generated image: {:?}", e);
                            msg.reply(&ctx.http, "Sorry, I couldn't send the generated image.").await?;
                        }

                        // Clean up the temporary file
                        if let Err(e) = std::fs::remove_file(file_path) {
                            error!("Failed to clean up temporary image file: {:?}", e);
                        }
                        
                        // Success - break out of the retry loop
                        break;
                    },
                    // API error
                    Err(e) => {
                        error!("Failed to generate image (attempt {}/{}): {:?}", attempt, max_attempts, e);
                        
                        // If we've used all our attempts, notify the user
                        if attempt >= max_attempts {
                            msg.reply(&ctx.http, "Sorry, I couldn't generate that image after several attempts.").await?;
                            break;
                        }
                        
                        // Otherwise, wait and retry
                        let retry_delay = retry_delays[attempt - 1];
                        warn!("Retrying image generation in {} seconds...", retry_delay);
                        sleep(Duration::from_secs(retry_delay)).await;
                    }
                }
            },
            // Timeout error
            Err(_) => {
                error!("Image generation timed out (attempt {}/{})", attempt, max_attempts);
                
                // If we've used all our attempts, notify the user
                if attempt >= max_attempts {
                    msg.reply(&ctx.http, "Sorry, image generation timed out after several attempts.").await?;
                    break;
                }
                
                // Otherwise, wait and retry
                let retry_delay = retry_delays[attempt - 1];
                warn!("Retrying image generation in {} seconds after timeout...", retry_delay);
                sleep(Duration::from_secs(retry_delay)).await;
            }
        }
    }

    Ok(())
}
