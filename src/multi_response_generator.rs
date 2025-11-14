use crate::gemini_api::GeminiClient;
use anyhow::Result;
use serde_json::Value;
use tracing::{error, info};

/// Configuration for multi-response generation
#[derive(Clone)]
pub struct MultiResponseConfig {
    pub num_responses: usize,
    pub quality_threshold: f32,
}

impl Default for MultiResponseConfig {
    fn default() -> Self {
        Self {
            num_responses: 4,
            quality_threshold: 6.0, // Out of 10
        }
    }
}

/// A response candidate with its rating
#[derive(Debug, Clone)]
pub struct ResponseCandidate {
    pub content: String,
    pub rating: f32,
    pub reasoning: String,
}

/// Multi-response generator that creates multiple responses and selects the best one
pub struct MultiResponseGenerator {
    gemini_client: GeminiClient,
    config: MultiResponseConfig,
}

impl MultiResponseGenerator {
    pub fn new(gemini_client: GeminiClient, config: MultiResponseConfig) -> Self {
        Self {
            gemini_client,
            config,
        }
    }

    /// Generate multiple responses and select the best one
    pub async fn generate_best_response(&self, prompt: &str) -> Result<Option<String>> {
        // Step 1: Generate multiple responses
        let responses = self.generate_multiple_responses(prompt).await?;

        if responses.is_empty() {
            return Ok(None);
        }

        // Step 2: Rate all responses
        let rated_responses = self.rate_responses(&responses, prompt).await?;

        // Step 3: Select the best response
        self.select_best_response(rated_responses).await
    }

    /// Generate multiple response candidates
    async fn generate_multiple_responses(&self, prompt: &str) -> Result<Vec<String>> {
        let generation_prompt = format!(
            "{}\n\nGenerate {} different response options. Each response should be on its own line, numbered 1-{}. Make each response unique in style and approach while staying true to the character.",
            prompt,
            self.config.num_responses,
            self.config.num_responses
        );

        match self
            .gemini_client
            .generate_content(&generation_prompt)
            .await
        {
            Ok(response) => {
                let responses = self.parse_numbered_responses(&response);
                info!("Generated {} response candidates", responses.len());
                Ok(responses)
            }
            Err(e) => {
                error!("Failed to generate multiple responses: {:?}", e);
                Err(e)
            }
        }
    }

    /// Parse numbered responses from the API output
    fn parse_numbered_responses(&self, text: &str) -> Vec<String> {
        let mut responses = Vec::new();

        for line in text.lines() {
            let line = line.trim();
            // Look for numbered responses (1., 2., etc.)
            if let Some(pos) = line.find('.') {
                if let Ok(_num) = line[..pos].parse::<usize>() {
                    let response = line[pos + 1..].trim();
                    if !response.is_empty() {
                        responses.push(response.to_string());
                    }
                }
            }
        }

        responses
    }

    /// Rate multiple responses for quality and appropriateness
    async fn rate_responses(
        &self,
        responses: &[String],
        original_prompt: &str,
    ) -> Result<Vec<ResponseCandidate>> {
        let rating_prompt = format!(
            "You are evaluating response quality for a Discord bot. Rate each response on a scale of 1-10 based on:\n\
            - Relevance to the conversation\n\
            - Natural conversational flow\n\
            - Humor/entertainment value (if appropriate)\n\
            - Character consistency\n\
            - Avoiding repetitive patterns\n\n\
            Original prompt context: {}\n\n\
            Responses to rate:\n{}\n\n\
            Provide your ratings in JSON format:\n\
            {{\n\
              \"ratings\": [\n\
                {{\"response_index\": 1, \"rating\": 7.5, \"reasoning\": \"Good humor but slightly forced\"}},\n\
                {{\"response_index\": 2, \"rating\": 8.2, \"reasoning\": \"Natural and relevant\"}}\n\
              ]\n\
            }}",
            original_prompt,
            responses.iter().enumerate()
                .map(|(i, r)| format!("{}. {}", i + 1, r))
                .collect::<Vec<_>>()
                .join("\n")
        );

        match self.gemini_client.generate_content(&rating_prompt).await {
            Ok(rating_response) => self.parse_ratings(&rating_response, responses).await,
            Err(e) => {
                error!("Failed to rate responses: {:?}", e);
                // Fallback: return all responses with neutral ratings
                Ok(responses
                    .iter()
                    .enumerate()
                    .map(|(i, response)| ResponseCandidate {
                        content: response.clone(),
                        rating: 5.0,
                        reasoning: format!("Fallback rating for response {}", i + 1),
                    })
                    .collect())
            }
        }
    }

