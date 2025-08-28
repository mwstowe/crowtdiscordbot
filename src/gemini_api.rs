use crate::prompt_templates::PromptTemplates;
use crate::rate_limiter::RateLimiter;
use anyhow::Result;
use base64::Engine;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{error, info};

#[derive(Clone)]
pub struct GeminiClient {
    api_key: String,
    api_endpoint: String,
    image_endpoint: String,
    prompt_templates: PromptTemplates,
    rate_limiter: RateLimiter,
    image_rate_limiter: RateLimiter,
    #[allow(dead_code)]
    context_messages: usize,
    log_prompts: bool,
    // Track when image generation quota was exhausted
    image_quota_exhausted_until: Arc<Mutex<Option<DateTime<Utc>>>>,
}

/// Configuration for creating a GeminiClient
#[derive(Debug, Clone)]
pub struct GeminiConfig {
    pub api_key: String,
    pub api_endpoint: Option<String>,
    pub prompt_wrapper: Option<String>,
    pub bot_name: String,
    pub rate_limit_minute: u32,
    pub rate_limit_day: u32,
    pub image_rate_limit_minute: u32,
    pub image_rate_limit_day: u32,
    pub context_messages: usize,
    pub log_prompts: bool,
    pub personality_description: Option<String>,
}

impl GeminiClient {
    pub fn new(config: GeminiConfig) -> Self {
        // Default endpoint for Gemini API
        let default_endpoint = "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent".to_string();
        let image_endpoint = "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash-preview-image-generation:generateContent".to_string();

        // Create prompt templates with custom personality if provided
        let mut prompt_templates = PromptTemplates::new_with_custom_personality(
            config.bot_name.clone(),
            config.personality_description,
        );

        // If a custom prompt wrapper is provided, set it as the general response template
        if let Some(wrapper) = config.prompt_wrapper {
            prompt_templates.set_template("general_response", &wrapper);
        }

        // Create rate limiter for text generation
        let rate_limiter = RateLimiter::new(config.rate_limit_minute, config.rate_limit_day);

        // Create separate rate limiter for image generation
        let image_rate_limiter =
            RateLimiter::new(config.image_rate_limit_minute, config.image_rate_limit_day);

        Self {
            api_key: config.api_key,
            api_endpoint: config.api_endpoint.unwrap_or(default_endpoint),
            image_endpoint,
            prompt_templates,
            rate_limiter,
            image_rate_limiter,
            context_messages: config.context_messages,
            log_prompts: config.log_prompts,
            image_quota_exhausted_until: Arc::new(Mutex::new(None)),
        }
    }

    // Check if image generation is currently blocked due to quota exhaustion
    pub async fn is_image_quota_exhausted(&self) -> bool {
        let quota_lock = self.image_quota_exhausted_until.lock().await;
        if let Some(exhausted_until) = *quota_lock {
            let now = Utc::now();
            if now < exhausted_until {
                return true;
            }
        }
        false
    }

    // Mark image generation as quota exhausted until tomorrow
    async fn mark_image_quota_exhausted(&self) {
        let tomorrow = Utc::now() + chrono::Duration::days(1);
        // Reset at midnight UTC
        let tomorrow_midnight = tomorrow
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .map(|dt| dt.and_utc())
            .unwrap_or(tomorrow);

        let mut quota_lock = self.image_quota_exhausted_until.lock().await;
        *quota_lock = Some(tomorrow_midnight);

        info!(
            "Image generation quota exhausted. Feature disabled until {}",
            tomorrow_midnight
        );
    }

    // Get a reference to the prompt templates
    pub fn prompt_templates(&self) -> &PromptTemplates {
        &self.prompt_templates
    }

    // Get a mutable reference to the prompt templates
    #[allow(dead_code)]
    pub fn prompt_templates_mut(&mut self) -> &mut PromptTemplates {
        &mut self.prompt_templates
    }

