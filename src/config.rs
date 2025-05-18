use anyhow::{Context as AnyhowContext, Result};
use serde::Deserialize;
use std::fs;
use std::path::Path;
use tracing::info;

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_preprocess_config_content() {
        let input = r#"
# Test config
DiScOrD_ToKeN = "test_token"
BOT_NAME = "TestCrow"
MESSAGE_HISTORY_LIMIT = "5000"
"#;
        
        let processed = preprocess_config_content(input);
        println!("Processed content: {}", processed);
        
        // Check that keys are converted to lowercase
        assert!(processed.contains("discord_token"));
        assert!(processed.contains("bot_name"));
        assert!(processed.contains("message_history_limit"));
        
        // Check that values are preserved
        assert!(processed.contains("\"test_token\""));
        assert!(processed.contains("\"TestCrow\""));
        assert!(processed.contains("\"5000\""));
        
        // Check that comments are preserved
        assert!(processed.contains("# Test config"));
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub discord_token: String,
    pub followed_channel_name: Option<String>,
    pub followed_channel_id: Option<String>,
    pub followed_channel_names: Option<String>,
    pub followed_channel_ids: Option<String>,
    pub followed_server_name: Option<String>,
    pub bot_name: Option<String>,
    pub message_history_limit: Option<String>,
    pub db_trim_interval_secs: Option<String>,
    pub gemini_rate_limit_minute: Option<String>,
    pub gemini_rate_limit_day: Option<String>,
    pub gemini_api_key: Option<String>,
    pub gemini_api_endpoint: Option<String>,
    pub gemini_prompt_wrapper: Option<String>,
    pub gemini_interjection_prompt: Option<String>,
    pub gemini_context_messages: Option<String>,
    pub interjection_mst3k_probability: Option<String>,
    pub interjection_memory_probability: Option<String>,
    pub interjection_pondering_probability: Option<String>,
    pub interjection_ai_probability: Option<String>,
    // thinking_message removed - only using typing indicator
    pub google_search_enabled: Option<String>,
    pub db_host: Option<String>,
    pub db_name: Option<String>,
    pub db_user: Option<String>,
    pub db_password: Option<String>,
    pub gateway_bot_ids: Option<String>,
}

pub fn load_config() -> Result<Config> {
    let config_path = Path::new("CrowConfig.toml");
    
    if config_path.exists() {
        let config_content = fs::read_to_string(config_path)
            .context("Failed to read CrowConfig.toml")?;
        
        // Pre-process the config content to make keys case-insensitive
        let processed_content = preprocess_config_content(&config_content);
        
        let config: Config = toml::from_str(&processed_content)
            .context("Failed to parse CrowConfig.toml")?;
        
        return Ok(config);
    }
    
    Err(anyhow::anyhow!("Configuration file CrowConfig.toml not found"))
}

// Helper function to make config keys case-insensitive
pub fn preprocess_config_content(content: &str) -> String {
    let mut processed = String::new();
    
    for line in content.lines() {
        let trimmed = line.trim();
        
        // Skip comments and empty lines
        if trimmed.starts_with('#') || trimmed.is_empty() {
            processed.push_str(line);
            processed.push('\n');
            continue;
        }
        
        // Check if this line contains a key-value pair
        if let Some(equals_pos) = trimmed.find('=') {
            let key = &trimmed[..equals_pos].trim();
            let value = &trimmed[equals_pos..];
            
            // Convert key to lowercase
            let lowercase_key = key.to_lowercase();
            processed.push_str(&lowercase_key);
            processed.push_str(value);
        } else {
            // Not a key-value pair, keep as is (section headers, etc.)
            processed.push_str(line);
        }
        
        processed.push('\n');
    }
    
    processed
}

