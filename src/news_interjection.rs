use anyhow::Result;
use serenity::model::channel::Message;
use serenity::prelude::*;
use tracing::{error, info};
use crate::db_utils;
use crate::response_timing::apply_realistic_delay;
use crate::gemini_api::GeminiClient;
use std::sync::Arc;
use tokio_rusqlite::Connection;

// Handle news interjection
pub async fn handle_news_interjection(
    ctx: &Context,
    msg: &Message,
    gemini_client: &GeminiClient,
    message_db: &Option<Arc<tokio::sync::Mutex<Connection>>>,
    bot_name: &str,
    gemini_context_messages: usize,
) -> Result<()> {
    // Get recent messages for context
    let context_messages = if let Some(db) = message_db {
        match db_utils::get_recent_messages(db.clone(), gemini_context_messages, Some(msg.channel_id.to_string().as_str())).await {
            Ok(messages) => messages,
            Err(e) => {
                error!("Error retrieving recent messages for news interjection: {:?}", e);
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    
    // Format context for the prompt
    let context_text = if !context_messages.is_empty() {
        // Reverse the messages to get chronological order (oldest first)
        let mut chronological_messages = context_messages.clone();
        chronological_messages.reverse();
        
        let formatted_messages: Vec<String> = chronological_messages.iter()
            .map(|(_author, display_name, content)| format!("{}: {}", display_name, content))
            .collect();
        formatted_messages.join("\n")
    } else {
        info!("No context available for news interjection in channel_id: {}", msg.channel_id);
        // Use empty string instead of "No recent messages" to avoid showing this in logs
        "".to_string()
    };
    
    // Create the news prompt
    let news_prompt = String::from(r#"You are {bot_name}, a Discord bot. Share an interesting technology or weird news article link with a brief comment about why it's interesting.

{context}

Guidelines:
1. Create a fictional but plausible news article link about technology or weird news (NO sports)
2. Format as: "Article title: https://example.com/article-path"
3. Then add a brief comment (1-2 sentences) on why it's interesting or relevant to the conversation
4. If possible, relate it to the conversation, but don't force it
5. Don't use phrases like "Check out this article" or "You might find this interesting"
6. If you can't think of a relevant article, respond with "pass"

Example good response: "AI Creates Perfect Pizza Recipe Through Taste Simulation: https://techcrunch.com/2025/06/ai-taste-simulation-pizza This shows how AI sensory processing is advancing beyond visual and audio into taste simulation."

Example bad response: "Check out this interesting article about AI and food: https://techcrunch.com/ai-food-article I thought you might find this interesting given our conversation about technology."

Be creative but realistic with your article title and URL."#)
        .replace("{bot_name}", bot_name)
        .replace("{context}", &context_text);
    
    // Call Gemini API with the news prompt
    match gemini_client.generate_response_with_context(&news_prompt, "", &Vec::new(), None).await {
        Ok(response) => {
            // Check if the response is "pass" - if so, don't send anything
            if response.trim().to_lowercase() == "pass" {
                info!("News interjection evaluation: decided to PASS - no response sent");
                return Ok(());
            }
            
            // Check if the response looks like the prompt itself (API error)
            if response.contains("{bot_name}") || 
               response.contains("{context}") || 
               response.contains("Guidelines:") ||
               response.contains("Example good response:") {
                error!("News interjection error: API returned the prompt instead of a response");
                return Ok(());
            }
            
            // Start typing indicator now that we've decided to send a message
            if let Err(e) = msg.channel_id.broadcast_typing(&ctx.http).await {
                error!("Failed to send typing indicator for news interjection: {:?}", e);
            }
            
            // Apply realistic typing delay
            apply_realistic_delay(&response, ctx, msg.channel_id).await;
            
            // Send the response
            let response_text = response.clone(); // Clone for logging
            if let Err(e) = msg.channel_id.say(&ctx.http, response).await {
                error!("Error sending news interjection: {:?}", e);
            } else {
                info!("News interjection evaluation: SENT response - {}", response_text);
            }
        },
        Err(e) => {
            error!("Error generating news interjection: {:?}", e);
        }
    }
    
    Ok(())
}