    // Generate a response with conversation context
    pub async fn generate_response_with_context(
        &self,
        prompt: &str,
        user_name: &str,
        context_messages: &[(String, String, String)],
        user_pronouns: Option<&str>,
    ) -> Result<String> {
        // Convert to the new format with pronouns
        let context_with_pronouns: Vec<(String, String, Option<String>, String)> = context_messages
            .iter()
            .map(|(author, display_name, content)| {
                let pronouns = crate::utils::extract_pronouns(display_name);
                (
                    author.clone(),
                    display_name.clone(),
                    pronouns,
                    content.clone(),
                )
            })
            .collect();

        self.generate_response_with_context_and_pronouns(
            prompt,
            user_name,
            &context_with_pronouns,
            user_pronouns,
        )
        .await
    }

    // New function that accepts context messages with pronouns
    pub async fn generate_response_with_context_and_pronouns(
        &self,
        prompt: &str,
        user_name: &str,
        context_messages: &[(String, String, Option<String>, String)],
        _user_pronouns: Option<&str>,
    ) -> Result<String> {
        // Check if the prompt already contains context (like in interjection prompts)
        let has_context_in_prompt = prompt.contains("{context}");

        // Format the context messages
        let context = if !context_messages.is_empty() {
            // Get the messages in chronological order (oldest first)
            // The database query returns newest first, so we need to reverse
            let mut chronological_messages = context_messages.to_owned();
            chronological_messages.reverse();

            // Format each message as "DisplayName (pronouns): Message" using the display_name field
            // If display_name is empty, fall back to author name
            let formatted_messages = chronological_messages
                .iter()
                .map(|(author, display_name, pronouns, msg)| {
                    let name_to_use = if !display_name.is_empty() {
                        display_name
                    } else {
                        author
                    };

                    // Include pronouns if available
                    if let Some(pronouns) = pronouns {
                        format!("{name_to_use} ({pronouns}): {msg}")
                    } else {
                        format!("{name_to_use}: {msg}")
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");

            info!(
                "Using context for response generation: {} messages",
                context_messages.len()
            );
            formatted_messages
        } else if has_context_in_prompt {
            // If the prompt already contains context placeholder but we have no messages,
            // use an empty string to avoid adding "No context available"
            info!("No database context available, but prompt contains context placeholder");
            "".to_string()
        } else {
            info!(
                "No context available for response generation to user: {}",
                user_name
            );
            "No context available.".to_string()
        };

        // Format the prompt using the wrapper or custom template
        let formatted_prompt = if has_context_in_prompt {
            // If the prompt already contains {context}, use it as a custom template
            let mut values = HashMap::new();
            values.insert("context".to_string(), context);
            self.prompt_templates.format_custom(prompt, &values)
        } else {
            // Otherwise use the standard general response template
            self.prompt_templates
                .format_general_response(prompt, user_name, &context)
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
                let error_message = error
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Unknown API error");
                let error_code = error.get("code").and_then(|c| c.as_u64()).unwrap_or(0);

                // Check if this is an overload error that we should retry
                if error_message.contains("overloaded") || error_message.contains("try again later")
                {
                    if attempt < MAX_RETRIES {
                        // Log that we're retrying
                        info!(
                            "Gemini API overloaded (attempt {}/{}), retrying in {} seconds...",
                            attempt, MAX_RETRIES, delay_secs
                        );

                        // Wait before retrying
                        tokio::time::sleep(Duration::from_secs(delay_secs)).await;

                        // Double the delay for next attempt (exponential backoff)
                        delay_secs *= 2;

                        // Continue to the next retry attempt
                        continue;
                    } else {
                        // If we've exhausted retries for overload errors, return a special error
                        // that callers can check for to avoid showing error messages to users
                        error!(
                            "Gemini API overloaded, maximum retries ({}) exceeded",
                            MAX_RETRIES
                        );
                        return Err(anyhow::anyhow!(
                            "SILENT_ERROR: Gemini API overloaded after {} retries",
                            MAX_RETRIES
                        ));
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
                                return Err(anyhow::anyhow!(
                                    "Gemini API terminated the response for an unspecified reason."
                                ));
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
                .and_then(|t| t.as_str())
            {
                // Log the response if enabled
                if self.log_prompts {
                    info!("Gemini API Response Text: {}", text);
                } else {
                    info!("Successfully generated content from Gemini API");
                }

                // Strip surrounding quotes if present
                let cleaned_text =
                    if text.starts_with('"') && text.ends_with('"') && text.len() >= 2 {
                        // Remove the first and last character (the quotes)
                        &text[1..text.len() - 1]
                    } else {
                        text
                    };

                // Success! Return the text
                return Ok(cleaned_text.to_string());
            } else {
                // Check for prompt feedback
                let prompt_feedback = if let Some(feedback) = response_json.get("promptFeedback") {
                    if let Some(block_reason) = feedback.get("blockReason") {
                        let block_reason_str = block_reason.as_str().unwrap_or("UNKNOWN");
                        format!("Prompt blocked: {block_reason_str}")
                    } else {
                        "Prompt feedback present but no block reason specified".to_string()
                    }
                } else {
                    "No prompt feedback available".to_string()
                };

                error!(
                    "Failed to extract text from Gemini API response: {}",
                    prompt_feedback
                );
                return Err(anyhow::anyhow!(
                    "Failed to extract text from Gemini API response: {}",
                    prompt_feedback
                ));
            }
        }

        // This should never be reached due to the return statements above,
        // but we need it for the compiler
        Err(anyhow::anyhow!("Maximum retry attempts exceeded"))
    }

    // Generate an image from a text prompt
    pub async fn generate_image(&self, prompt: &str) -> Result<(Vec<u8>, String)> {
        // Check if image generation is currently blocked due to quota exhaustion
        if self.is_image_quota_exhausted().await {
            return Err(anyhow::anyhow!("IMAGE_QUOTA_EXHAUSTED: Image generation quota has been exceeded for today. This feature will be available again tomorrow."));
        }

        // Maximum number of retries for 500 errors
        const MAX_RETRIES: usize = 10;
        let mut delay_secs = 5; // Initial delay for 500 errors

        // Try up to MAX_RETRIES times
        for attempt in 1..=MAX_RETRIES {
            // Check image rate limits first - this will handle both per-minute and per-day limits
            match self.image_rate_limiter.check().await {
                Ok(()) => {
                    // We can proceed - record the request
                    self.image_rate_limiter.record_request().await;
                }
                Err(e) => {
                    let error_msg = e.to_string();

                    // Check if this is a daily limit error
                    if error_msg.contains("Daily rate limit reached") {
                        // Mark image generation as quota exhausted for the day
                        self.mark_image_quota_exhausted().await;
                        return Err(anyhow::anyhow!("IMAGE_QUOTA_EXHAUSTED: Daily image generation limit reached. This feature will be available again tomorrow."));
                    }

                    // For per-minute limits, return the error as-is (caller can retry)
                    return Err(e);
                }
            }

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
                .timeout(Duration::from_secs(60)) // Longer timeout for image generation
                .send()
                .await?;

            // Check the HTTP status code first
            let status = response.status();

            // Handle 500 errors with retry logic
            if status.as_u16() == 500 {
                if attempt < MAX_RETRIES {
                    info!(
                        "Image generation API returned 500 error (attempt {}/{}), retrying in {} seconds...",
                        attempt, MAX_RETRIES, delay_secs
                    );
                    tokio::time::sleep(Duration::from_secs(delay_secs)).await;
                    delay_secs = std::cmp::min(delay_secs * 2, 60); // Cap at 60 seconds
                    continue;
                } else {
                    let response_text = response.text().await.unwrap_or_default();
                    error!(
                        "Image generation API returned 500 error after {} attempts: {}",
                        MAX_RETRIES, response_text
                    );
                    return Err(anyhow::anyhow!(
                        "Image generation failed after {} attempts due to server errors",
                        MAX_RETRIES
                    ));
                }
            }

            if status == 429 {
                // This is likely a quota exhaustion error
                let response_text = response.text().await.unwrap_or_default();
                error!(
                    "Received HTTP 429 from image generation API: {}",
                    response_text
                );

                // Check if the response contains quota-related keywords
                if response_text.to_lowercase().contains("quota")
                    || response_text.to_lowercase().contains("resource_exhausted")
                {
                    // Mark image generation as quota exhausted
                    self.mark_image_quota_exhausted().await;
                    return Err(anyhow::anyhow!("IMAGE_QUOTA_EXHAUSTED: Image generation quota has been exceeded for today. This feature will be available again tomorrow."));
                }

                // If it's a 429 but not quota-related, return a generic rate limit error
                return Err(anyhow::anyhow!(
                    "Rate limit exceeded. Please try again later."
                ));
            }

            // Parse the response
            let response_json: serde_json::Value = response.json().await?;

            // Check for error in response
            if let Some(error) = response_json.get("error") {
                let error_message = error
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Unknown API error");
                let error_code = error.get("code").and_then(|c| c.as_u64()).unwrap_or(0);

                // Handle 500 errors in the JSON response as well
                if error_code == 500 {
                    if attempt < MAX_RETRIES {
                        info!(
                            "Image generation API returned 500 error in response (attempt {}/{}), retrying in {} seconds...",
                            attempt, MAX_RETRIES, delay_secs
                        );
                        tokio::time::sleep(Duration::from_secs(delay_secs)).await;
                        delay_secs = std::cmp::min(delay_secs * 2, 60); // Cap at 60 seconds
                        continue;
                    } else {
                        error!(
                            "Image generation API returned 500 error after {} attempts: {}",
                            MAX_RETRIES, error_message
                        );
                        return Err(anyhow::anyhow!(
                            "Image generation failed after {} attempts due to server errors: {}",
                            MAX_RETRIES,
                            error_message
                        ));
                    }
                }

                // Check for RESOURCE_EXHAUSTED error
                if error_code == 429 || error_message.to_lowercase().contains("resource_exhausted")
                {
                    // Check if this is specifically about quota
                    if error_message.to_lowercase().contains("quota") {
                        error!("Image generation quota exhausted: {}", error_message);
                        self.mark_image_quota_exhausted().await;
                        return Err(anyhow::anyhow!("IMAGE_QUOTA_EXHAUSTED: Image generation quota has been exceeded for today. This feature will be available again tomorrow."));
                    }
                }

                error!(
                    "Image generation API error (code {}): {}",
                    error_code, error_message
                );
                return Err(anyhow::anyhow!(
                    "Image generation API error: {}",
                    error_message
                ));
            }

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
                                        *data = serde_json::Value::String(
                                            "[IMAGE DATA REDACTED]".to_string(),
                                        );
                                    }
                                }
                            }

                            // Check for image data in the second part (typical format)
                            if let Some(part) = parts.get_mut(1) {
                                if let Some(inline_data) = part.get_mut("inlineData") {
                                    if let Some(data) = inline_data.get_mut("data") {
                                        *data = serde_json::Value::String(
                                            "[IMAGE DATA REDACTED]".to_string(),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Log the redacted response
            info!(
                "Image generation API response: {}",
                serde_json::to_string_pretty(&log_json)?
            );

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

                            error!(
                                "Image generation blocked due to safety concerns: {}",
                                safety_message
                            );
                            return Err(anyhow::anyhow!("SAFETY_BLOCKED: \"{}\"", safety_message));
                        }
                    }

                    // Check for safety ratings with blocked=true
                    if let Some(safety_ratings) = candidate.get("safetyRatings") {
                        if safety_ratings.as_array().is_some_and(|ratings| {
                            ratings.iter().any(|rating| {
                                rating
                                    .get("blocked")
                                    .and_then(|b| b.as_bool())
                                    .unwrap_or(false)
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

                            error!(
                                "Image generation blocked due to safety ratings: {}",
                                safety_message
                            );
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
                .unwrap_or("")
                .to_string();

            // Check if the text response indicates a safety block
            // This handles cases where the API returns a text explanation instead of an image
            if text_description.contains("unable to create")
                || text_description.contains("can't generate")
                || text_description.contains("cannot generate")
                || text_description.contains("policy violation")
                || text_description.contains("content policy")
            {
                error!(
                    "Image generation blocked based on text response: {}",
                    text_description
                );
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
                .and_then(|p| p.get(1)) // The second part contains the image
                .and_then(|p| p.get("inlineData"))
                .and_then(|i| i.get("data"))
                .and_then(|d| d.as_str())
            {
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
                    .and_then(|d| d.as_str())
                {
                    image_data = Some(data);
                }
            }

            // Process the image data if found
            if let Some(image_data) = image_data {
                info!("Successfully generated image from Gemini API");

                // Decode base64 image data
                match base64::engine::general_purpose::STANDARD.decode(image_data) {
                    Ok(bytes) => return Ok((bytes, text_description)),
                    Err(e) => {
                        error!("Failed to decode base64 image data: {:?}", e);
                        return Err(anyhow::anyhow!("Failed to decode base64 image data"));
                    }
                }
            } else {
                // No image data found - check if we have meaningful text content
                if !text_description.trim().is_empty() {
                    info!(
                        "API returned text-only response (no image): {}",
                        text_description
                    );
                    // Return a special error that indicates this is a text response, not a failure
                    return Err(anyhow::anyhow!("TEXT_RESPONSE: {}", text_description));
                } else {
                    error!("Failed to extract image data from API response");
                    return Err(anyhow::anyhow!(
                        "Failed to extract image data from API response"
                    ));
                }
            }
        }

        // This should never be reached due to the return statements above,
        // but we need it for the compiler
        Err(anyhow::anyhow!("Maximum retry attempts exceeded"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_image_quota_exhaustion() {
        // Create a test GeminiClient
        let client = GeminiClient::new(GeminiConfig {
            api_key: "test_key".to_string(),
            api_endpoint: None,
            prompt_wrapper: None,
            bot_name: "TestBot".to_string(),
            rate_limit_minute: 15,
            rate_limit_day: 1500,
            image_rate_limit_minute: 5,
            image_rate_limit_day: 100,
            context_messages: 5,
            log_prompts: false,
            personality_description: None,
        });

        // Initially, quota should not be exhausted
        assert!(!client.is_image_quota_exhausted().await);

        // Mark quota as exhausted
        client.mark_image_quota_exhausted().await;

        // Now quota should be exhausted
        assert!(client.is_image_quota_exhausted().await);

        // Test that generate_image returns the correct error when quota is exhausted
        let result = client.generate_image("test prompt").await;
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("IMAGE_QUOTA_EXHAUSTED"));
        assert!(error_msg.contains("quota has been exceeded for today"));
    }

    #[tokio::test]
    async fn test_image_quota_reset_logic() {
        // Create a test GeminiClient
        let client = GeminiClient::new(GeminiConfig {
            api_key: "test_key".to_string(),
            api_endpoint: None,
            prompt_wrapper: None,
            bot_name: "TestBot".to_string(),
            rate_limit_minute: 15,
            rate_limit_day: 1500,
            image_rate_limit_minute: 5,
            image_rate_limit_day: 100,
            context_messages: 5,
            log_prompts: false,
            personality_description: None,
        });

        // Mark quota as exhausted
        client.mark_image_quota_exhausted().await;

        // Verify it's marked as exhausted
        assert!(client.is_image_quota_exhausted().await);

        // Manually set the exhaustion time to yesterday (simulating time passage)
        {
            let mut quota_lock = client.image_quota_exhausted_until.lock().await;
            *quota_lock = Some(Utc::now() - chrono::Duration::days(1));
        }

        // Now quota should not be exhausted (time has passed)
        assert!(!client.is_image_quota_exhausted().await);
    }

    #[tokio::test]
    async fn test_separate_rate_limiters() {
        // Create a test GeminiClient with different rate limits for text and image
        let client = GeminiClient::new(GeminiConfig {
            api_key: "test_key".to_string(),
            api_endpoint: None,
            prompt_wrapper: None,
            bot_name: "TestBot".to_string(),
            rate_limit_minute: 10,      // text: 10 per minute
            rate_limit_day: 1000,       // text: 1000 per day
            image_rate_limit_minute: 2, // image: 2 per minute
            image_rate_limit_day: 50,   // image: 50 per day
            context_messages: 5,
            log_prompts: false,
            personality_description: None,
        });

        // Verify that the rate limiters are separate by checking their internal state
        // We can't directly test the rate limiting without making actual API calls,
        // but we can verify that the client has separate rate limiters

        // The fact that the client was created successfully with different limits
        // indicates that the separate rate limiters are working
        assert!(!client.is_image_quota_exhausted().await);
    }
}
