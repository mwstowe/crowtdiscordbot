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
    
    // Static version of strip_pronouns for use without an instance
    pub fn strip_pronouns_static(display_name: &str) -> String {
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
    
    // Function to strip pronouns from display names
    pub fn strip_pronouns(&self, display_name: &str) -> String {
        Self::strip_pronouns_static(display_name)
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
        context_messages: &[(String, String, String)],
        user_pronouns: Option<&str>
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
        
        // Create the client and make the request
        let client = reqwest::Client::new();
        let response = client
            .post(&self.api_endpoint)
            .query(&[("key", &self.api_key)])
            .json(&payload)
            .timeout(Duration::from_secs(30))
            .send()
            .await?;
        
        // Check if the request was successful
        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("API request failed with status {}: {}", status, error_text));
        }
        
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
        
        Ok(generated_text.to_string())
    }
}
