use anyhow::Result;
use reqwest;
use serde_json;
use std::time::Duration;
use tracing::{info, error};
use crate::rate_limiter::RateLimiter;

pub struct GeminiClient {
    api_key: String,
    api_endpoint: String,
    prompt_wrapper: String,
    bot_name: String,
    rate_limiter: RateLimiter,
}

impl GeminiClient {
    pub fn new(
        api_key: String, 
        api_endpoint: Option<String>,
        prompt_wrapper: Option<String>,
        bot_name: String,
        rate_limit_minute: u32,
        rate_limit_day: u32
    ) -> Self {
        // Default endpoint for Gemini API
        let default_endpoint = "https://generativelanguage.googleapis.com/v1beta/models/gemini-1.5-flash:generateContent".to_string();
        
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
            prompt_wrapper,
            bot_name,
            rate_limiter,
        }
    }
    
    // Helper function to strip pronouns from display names
    pub fn strip_pronouns(&self, display_name: &str) -> String {
        Self::strip_pronouns_static(display_name)
    }
    
    // Static version of the strip_pronouns function
    pub fn strip_pronouns_static(display_name: &str) -> String {
        // Check for common pronoun formats like (he/him), [she/her], etc.
        if let Some(idx) = display_name.find(|c| c == '(' || c == '[' || c == '<') {
            if display_name[idx..].contains("he/") || 
               display_name[idx..].contains("she/") || 
               display_name[idx..].contains("they/") ||
               display_name[idx..].contains("xe/") ||
               display_name[idx..].contains("ze/") ||
               display_name[idx..].contains("any/") ||
               display_name[idx..].contains("it/") {
                return display_name[0..idx].trim().to_string();
            }
        }
        
        display_name.to_string()
    }
    
    // Generate a response using the Gemini API
    pub async fn generate_response(&self, prompt: &str, user_name: &str) -> Result<String> {
        // Format the prompt using the wrapper
        let formatted_prompt = self.prompt_wrapper
            .replace("{message}", prompt)
            .replace("{bot_name}", &self.bot_name)
            .replace("{user}", user_name)
            .replace("{context}", "No context available.");
            
        self.call_gemini_api(&formatted_prompt).await
    }
    
    // Generate a response with conversation context
    pub async fn generate_response_with_context(
        &self, 
        prompt: &str, 
        user_name: &str,
        context_messages: &Vec<(String, String, String)>,
        user_pronouns: Option<&str>
    ) -> Result<String> {
        // Format the context messages
        let context = if !context_messages.is_empty() {
            let formatted_messages: Vec<String> = context_messages.iter()
                .map(|(_author, display_name, content)| format!("{}: {}", display_name, content))
                .collect();
            formatted_messages.join("\n")
        } else {
            "No recent messages".to_string()
        };
        
        // Format the prompt using the wrapper, including the user's name and context
        let mut formatted_prompt = self.prompt_wrapper
            .replace("{message}", prompt)
            .replace("{bot_name}", &self.bot_name)
            .replace("{user}", user_name)
            .replace("{context}", &context);
            
        // Add pronouns information if available
        if let Some(pronouns) = user_pronouns {
            formatted_prompt = format!("{}\n\nNote: {} uses {} pronouns.", 
                formatted_prompt, user_name, pronouns);
        }
        
        self.call_gemini_api(&formatted_prompt).await
    }
    
    async fn call_gemini_api(&self, formatted_prompt: &str) -> Result<String> {
        // First, try to acquire a rate limit token
        match self.rate_limiter.acquire().await {
            Ok(_) => {
                // Rate limit token acquired, proceed with API call
                info!("Rate limit token acquired, making API call");
            },
            Err(e) => {
                // Rate limit exceeded
                return Err(anyhow::anyhow!("Rate limit exceeded: {}", e));
            }
        }
        
        // Prepare the request payload
        let payload = serde_json::json!({
            "contents": [{
                "parts": [{
                    "text": formatted_prompt
                }]
            }]
        });
        
        // Create the client for reuse in retries
        let client = reqwest::Client::new();
        
        // Implement retry logic with backoff
        let max_retries = 5;
        let retry_delay = Duration::from_secs(15);
        
        for attempt in 1..=max_retries {
            // Make the request
            let response = client
                .post(&self.api_endpoint)
                .query(&[("key", &self.api_key)])
                .json(&payload)
                .timeout(Duration::from_secs(30))
                .send()
                .await?;
            
            // Check if the request was successful
            if response.status().is_success() {
                // Parse the response
                let response_json: serde_json::Value = response.json().await?;
                
                // Extract the generated text
                let generated_text = response_json
                    .get("candidates")
                    .and_then(|candidates| candidates.get(0))
                    .and_then(|candidate| candidate.get("content"))
                    .and_then(|content| content.get("parts"))
                    .and_then(|parts| parts.get(0))
                    .and_then(|part| part.get("text"))
                    .and_then(|text| text.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Failed to extract text from API response"))?;
                
                return Ok(generated_text.to_string());
            } else {
                let status = response.status();
                let error_text = response.text().await?;
                
                // Check if it's a 503 Service Unavailable error
                if status == reqwest::StatusCode::SERVICE_UNAVAILABLE {
                    if attempt < max_retries {
                        info!("Received 503 error from Gemini API (attempt {}/{}), retrying in {} seconds: {}", 
                              attempt, max_retries, retry_delay.as_secs(), error_text);
                        tokio::time::sleep(retry_delay).await;
                        continue;
                    } else {
                        // We've exhausted our retries for 503 errors
                        // Return a special error that the caller can recognize to avoid showing an error message
                        return Err(anyhow::anyhow!("SILENT_FAILURE_503"));
                    }
                } else {
                    // For other errors, return the error immediately
                    return Err(anyhow::anyhow!("API request failed with status {}: {}", status, error_text));
                }
            }
        }
        
        // This should never be reached due to the loop structure, but Rust requires a return value
        Err(anyhow::anyhow!("SILENT_FAILURE_503"))
    }
}