    /// Parse rating JSON response
    async fn parse_ratings(
        &self,
        rating_text: &str,
        responses: &[String],
    ) -> Result<Vec<ResponseCandidate>> {
        // Try to extract JSON from the response
        let json_start = rating_text.find('{');
        let json_end = rating_text.rfind('}');

        if let (Some(start), Some(end)) = (json_start, json_end) {
            let json_str = &rating_text[start..=end];

            match serde_json::from_str::<Value>(json_str) {
                Ok(json) => {
                    let mut candidates = Vec::new();

                    if let Some(ratings) = json["ratings"].as_array() {
                        for rating in ratings {
                            if let (Some(index), Some(score), Some(reasoning)) = (
                                rating["response_index"].as_u64(),
                                rating["rating"].as_f64(),
                                rating["reasoning"].as_str(),
                            ) {
                                let idx = (index as usize).saturating_sub(1);
                                if idx < responses.len() {
                                    candidates.push(ResponseCandidate {
                                        content: responses[idx].clone(),
                                        rating: score as f32,
                                        reasoning: reasoning.to_string(),
                                    });
                                }
                            }
                        }
                    }

                    if !candidates.is_empty() {
                        return Ok(candidates);
                    }
                }
                Err(e) => {
                    error!("Failed to parse rating JSON: {:?}", e);
                }
            }
        }

        // Fallback: return all responses with neutral ratings
        Ok(responses
            .iter()
            .enumerate()
            .map(|(i, response)| ResponseCandidate {
                content: response.clone(),
                rating: 5.0,
                reasoning: format!("Fallback rating for response {}", i + 1),
            })
            .collect())
    }

    /// Select the best response based on ratings and threshold
    async fn select_best_response(
        &self,
        mut candidates: Vec<ResponseCandidate>,
    ) -> Result<Option<String>> {
        if candidates.is_empty() {
            return Ok(None);
        }

        // Sort by rating (highest first)
        candidates.sort_by(|a, b| {
            b.rating
                .partial_cmp(&a.rating)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let best = &candidates[0];

        info!(
            "Best response rated {:.1}/10: {} (Reason: {})",
            best.rating,
            best.content.chars().take(50).collect::<String>(),
            best.reasoning
        );

        // Check if the best response meets the quality threshold
        if best.rating >= self.config.quality_threshold {
            Ok(Some(best.content.clone()))
        } else {
            info!(
                "Best response rating {:.1} below threshold {:.1}, passing",
                best.rating, self.config.quality_threshold
            );
            Ok(None)
        }
    }

    /// Generate best response with context (for interjections)
    pub async fn generate_best_response_with_context(
        &self,
        prompt: &str,
        context: &[(String, String, Option<String>, String)],
    ) -> Result<Option<String>> {
        // For context-based responses, we'll use a simpler approach
        // Generate the response normally first, then evaluate it
        match self
            .gemini_client
            .generate_response_with_context_and_pronouns(prompt, "", context, None)
            .await
        {
            Ok(response) => {
                let response = response.trim();

                // If it's already a "pass", return None
                if response.to_lowercase() == "pass" {
                    return Ok(None);
                }

                // Evaluate the single response
                let evaluation = self.evaluate_single_response(response, prompt).await?;

                if evaluation >= self.config.quality_threshold {
                    info!("Response rated {:.1}/10, using it", evaluation);
                    Ok(Some(response.to_string()))
                } else {
                    info!(
                        "Response rated {:.1}/10, below threshold {:.1}, passing",
                        evaluation, self.config.quality_threshold
                    );
                    Ok(None)
                }
            }
            Err(e) => {
                error!("Failed to generate response with context: {:?}", e);
                Err(e)
            }
        }
    }

    /// Evaluate a single response for quality
    async fn evaluate_single_response(&self, response: &str, original_prompt: &str) -> Result<f32> {
        let evaluation_prompt = format!(
            "Rate this Discord bot response on a scale of 1-10 based on:\n\
            - Relevance and naturalness\n\
            - Humor/entertainment value (if appropriate)\n\
            - Character consistency\n\
            - Avoiding repetitive patterns\n\n\
            Original prompt: {}\n\
            Response: {}\n\n\
            Respond with just a number between 1.0 and 10.0",
            original_prompt, response
        );

        match self
            .gemini_client
            .generate_content(&evaluation_prompt)
            .await
        {
            Ok(rating_text) => {
                // Try to extract a number from the response
                let rating_str = rating_text.trim();
                if let Ok(rating) = rating_str.parse::<f32>() {
                    Ok(rating.clamp(1.0, 10.0))
                } else {
                    // Try to find a number in the text
                    for word in rating_str.split_whitespace() {
                        if let Ok(rating) = word.parse::<f32>() {
                            return Ok(rating.clamp(1.0, 10.0));
                        }
                    }
                    // Fallback to neutral rating
                    Ok(5.0)
                }
            }
            Err(_) => {
                // Fallback to neutral rating on error
                Ok(5.0)
            }
        }
    }
}
