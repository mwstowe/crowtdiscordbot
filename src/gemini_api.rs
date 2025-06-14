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
    #[allow(dead_code)]
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
    
    // Generate a response with conversation context
    pub async fn generate_response_with_context(
        &self, 
        prompt: &str, 
        user_name: &str,
        context_messages: &Vec<(String, String, String)>,
        _user_pronouns: Option<&str>
    ) -> Result<String> {
        // Check if the prompt already contains context (like in interjection prompts)
        let has_context_in_prompt = prompt.contains("{context}");
        
        // Format the context messages
        let context = if !context_messages.is_empty() {
            // Get the messages in chronological order (oldest first)
            // The database query returns newest first, so we need to reverse
            let mut chronological_messages = context_messages.clone();
            chronological_messages.reverse();
            
            // Format each message as "DisplayName: Message" using the display_name field
            // If display_name is empty, fall back to author name
            let formatted_messages = chronological_messages.iter()
                .map(|(author, display_name, msg)| {
                    let name_to_use = if !display_name.is_empty() {
                        display_name
                    } else {
                        author
                    };
                    format!("{}: {}", name_to_use, msg)
                })
                .collect::<Vec<_>>()
                .join("\n");
            
            info!("Using context for response generation: {} messages", context_messages.len());
            formatted_messages
        } else if has_context_in_prompt {
            // If the prompt already contains context placeholder but we have no messages,
            // use an empty string to avoid adding "No context available"
            info!("No database context available, but prompt contains context placeholder");
            "".to_string()
        } else {
            info!("No context available for response generation to user: {}", user_name);
            "No context available.".to_string()
        };
        
        // Format the prompt using the wrapper
        let formatted_prompt = if has_context_in_prompt {
            // If the prompt already contains {context}, use it directly
            prompt.replace("{bot_name}", &self.bot_name)
                 .replace("{context}", &context)
        } else {
            // Otherwise use the standard wrapper
            self.prompt_wrapper
                .replace("{message}", prompt)
                .replace("{bot_name}", &self.bot_name)
                .replace("{user}", user_name)
                .replace("{context}", &context)
        };
            
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
                    } else {
                        // If we've exhausted retries for overload errors, return a special error
                        // that callers can check for to avoid showing error messages to users
                        error!("Gemini API overloaded, maximum retries ({}) exceeded", MAX_RETRIES);
                        return Err(anyhow::anyhow!("SILENT_ERROR: Gemini API overloaded after {} retries", MAX_RETRIES));
                    }
                }
                
                // If it's not a retryable error, return the error
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
        
        // Check for safety blocks or other issues
        if let Some(candidates) = response_json.get("candidates") {
            if let Some(candidate) = candidates.get(0) {
                // Check for finish reason
                if let Some(finish_reason) = candidate.get("finishReason") {
                    let reason = finish_reason.as_str().unwrap_or("UNKNOWN");
                    if reason == "IMAGE_SAFETY" {
                        // Extract the text response which contains the safety explanation
                        let safety_message = response_json
                            .get("candidates")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("content"))
                            .and_then(|c| c.get("parts"))
                            .and_then(|p| p.get(0))
                            .and_then(|p| p.get("text"))
                            .and_then(|t| t.as_str())
                            .unwrap_or("I'm unable to generate that image due to content policy restrictions.");
                            
                        error!("Image generation blocked due to safety concerns: {}", safety_message);
                        return Err(anyhow::anyhow!("SAFETY_BLOCKED: \"{}\"", safety_message));
                    }
                }
                
                // Check for safety ratings with blocked=true
                if let Some(safety_ratings) = candidate.get("safetyRatings") {
                    if safety_ratings.as_array().map_or(false, |ratings| {
                        ratings.iter().any(|rating| {
                            rating.get("blocked").and_then(|b| b.as_bool()).unwrap_or(false)
                        })
                    }) {
                        // Extract the text response which contains the safety explanation
                        let safety_message = response_json
                            .get("candidates")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("content"))
                            .and_then(|c| c.get("parts"))
                            .and_then(|p| p.get(0))
                            .and_then(|p| p.get("text"))
                            .and_then(|t| t.as_str())
                            .unwrap_or("I'm unable to generate that image due to content policy restrictions.");
                            
                        error!("Image generation blocked due to safety ratings: {}", safety_message);
                        return Err(anyhow::anyhow!("SAFETY_BLOCKED: \"{}\"", safety_message));
                    }
                }
            }
        }
        
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
            
        // Check if the text response indicates a safety block
        // This handles cases where the API returns a text explanation instead of an image
        if text_description.contains("unable to create") || 
           text_description.contains("can't generate") || 
           text_description.contains("cannot generate") ||
           text_description.contains("policy violation") ||
           text_description.contains("content policy") {
            error!("Image generation blocked based on text response: {}", text_description);
            return Err(anyhow::anyhow!("SAFETY_BLOCKED: \"{}\"", text_description));
        }
        
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
