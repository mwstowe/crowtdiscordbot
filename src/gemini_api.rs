use anyhow::Result;
use reqwest;
use serde_json;
use tracing::info;
use regex::Regex;

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
        
        // Make the request
        let response = client.post(format!("{}?key={}", self.api_endpoint, self.api_key))
            .header("Content-Type", "application/json")
            .body(request_body.to_string())
            .send()
            .await?;
        
        // Check if the request was successful
        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("Gemini API request failed: {}", error_text));
        }
        
        // Parse the response
        let response_json: serde_json::Value = response.json().await?;
        
        // Extract the generated text
        let generated_text = response_json["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Failed to extract text from Gemini API response"))?
            .to_string();
        
        Ok(generated_text)
    }
}
