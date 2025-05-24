use anyhow::Result;
use reqwest;
use serde_json;
use std::time::Duration;
use tracing::{error, info};
use crate::rate_limiter::RateLimiter;
use crate::display_name::clean_display_name;
use base64::Engine;

pub struct GeminiClient {
    api_key: String,
    api_endpoint: String,
    image_endpoint: String,
    prompt_wrapper: String,
    bot_name: String,
    rate_limiter: RateLimiter,
    context_messages: usize,
}

impl GeminiClient {
    pub fn new(
        api_key: String, 
        api_endpoint: Option<String>,
        prompt_wrapper: Option<String>,
        bot_name: String,
        rate_limit_minute: u32,
        rate_limit_day: u32,
        context_messages: usize
    ) -> Self {
        // Default endpoint for Gemini API
        let default_endpoint = "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent".to_string();
        let image_endpoint = "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash-preview-image-generation:generateContent".to_string();
        
        // Default prompt wrapper
        let default_prompt_wrapper = "You are {bot_name}, a helpful Discord bot. You are responding to {user}. Be concise, helpful, and friendly. Here is their message: {message}\n\nRecent conversation context:\n{context}".to_string();
        
        // Use provided values or defaults
        let api_endpoint = api_endpoint.unwrap_or(default_endpoint);
        let prompt_wrapper = prompt_wrapper.unwrap_or(default_prompt_wrapper);
        
        // Create rate limiter
        let rate_limiter = RateLimiter::new(rate_limit_minute, rate_limit_day);
        
        Self {
            api_key,
            api_endpoint,
            image_endpoint,
            prompt_wrapper,
            bot_name,
            rate_limiter,
            context_messages,
        }
    }
    
    // Generate a response using the Gemini API
    pub async fn generate_response(&self, prompt: &str, user_name: &str) -> Result<String> {
        // Format the prompt using the wrapper
        let formatted_prompt = self.prompt_wrapper
            .replace("{message}", prompt)
            .replace("{bot_name}", &self.bot_name)
            .replace("{user}", user_name)
            .replace("{context}", "No context available.");
            
        self.generate_content(&formatted_prompt).await
    }
    
    // Generate a response with conversation context
    pub async fn generate_response_with_context(
        &self, 
        prompt: &str, 
        user_name: &str,
        context_messages: &Vec<(String, String, String)>,
        _user_pronouns: Option<&str>
    ) -> Result<String> {
        // Format the context messages - limit to configured number and reverse to get chronological order
        let context = if !context_messages.is_empty() {
            // Take only the configured number of messages
            let limited_messages = if context_messages.len() > self.context_messages {
                &context_messages[0..self.context_messages]
            } else {
                context_messages
            };
            
            // Format each message as "User: Message"
            limited_messages.iter()
                .map(|(user, _, msg)| format!("{}: {}", clean_display_name(user), msg))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            "No context available.".to_string()
        };
        
        // Format the prompt using the wrapper
        let formatted_prompt = self.prompt_wrapper
            .replace("{message}", prompt)
            .replace("{bot_name}", &self.bot_name)
            .replace("{user}", user_name)
            .replace("{context}", &context);
            
        self.generate_content(&formatted_prompt).await
    }
    
    // Generate content with a raw prompt
    pub async fn generate_content(&self, prompt: &str) -> Result<String> {
        // Use acquire() which includes retry logic and request recording
        self.rate_limiter.acquire().await?;
        
        // Prepare the request body
        let request_body = serde_json::json!({
            "contents": [{
                "parts": [{
                    "text": prompt
                }]
            }]
        });
        
        // Build the URL with API key
        let url = format!("{}?key={}", self.api_endpoint, self.api_key);
        
        // Make the API call
        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .json(&request_body)
            .timeout(Duration::from_secs(30))
            .send()
            .await?;
            
        // Parse the response
        let response_json: serde_json::Value = response.json().await?;
        
        // Extract the generated text
        if let Some(text) = response_json
            .get("candidates")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.get(0))
            .and_then(|p| p.get("text"))
            .and_then(|t| t.as_str()) {
            info!("Successfully generated content from Gemini API");
            Ok(text.to_string())
        } else {
            error!("Failed to extract text from Gemini API response");
            Err(anyhow::anyhow!("Failed to extract text from Gemini API response"))
        }
    }

    // Generate an image from a text prompt
    pub async fn generate_image(&self, prompt: &str) -> Result<Vec<u8>> {
        // Use acquire() which includes retry logic and request recording
        self.rate_limiter.acquire().await?;
        
        // Prepare the request body for the gemini-2.0-flash-preview-image-generation model
        // Include the required response modalities (IMAGE, TEXT)
        let request_body = serde_json::json!({
            "contents": [{
                "parts": [{
                    "text": prompt
                }]
            }],
            "generation_config": {
                "response_mime_type": ["image/png", "text/plain"]
            }
        });
        
        // Build the URL with API key
        let url = format!("{}?key={}", self.image_endpoint, self.api_key);
        
        // Make the API call
        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .json(&request_body)
            .timeout(Duration::from_secs(60))  // Longer timeout for image generation
            .send()
            .await?;
            
        // Parse the response
        let response_json: serde_json::Value = response.json().await?;
        
        // Log the full response for debugging
        info!("Image generation API response: {}", serde_json::to_string_pretty(&response_json)?);
        
        // Extract the generated image data
        if let Some(image_data) = response_json
            .get("candidates")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.get(0))
            .and_then(|p| p.get("image"))
            .and_then(|i| i.get("data"))
            .and_then(|d| d.as_str()) {
            info!("Successfully generated image from Gemini API");
            
            // Decode base64 image data
            match base64::engine::general_purpose::STANDARD.decode(image_data) {
                Ok(bytes) => Ok(bytes),
                Err(e) => {
                    error!("Failed to decode base64 image data: {:?}", e);
                    Err(anyhow::anyhow!("Failed to decode base64 image data"))
                }
            }
        } else {
            error!("Failed to extract image data from API response");
            Err(anyhow::anyhow!("Failed to extract image data from API response"))
        }
    }
}
