use anyhow::Result;
use serenity::model::channel::Message;
use serenity::prelude::*;
use tracing::{error, info};
use crate::db_utils;
use crate::gemini_api::GeminiClient;
use std::sync::Arc;
use tokio_rusqlite::Connection;
use serenity::model::id::ChannelId;
use serenity::http::Http;

// Handle fact interjection with Message object
pub async fn handle_fact_interjection(
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
                error!("Error retrieving recent messages for fact interjection: {:?}", e);
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    
    // Call the common implementation
    handle_fact_interjection_common(
        &ctx.http,
        msg.channel_id,
        gemini_client,
        &context_messages,
        bot_name,
    ).await
}

// Handle fact interjection for spontaneous interjections (without Message object)
pub async fn handle_spontaneous_fact_interjection(
    http: &Http,
    channel_id: ChannelId,
    gemini_client: &GeminiClient,
    message_db: &Option<Arc<tokio::sync::Mutex<Connection>>>,
    bot_name: &str,
    gemini_context_messages: usize,
) -> Result<()> {
    // Get recent messages for context
    let context_messages = if let Some(db) = message_db {
        match db_utils::get_recent_messages(db.clone(), gemini_context_messages, Some(&channel_id.to_string())).await {
            Ok(messages) => messages,
            Err(e) => {
                error!("Error retrieving recent messages for spontaneous fact interjection: {:?}", e);
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    
    // Call the common implementation
    handle_fact_interjection_common(
        http,
        channel_id,
        gemini_client,
        &context_messages,
        bot_name,
    ).await
}

// Common implementation for both regular and spontaneous fact interjections
async fn handle_fact_interjection_common(
    http: &Http,
    channel_id: ChannelId,
    gemini_client: &GeminiClient,
    context_messages: &Vec<(String, String, String)>,
    bot_name: &str,
) -> Result<()> {
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
        info!("No context available for fact interjection in channel_id: {}", channel_id);
        // Use empty string instead of "No recent messages" to avoid showing this in logs
        "".to_string()
    };
    
    // Create the fact prompt
    let fact_prompt = String::from(r#"You are {bot_name}, a Discord bot. Share an interesting and factually accurate fact related to the conversation.

{context}

Guidelines:
1. Share a single, concise, factually accurate fact that is relevant to the recent conversation
2. The fact MUST be true and verifiable - this is extremely important
3. Start with "Fun fact:" or "Did you know?"
4. Keep it brief (1-2 sentences)
5. Make it interesting and educational
6. If possible, relate it to the conversation topic, but don't force it
7. If you can't find a relevant fact based on the conversation, share a general interesting fact about technology, science, history, or nature
8. If you can't think of a relevant fact, respond with "pass"

Example good response: "Fun fact: The average cloud weighs around 1.1 million pounds due to the weight of water droplets."

Example bad response: "I noticed you were talking about weather. Here's an interesting fact: clouds are actually quite heavy! The average cloud weighs around 1.1 million pounds due to the weight of water droplets. Isn't that fascinating?"

Be concise and factual."#)
        .replace("{bot_name}", bot_name)
        .replace("{context}", &context_text);
    
    // Call Gemini API with the fact prompt
    match gemini_client.generate_response_with_context(&fact_prompt, "", &Vec::new(), None).await {
        Ok(response) => {
            // Check if the response is "pass" - if so, don't send anything
            if response.trim().to_lowercase() == "pass" {
                info!("Fact interjection evaluation: decided to PASS - no response sent");
                return Ok(());
            }
            
            // Check if the response looks like the prompt itself (API error)
            if response.contains("{bot_name}") || 
               response.contains("{context}") || 
               response.contains("Guidelines:") ||
               response.contains("Example good response:") {
                error!("Fact interjection error: API returned the prompt instead of a response");
                return Ok(());
            }
            
            // Start typing indicator
            if let Err(e) = channel_id.broadcast_typing(http).await {
                error!("Failed to send typing indicator for fact interjection: {:?}", e);
            }
            
            // Apply realistic typing delay based on response length
            let words = response.split_whitespace().count();
            let delay_secs = (words as f32 * 0.2).max(2.0).min(5.0) as u64;
            tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
            
            // Send the response
            let response_text = response.clone(); // Clone for logging
            if let Err(e) = channel_id.say(http, response).await {
                error!("Error sending fact interjection: {:?}", e);
            } else {
                info!("Fact interjection evaluation: SENT response - {}", response_text);
            }
        },
        Err(e) => {
            error!("Error generating fact interjection: {:?}", e);
        }
    }
    
    Ok(())
}
