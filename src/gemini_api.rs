use anyhow::Result;
use reqwest;
use serde_json;
use tracing::info;
use regex::Regex;
use std::time::Duration;
use std::sync::Arc;

// Import our rate limiter
use crate::rate_limiter::RateLimiter;

pub struct GeminiClient {
    api_key: String,
    api_endpoint: String,
    prompt_wrapper: String,
    bot_name: String,
    rate_limiter: Arc<RateLimiter>,
}

impl GeminiClient {
    pub fn new(
        api_key: String, 
        api_endpoint: Option<String>, 
        prompt_wrapper: Option<String>,
        bot_name: String,
        minute_limit: u32,
        day_limit: u32
    ) -> Self {
        let default_endpoint = "https://generativelanguage.googleapis.com/v1beta/models/gemini-1.5-flash:generateContent";
        let default_prompt = "You are {bot_name}, a helpful and friendly Discord bot. Here is recent conversation context:\n\n{context}\n\nRespond to {user}: {message}";
        
        // Create the rate limiter with the specified limits
        let rate_limiter = Arc::new(RateLimiter::new(minute_limit, day_limit));
        
        Self {
            api_key,
            api_endpoint: api_endpoint.unwrap_or_else(|| default_endpoint.to_string()),
            prompt_wrapper: prompt_wrapper.unwrap_or_else(|| default_prompt.to_string()),
            bot_name,
            rate_limiter,
        }
    }
    
    // Function to strip pronouns from display names
    pub fn strip_pronouns(&self, display_name: &str) -> String {
        // Remove content in parentheses (they/them)
        let without_parentheses = Regex::new(r"\s*\([^)]*\)").unwrap_or_else(|_| Regex::new(r"").unwrap())
            .replace_all(display_name, "").to_string();
        
        // Remove content in brackets [she/her]
        let without_brackets = Regex::new(r"\s*\[[^\]]*\]").unwrap_or_else(|_| Regex::new(r"").unwrap())
            .replace_all(&without_parentheses, "").to_string();
        
        // Remove content after | or pipe character (common separator for pronouns)
        let without_pipe = without_brackets.split('|').next().unwrap_or("").trim().to_string();
        
        // Return cleaned name, or original if empty
        if without_pipe.is_empty() {
            display_name.to_string()
        } else {
            without_pipe
        }
    }

    pub async fn generate_response(&self, prompt: &str, user_name: &str) -> Result<String> {
        // Format the prompt using the wrapper, including the user's name
        // For backward compatibility, replace {context} with empty string if it exists
        let formatted_prompt = self.prompt_wrapper
            .replace("{message}", prompt)
            .replace("{bot_name}", &self.bot_name)
            .replace("{user}", user_name)
            .replace("{context}", "No recent conversation.");
        
        self.call_gemini_api(&formatted_prompt).await
    }
    
    pub async fn generate_response_with_context(
        &self, 
        prompt: &str, 
        user_name: &str, 
        context_messages: &[(String, String, String)]
    ) -> Result<String> {
        // Format the context messages
        let context = if context_messages.is_empty() {
            "No recent conversation.".to_string()
        } else {
            let mut formatted_context = String::new();
            for (_, display_name, content) in context_messages {
                formatted_context.push_str(&format!("{}: {}\n", display_name, content));
            }
            formatted_context.trim().to_string()
        };
        
        // Format the prompt using the wrapper, including the user's name and context
        let formatted_prompt = self.prompt_wrapper
            .replace("{message}", prompt)
            .replace("{bot_name}", &self.bot_name)
            .replace("{user}", user_name)
            .replace("{context}", &context);
        
        self.call_gemini_api(&formatted_prompt).await
    }
    
    async fn call_gemini_api(&self, formatted_prompt: &str) -> Result<String> {
        // First, try to acquire a rate limit token
        match self.rate_limiter.acquire().await {
            Ok(()) => {
                // We've acquired the token, proceed with the API call
                info!("Rate limit check passed, proceeding with Gemini API call");
            },
            Err(e) => {
                // If it's a daily limit error, return it directly
                if e.to_string().contains("Daily rate limit reached") {
                    return Err(e);
                }
                // For other errors (which shouldn't happen with acquire()), return them
                return Err(e);
            }
        }
        
        info!("Calling Gemini API with prompt: {}", formatted_prompt);
        
        // Create the request body
        let request_body = serde_json::json!({
            "contents": [{
                "parts": [{
                    "text": formatted_prompt
                }]
            }],
            "generationConfig": {
                "temperature": 0.7,
                "topK": 40,
                "topP": 0.95,
                "maxOutputTokens": 1024
            }
        });
        
        // Create the client
        let client = reqwest::Client::new();
        
        // Implement retry logic for 503 errors
        let max_retries = 5;
        let retry_delay_secs = 15;
        let mut attempts = 0;
        
        loop {
            attempts += 1;
            info!("Attempt {} of {} to call Gemini API", attempts, max_retries);
            
            // Make the request
            let response = client.post(format!("{}?key={}", self.api_endpoint, self.api_key))
                .header("Content-Type", "application/json")
                .body(request_body.to_string())
                .send()
                .await?;
            
            let status = response.status();
            
            // Check if the request was successful
            if status.is_success() {
                // Parse the response
                let response_json: serde_json::Value = response.json().await?;
                
                // Extract the generated text
                let generated_text = response_json["candidates"][0]["content"]["parts"][0]["text"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Failed to extract text from Gemini API response"))?
                    .to_string();
                
                return Ok(generated_text);
            } else if status == reqwest::StatusCode::SERVICE_UNAVAILABLE && attempts < max_retries {
                // If we get a 503 and haven't exceeded max retries, wait and try again
                let error_text = response.text().await?;
                info!("Received 503 error from Gemini API: {}. Retrying in {} seconds...", error_text, retry_delay_secs);
                tokio::time::sleep(Duration::from_secs(retry_delay_secs)).await;
                continue;
            } else {
                // For other errors or if we've exceeded max retries, return an error
                let error_text = response.text().await?;
                if status == reqwest::StatusCode::SERVICE_UNAVAILABLE {
                    return Err(anyhow::anyhow!("Gemini API unavailable after {} attempts: {}", max_retries, error_text));
                } else {
                    return Err(anyhow::anyhow!("Gemini API request failed with status {}: {}", status, error_text));
                }
            }
        }
    }
}