pub fn parse_config(config: &Config) -> (
    String,                 // bot_name
    usize,                  // message_history_limit
    u64,                    // db_trim_interval
    u32,                    // gemini_rate_limit_minute
    u32,                    // gemini_rate_limit_day
    Vec<u64>,               // gateway_bot_ids
    bool,                   // google_search_enabled
    usize,                  // gemini_context_messages
    f64,                    // interjection_mst3k_probability
    f64,                    // interjection_memory_probability
    f64,                    // interjection_pondering_probability
    f64                     // interjection_ai_probability
) {
    // Get the bot name
    let bot_name = config.bot_name.clone().unwrap_or_else(|| "Crow".to_string());
    
    // Get the message history limit
    let message_history_limit = config.message_history_limit
        .as_ref()
        .and_then(|limit| limit.parse::<usize>().ok())
        .unwrap_or(10000);
    
    info!("Message history limit set to {}", message_history_limit);
    
    // Get database trim interval (default: 1 hour)
    let db_trim_interval = config.db_trim_interval_secs
        .as_ref()
        .and_then(|interval| interval.parse::<u64>().ok())
        .unwrap_or(3600); // Default: 1 hour
    
    info!("Database trim interval set to {} seconds", db_trim_interval);
    
    // Get Gemini API rate limits
    let gemini_rate_limit_minute = config.gemini_rate_limit_minute
        .as_ref()
        .and_then(|limit| limit.parse::<u32>().ok())
        .unwrap_or(15); // Default: 15 calls per minute
    
    let gemini_rate_limit_day = config.gemini_rate_limit_day
        .as_ref()
        .and_then(|limit| limit.parse::<u32>().ok())
        .unwrap_or(1500); // Default: 1500 calls per day
    
    info!("Gemini API rate limits set to {} calls per minute and {} calls per day", 
          gemini_rate_limit_minute, gemini_rate_limit_day);
    
    // Parse gateway bot IDs
    let gateway_bot_ids = config.gateway_bot_ids
        .as_ref()
        .map(|ids_str| {
            ids_str.split(',')
                .filter_map(|id_str| {
                    let trimmed = id_str.trim();
                    match trimmed.parse::<u64>() {
                        Ok(id) => Some(id),
                        Err(_) => {
                            info!("Invalid gateway bot ID: {}", trimmed);
                            None
                        }
                    }
                })
                .collect::<Vec<u64>>()
        })
        .unwrap_or_else(Vec::new);
    
    if !gateway_bot_ids.is_empty() {
        info!("Will respond to {} gateway bots: {:?}", gateway_bot_ids.len(), gateway_bot_ids);
    } else {
        info!("No gateway bots configured, will ignore all bot messages");
    }
    
    // Parse Google search enabled flag (default: true for backward compatibility)
    let google_search_enabled = config.google_search_enabled
        .as_ref()
        .and_then(|enabled| {
            match enabled.to_lowercase().as_str() {
                "false" | "0" | "no" | "disabled" | "off" => Some(false),
                "true" | "1" | "yes" | "enabled" | "on" => Some(true),
                _ => {
                    info!("Invalid google_search_enabled value: {}, defaulting to enabled", enabled);
                    Some(true)
                }
            }
        })
        .unwrap_or(true); // Default to enabled for backward compatibility
        
    // Parse number of context messages to include in Gemini API calls
    let gemini_context_messages = config.gemini_context_messages
        .as_ref()
        .and_then(|count| count.parse::<usize>().ok())
        .unwrap_or(5); // Default: 5 messages
        
    info!("Gemini API context messages set to {}", gemini_context_messages);
    
    // Parse interjection probabilities
    let interjection_mst3k_probability = config.interjection_mst3k_probability
        .as_ref()
        .and_then(|prob| prob.parse::<f64>().ok())
        .unwrap_or(0.005); // Default: 0.5% chance (1 in 200)
        
    let interjection_memory_probability = config.interjection_memory_probability
        .as_ref()
        .and_then(|prob| prob.parse::<f64>().ok())
        .unwrap_or(0.005); // Default: 0.5% chance (1 in 200)
        
    let interjection_pondering_probability = config.interjection_pondering_probability
        .as_ref()
        .and_then(|prob| prob.parse::<f64>().ok())
        .unwrap_or(0.005); // Default: 0.5% chance (1 in 200)
        
    let interjection_ai_probability = config.interjection_ai_probability
        .as_ref()
        .and_then(|prob| prob.parse::<f64>().ok())
        .unwrap_or(0.005); // Default: 0.5% chance (1 in 200)
        
    info!("Interjection probabilities: MST3K: {}, Memory: {}, Pondering: {}, AI: {}", 
          interjection_mst3k_probability, 
          interjection_memory_probability,
          interjection_pondering_probability,
          interjection_ai_probability);
    
    info!("Google search feature is {}", if google_search_enabled { "enabled" } else { "disabled" });
          
    (
        bot_name,
        message_history_limit,
        db_trim_interval,
        gemini_rate_limit_minute,
        gemini_rate_limit_day,
        gateway_bot_ids,
        google_search_enabled,
        gemini_context_messages,
        interjection_mst3k_probability,
        interjection_memory_probability,
        interjection_pondering_probability,
        interjection_ai_probability
    )
}
