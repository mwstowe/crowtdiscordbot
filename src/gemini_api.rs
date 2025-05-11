use anyhow::Result;
use reqwest;
use serde_json;
use tracing::info;
use regex::Regex;
use std::time::Duration;

pub struct GeminiClient {
    api_key: String,
    api_endpoint: String,
    prompt_wrapper: String,
    bot_name: String,
}

impl GeminiClient {
    pub fn new(
        api_key: String, 
        api_endpoint: Option<String>, 
        prompt_wrapper: Option<String>,
        bot_name: String
    ) -> Self {
        let default_endpoint = "https://generativelanguage.googleapis.com/v1beta/models/gemini-1.5-flash:generateContent";
        let default_prompt = "You are {bot_name}, a helpful and friendly Discord bot. Respond to {user}: {message}";
        
        Self {
            api_key,
            api_endpoint: api_endpoint.unwrap_or_else(|| default_endpoint.to_string()),
            prompt_wrapper: prompt_wrapper.unwrap_or_else(|| default_prompt.to_string()),
            bot_name,
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
        let formatted_prompt = self.prompt_wrapper
            .replace("{message}", prompt)
            .replace("{bot_name}", &self.bot_name)
            .replace("{user}", user_name);
        
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
