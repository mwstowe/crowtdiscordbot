use anyhow::Result;
use reqwest;
use serde_json;
use std::time::Duration;
use tracing::{error, info};
use crate::rate_limiter::RateLimiter;
use base64::Engine;

pub struct GeminiClient {
    api_key: String,
    api_endpoint: String,
    image_endpoint: String,
    prompt_wrapper: String,
    bot_name: String,
    rate_limiter: RateLimiter,
    context_messages: usize,
    log_prompts: bool,
}

impl GeminiClient {
    pub fn new(
        api_key: String, 
        api_endpoint: Option<String>,
        prompt_wrapper: Option<String>,
        bot_name: String,
        rate_limit_minute: u32,
        rate_limit_day: u32,
        context_messages: usize,
        log_prompts: bool
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
            log_prompts,
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
        // Format the context messages - already in chronological order from the database query
        let context = if !context_messages.is_empty() {
            // Take only the configured number of messages
            let limited_messages = if context_messages.len() > self.context_messages {
                // Take the most recent messages (which are at the end since we changed the order)
                let start_idx = context_messages.len() - self.context_messages;
                &context_messages[start_idx..]
            } else {
                context_messages
            };
            
            // Format each message as "DisplayName: Message" using the display_name field
            limited_messages.iter()
                .map(|(_, display_name, msg)| format!("{}: {}", display_name, msg))
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
    
    // Generate content with a raw prompt and retry on overload errors
    pub async fn generate_content(&self, prompt: &str) -> Result<String> {
        // Maximum number of retries
        const MAX_RETRIES: usize = 5;
        
        // Initial delay in seconds (will be doubled each retry - exponential backoff)
        let mut delay_secs = 10;
        
        // Try up to MAX_RETRIES times
        for attempt in 1..=MAX_RETRIES {
            // Use acquire() which includes retry logic and request recording
            self.rate_limiter.acquire().await?;
            
            // Log the prompt if enabled
            if self.log_prompts {
                info!("Gemini API Prompt: {}", prompt);
            }
            
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
            
            // Log the full raw response if logging is enabled
            if self.log_prompts {
                if let Ok(pretty_json) = serde_json::to_string_pretty(&response_json) {
                    info!("Gemini API Raw Response: {}", pretty_json);
                } else {
                    info!("Gemini API Raw Response: {}", response_json);
                }
            }
            
            // Check for error in response
            if let Some(error) = response_json.get("error") {
                let error_message = error.get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Unknown API error");
                let error_code = error.get("code")
                    .and_then(|c| c.as_u64())
                    .unwrap_or(0);
                
                // Check if this is an overload error that we should retry
                if error_message.contains("overloaded") || error_message.contains("try again later") {
                    if attempt < MAX_RETRIES {
                        // Log that we're retrying
                        info!("Gemini API overloaded (attempt {}/{}), retrying in {} seconds...", 
                             attempt, MAX_RETRIES, delay_secs);
                        
                        // Wait before retrying
                        tokio::time::sleep(Duration::from_secs(delay_secs)).await;
                        
                        // Double the delay for next attempt (exponential backoff)
                        delay_secs *= 2;
                        
                        // Continue to the next retry attempt
                        continue;
                    }
                }
                
                // If we've exhausted retries or it's not a retryable error, return the error
                error!("Gemini API error (code {}): {}", error_code, error_message);
                return Err(anyhow::anyhow!("Gemini API error: {}", error_message));
            }
            
            // Check for finish reason
            if let Some(candidates) = response_json.get("candidates") {
                if let Some(candidate) = candidates.get(0) {
                    if let Some(finish_reason) = candidate.get("finishReason") {
                        if finish_reason != "STOP" {
                            let reason = finish_reason.as_str().unwrap_or("UNKNOWN");
                            error!("Gemini API response has non-STOP finish reason: {}", reason);
                            if reason == "SAFETY" {
                                return Err(anyhow::anyhow!("Gemini API safety filters triggered. The prompt may contain inappropriate content."));
                            } else if reason == "RECITATION" {
                                return Err(anyhow::anyhow!("Gemini API detected content recitation. The response may contain copied content."));
                            } else if reason == "OTHER" {
                                return Err(anyhow::anyhow!("Gemini API terminated the response for an unspecified reason."));
                            }
                        }
                    }
                }
            }
            
            // Extract the generated text
            if let Some(text) = response_json
                .get("candidates")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("content"))
                .and_then(|c| c.get("parts"))
                .and_then(|p| p.get(0))
                .and_then(|p| p.get("text"))
                .and_then(|t| t.as_str()) {
                
                // Log the response if enabled
                if self.log_prompts {
                    info!("Gemini API Response Text: {}", text);
                } else {
                    info!("Successfully generated content from Gemini API");
                }
                
                // Success! Return the text
                return Ok(text.to_string());
            } else {
                // Check for prompt feedback
                let prompt_feedback = if let Some(feedback) = response_json.get("promptFeedback") {
                    if let Some(block_reason) = feedback.get("blockReason") {
                        format!("Prompt blocked: {}", block_reason.as_str().unwrap_or("UNKNOWN"))
                    } else {
                        "Prompt feedback present but no block reason specified".to_string()
                    }
                } else {
                    "No prompt feedback available".to_string()
                };
                
                error!("Failed to extract text from Gemini API response: {}", prompt_feedback);
                return Err(anyhow::anyhow!("Failed to extract text from Gemini API response: {}", prompt_feedback));
            }
        }
        
        // This should never be reached due to the return statements above,
        // but we need it for the compiler
        Err(anyhow::anyhow!("Maximum retry attempts exceeded"))
    }

    // Generate an image from a text prompt
    pub async fn generate_image(&self, prompt: &str) -> Result<(Vec<u8>, String)> {
        // Use acquire() which includes retry logic and request recording
        self.rate_limiter.acquire().await?;
        
        // Prepare the request body for the gemini-2.0-flash-preview-image-generation model
        // Based on the working example using responseModalities
        let request_body = serde_json::json!({
            "contents": [{
                "parts": [{
                    "text": prompt
                }]
            }],
            "generationConfig": {
                "responseModalities": ["TEXT", "IMAGE"]
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
        
        // Create a copy of the response for logging, but remove the image data to avoid huge logs
        let mut log_json = response_json.clone();
        if let Some(candidates) = log_json.get_mut("candidates") {
            if let Some(candidate) = candidates.get_mut(0) {
                if let Some(content) = candidate.get_mut("content") {
                    if let Some(parts) = content.get_mut("parts") {
                        // Check for image data in the first part (alternative format)
                        if let Some(part) = parts.get_mut(0) {
                            if let Some(inline_data) = part.get_mut("inlineData") {
                                if let Some(data) = inline_data.get_mut("data") {
                                    *data = serde_json::Value::String("[IMAGE DATA REDACTED]".to_string());
                                }
                            }
                        }
                        
                        // Check for image data in the second part (typical format)
                        if let Some(part) = parts.get_mut(1) {
                            if let Some(inline_data) = part.get_mut("inlineData") {
                                if let Some(data) = inline_data.get_mut("data") {
                                    *data = serde_json::Value::String("[IMAGE DATA REDACTED]".to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
        
        // Log the redacted response
        info!("Image generation API response: {}", serde_json::to_string_pretty(&log_json)?);
        
        // Extract the text description
        let text_description = response_json
            .get("candidates")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.get(0))
            .and_then(|p| p.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("").to_string();
        
        // Extract the generated image data - handle both possible response formats
        let mut image_data = None;
        
        // First try to find the image in the second part (typical format)
        if let Some(data) = response_json
            .get("candidates")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.get(1))  // The second part contains the image
            .and_then(|p| p.get("inlineData"))
            .and_then(|i| i.get("data"))
            .and_then(|d| d.as_str()) {
            image_data = Some(data);
        }
        
        // If not found, try to find it in the first part (alternative format)
        if image_data.is_none() {
            if let Some(data) = response_json
                .get("candidates")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("content"))
                .and_then(|c| c.get("parts"))
                .and_then(|p| p.get(0))
                .and_then(|p| p.get("inlineData"))
                .and_then(|i| i.get("data"))
                .and_then(|d| d.as_str()) {
                image_data = Some(data);
            }
        }
        
        // Process the image data if found
        if let Some(image_data) = image_data {
            info!("Successfully generated image from Gemini API");
            
            // Decode base64 image data
            match base64::engine::general_purpose::STANDARD.decode(image_data) {
                Ok(bytes) => Ok((bytes, text_description)),
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
