use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use anyhow::Result;
use serenity::all::*;
use serenity::async_trait;
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::model::id::{ChannelId, MessageId};
use serenity::prelude::*;
use serenity::builder::{CreateMessage, GetMessages};
use serenity::model::channel::MessageReference;
use tokio::sync::RwLock;
use tracing::{debug, error, info};
use tokio_rusqlite::Connection;
use rand::seq::SliceRandom;
use rand::Rng;

// Import modules
mod db_utils;
mod config;
mod database;
mod response_timing;
mod google_search;
mod gemini_api;
mod crime_fighting;
mod rate_limiter;
mod frinkiac;
mod morbotron;
mod masterofallscience;
mod trump_insult;
mod display_name;
mod buzz;
mod lastseen;
mod image_generation;
mod news_interjection;
mod news_search;
mod enhanced_frinkiac_search;
mod enhanced_morbotron_search;
mod enhanced_masterofallscience_search;
mod fill_silence;
mod fact_interjection;
mod screenshot_search_utils;
mod text_formatting;
mod screenshot_search_common;
mod prompt_templates;
mod utils;

// Helper function to check if a response looks like a prompt
fn is_prompt_echo(response: &str) -> bool {
    response.contains("{bot_name}") || 
    response.contains("{user}") || 
    response.contains("{message}") || 
    response.contains("{context}") ||
    response.contains("You should ONLY respond with an interjection if") ||
    response.contains("For criterion #2") ||
    response.contains("Guidelines for your fact") ||
    response.contains("If you can't think of a good fact")
}
mod regex_substitution;
mod bandname;
mod mst3k_quotes;
mod unknown_command;
mod celebrity_status;

// Use our modules
use config::{load_config, parse_config};
use database::DatabaseManager;
use google_search::GoogleSearchClient;
use gemini_api::GeminiClient;
use crime_fighting::CrimeFightingGenerator;
use news_interjection::handle_news_interjection;
use frinkiac::{FrinkiacClient, handle_frinkiac_command};
use morbotron::{MorbotronClient, handle_morbotron_command};
use response_timing::apply_realistic_delay;
use masterofallscience::{MasterOfAllScienceClient, handle_masterofallscience_command};
use display_name::{get_best_display_name, clean_display_name};
use buzz::handle_buzz_command;
use lastseen::handle_lastseen_command;
use image_generation::handle_imagine_command;
use regex_substitution::handle_regex_substitution;
use mst3k_quotes::fallback_mst3k_quote;
use celebrity_status::handle_aliveordead_command;
use unknown_command::handle_unknown_command;

// Define keys for the client data
struct RecentSpeakersKey;
impl TypeMapKey for RecentSpeakersKey {
    type Value = Arc<RwLock<VecDeque<(String, String)>>>;  // (username, display_name)
}

struct MessageHistoryKey;
impl TypeMapKey for MessageHistoryKey {
    type Value = Arc<RwLock<VecDeque<Message>>>;
}

struct Bot {
    followed_channels: Vec<ChannelId>,
    db_manager: DatabaseManager,
    google_client: Option<GoogleSearchClient>,
    gemini_client: Option<GeminiClient>,
    frinkiac_client: FrinkiacClient,
    morbotron_client: MorbotronClient,
    masterofallscience_client: MasterOfAllScienceClient,
    bot_name: String,
    message_db: Option<Arc<tokio::sync::Mutex<Connection>>>,
    message_history_limit: usize,
    commands: HashMap<String, String>,
    keyword_triggers: Vec<(Vec<String>, String)>,
    crime_generator: CrimeFightingGenerator,
    trump_insult_generator: trump_insult::TrumpInsultGenerator,
    band_genre_generator: bandname::BandGenreGenerator,
    gateway_bot_ids: Vec<u64>,
    google_search_enabled: bool,
    gemini_interjection_prompt: Option<String>,
    imagine_channels: Vec<String>,
    start_time: Instant,
    #[allow(dead_code)]
    gemini_context_messages: usize,
    #[allow(dead_code)]
    interjection_mst3k_probability: f64,
    #[allow(dead_code)]
    interjection_memory_probability: f64,
    #[allow(dead_code)]
    interjection_pondering_probability: f64,
    #[allow(dead_code)]
    interjection_ai_probability: f64,
    interjection_fact_probability: f64,
    interjection_news_probability: f64,
    fill_silence_manager: Arc<fill_silence::FillSilenceManager>,
    // Track the last seen message timestamp for each channel
    last_seen_message: Arc<RwLock<HashMap<ChannelId, (serenity::model::Timestamp, MessageId)>>>,
}

impl Bot {
    // Check for missed messages after reconnection
    async fn check_missed_messages(&self, ctx: &Context) {
        info!("Checking for missed messages after reconnection...");
        
        // Get the last seen messages
        let last_seen = self.last_seen_message.read().await.clone();
        
        // For each followed channel
        for channel_id in &self.followed_channels {
            // If we have a last seen message for this channel
            if let Some((_, last_message_id)) = last_seen.get(channel_id) {
                info!("Checking for missed messages in channel {} since message ID {}", channel_id, last_message_id);
                
                // Get messages after the last seen message
                let retriever = GetMessages::default();
                let retriever = retriever.after(*last_message_id).limit(50);
                
                match channel_id.messages(&ctx.http, retriever).await {
                    Ok(messages) => {
                        if !messages.is_empty() {
                            info!("Found {} missed messages in channel {}", messages.len(), channel_id);
                            
                            // Process each missed message in chronological order (oldest first)
                            for msg in messages.iter().rev() {
                                // Skip our own messages
                                if msg.author.id == ctx.http.get_current_user().await.map(|u| u.id).unwrap_or_default() {
                                    continue;
                                }
                                
                                info!("Processing missed message from {}: {}", msg.author.name, msg.content);
                                
                                // Process the message
                                if let Err(e) = self.process_message(ctx, msg).await {
                                    error!("Error processing missed message: {:?}", e);
                                }
                                
                                // Update the last seen message
                                {
                                    let mut last_seen = self.last_seen_message.write().await;
                                    last_seen.insert(*channel_id, (msg.timestamp, msg.id));
                                }
                            }
                        } else {
                            info!("No missed messages in channel {}", channel_id);
                        }
                    },
                    Err(e) => {
                        error!("Error retrieving missed messages for channel {}: {:?}", channel_id, e);
                    }
                }
            } else {
                info!("No last seen message for channel {}, skipping missed message check", channel_id);
            }
        }
    }
    
    // Helper function to mark the bot as the last speaker in a channel
    // NOTE: This method is currently unused but kept for future reference
    async fn _mark_as_last_speaker(&self, channel_id: ChannelId) {
        self.fill_silence_manager.mark_bot_as_last_speaker(channel_id).await;
    }
    
    // Helper function to save bot's own response to the database
    #[allow(dead_code)]
    async fn save_bot_response(&self, response: &str) {
        if let Some(db) = &self.message_db {
            if let Err(e) = db_utils::save_message(
                db.clone(),
                &self.bot_name,
                &self.bot_name,
                response,
                None, // No Message object for our own response
                None  // No operation ID
            ).await {
                error!("Error saving bot response to database: {:?}", e);
            } else {
                debug!("Saved bot response to database for context");
            }
        }
    }

    fn new(
        followed_channels: Vec<ChannelId>,
        mysql_host: Option<String>,
        mysql_db: Option<String>,
        mysql_user: Option<String>,
        mysql_password: Option<String>,
        _google_api_key: Option<String>,         // Unused but kept for compatibility
        _google_search_engine_id: Option<String>, // Unused but kept for compatibility
        gemini_api_key: Option<String>,
        gemini_api_endpoint: Option<String>,
        gemini_prompt_wrapper: Option<String>,
        gemini_interjection_prompt: Option<String>,
        bot_name: String,
        message_db: Option<Arc<tokio::sync::Mutex<Connection>>>,
        message_history_limit: usize,
        gateway_bot_ids: Vec<u64>,
        google_search_enabled: bool,
        gemini_rate_limit_minute: u32,
        gemini_rate_limit_day: u32,
        gemini_context_messages: usize,
        interjection_mst3k_probability: f64,
        interjection_memory_probability: f64,
        interjection_pondering_probability: f64,
        interjection_ai_probability: f64,
        interjection_fact_probability: f64,
        interjection_news_probability: f64,
        log_prompts: bool,
        imagine_channels: Vec<String>,
        fill_silence_enabled: bool,
        fill_silence_start_hours: f64,
        fill_silence_max_hours: f64,
        gemini_personality_description: Option<String>
    ) -> Self {
        // Define the commands the bot will respond to
        let mut commands = HashMap::new();
        commands.insert("hello".to_string(), "world!".to_string());
        
        // Generate a comprehensive help message with all commands
        let help_message = if !imagine_channels.is_empty() {
            // Include the imagine command if channels are configured
            "Available commands:\n!help - Show help\n!hello - Say hello\n!buzz - Generate corporate buzzwords\n!fightcrime - Generate a crime fighting duo\n!trump - Generate a Trump insult\n!bandname [name] - Generate music genre for a band\n!lastseen [name] - Find when a user was last active\n!quote [term] - Get a random quote\n!quote -show [show] - Get quote from specific show\n!quote -dud [user] - Get random message from a user\n!slogan [term] - Get a random advertising slogan\n!frinkiac [term] [-s season] [-e episode] - Get a Simpsons screenshot\n!morbotron [term] - Get a Futurama screenshot\n!masterofallscience [term] - Get a Rick and Morty screenshot\n!imagine [text] - Generate an image\n!alive [name] - Check if a celebrity is alive or dead\n!info - Show bot statistics"
        } else {
            // Exclude the imagine command if no channels are configured
            "Available commands:\n!help - Show help\n!hello - Say hello\n!buzz - Generate corporate buzzwords\n!fightcrime - Generate a crime fighting duo\n!trump - Generate a Trump insult\n!bandname [name] - Generate music genre for a band\n!lastseen [name] - Find when a user was last active\n!quote [term] - Get a random quote\n!quote -show [show] - Get quote from specific show\n!quote -dud [user] - Get random message from a user\n!slogan [term] - Get a random advertising slogan\n!frinkiac [term] [-s season] [-e episode] - Get a Simpsons screenshot\n!morbotron [term] - Get a Futurama screenshot\n!masterofallscience [term] - Get a Rick and Morty screenshot\n!alive [name] - Check if a celebrity is alive or dead\n!info - Show bot statistics"
        };
        
        commands.insert("help".to_string(), help_message.to_string());
        
        // Define keyword triggers - empty but we keep the structure for future additions
        let keyword_triggers = Vec::new();
        // We handle exact phrase matches separately in the message processing logic
        
        // Create database manager
        let db_manager = DatabaseManager::new(mysql_host.clone(), mysql_db.clone(), mysql_user.clone(), mysql_password.clone());
        info!("Database manager created, is configured: {}", db_manager.is_configured());
        
        // Create Google search client if feature is enabled
        let google_client = if google_search_enabled {
            info!("Creating Google search client for web scraping");
            Some(GoogleSearchClient::new())
        } else {
            info!("Google search feature is disabled in configuration");
            None
        };
        
        // Create Gemini client if API key is provided
        let gemini_client = match gemini_api_key {
            Some(api_key) => {
                info!("Creating Gemini client with provided API key");
                info!("Gemini rate limits: {} per minute, {} per day", gemini_rate_limit_minute, gemini_rate_limit_day);
                Some(GeminiClient::new(
                    api_key,
                    gemini_api_endpoint,
                    gemini_prompt_wrapper,
                    bot_name.clone(),
                    gemini_rate_limit_minute,
                    gemini_rate_limit_day,
                    gemini_context_messages,
                    log_prompts,
                    gemini_personality_description
                ))
            },
            None => {
                info!("Gemini client not created - missing API key");
                None
            }
        };
        
        // Create crime fighting generator
        let crime_generator = CrimeFightingGenerator::new();
        
        // Create Frinkiac client
        let frinkiac_client = FrinkiacClient::new();
        
        // Create Morbotron client
        let morbotron_client = MorbotronClient::new();
        
        // Create MasterOfAllScience client
        let masterofallscience_client = MasterOfAllScienceClient::new();
        
        // Create Trump insult generator
        let trump_insult_generator = trump_insult::TrumpInsultGenerator::new();
        
        // Create Band genre generator
        let band_genre_generator = bandname::BandGenreGenerator::new();
        
        // Initialize the fill silence manager
        let fill_silence_manager = Arc::new(fill_silence::FillSilenceManager::new(
            fill_silence_enabled,
            fill_silence_start_hours,
            fill_silence_max_hours
        ));
        
        Self {
            followed_channels,
            db_manager,
            google_client,
            gemini_client,
            frinkiac_client,
            morbotron_client,
            masterofallscience_client,
            bot_name,
            message_db,
            message_history_limit,
            commands,
            keyword_triggers,
            crime_generator,
            trump_insult_generator,
            band_genre_generator,
            gateway_bot_ids,
            google_search_enabled,
            gemini_interjection_prompt,
            imagine_channels,
            start_time: Instant::now(),
            gemini_context_messages,
            interjection_mst3k_probability,
            interjection_memory_probability,
            interjection_pondering_probability,
            interjection_ai_probability,
            interjection_fact_probability,
            interjection_news_probability,
            fill_silence_manager,
            last_seen_message: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    // Add this method to check the database connection at startup
    async fn check_database_connection(&self) -> Result<()> {
        info!("Checking database connection...");
        if !self.db_manager.is_configured() {
            error!("❌ Database manager is not configured. Check your database credentials in CrowConfig.toml");
            return Ok(());
        }
        
        info!("✅ Database manager is configured");
        
        // Test the connection
        match self.db_manager.test_connection() {
            Ok(true) => info!("✅ Database connection test passed"),
            Ok(false) => error!("❌ Database connection test failed"),
            Err(e) => error!("❌ Error testing database connection: {:?}", e),
        }
        
        Ok(())
    }
    
    // Format a duration into a human-readable string
    fn format_duration(duration: Duration) -> String {
        let total_seconds = duration.as_secs();
        let days = total_seconds / 86400;
        let hours = (total_seconds % 86400) / 3600;
        let minutes = (total_seconds % 3600) / 60;
        let seconds = total_seconds % 60;
        
        if days > 0 {
            format!("{}d {}h {}m {}s", days, hours, minutes, seconds)
        } else if hours > 0 {
            format!("{}h {}m {}s", hours, minutes, seconds)
        } else if minutes > 0 {
            format!("{}m {}s", minutes, seconds)
        } else {
            format!("{}s", seconds)
        }
    }
    
    // Handle the !info command
    async fn handle_info_command(&self, ctx: &Context, msg: &Message) -> Result<()> {
        // Calculate uptime
        let uptime = self.start_time.elapsed();
        let uptime_str = Self::format_duration(uptime);
        
        // Get message history count
        let message_count = if let Some(db) = &self.message_db {
            match db.lock().await.call(|conn| {
                let mut stmt = conn.prepare("SELECT COUNT(*) FROM messages")?;
                let count: i64 = stmt.query_row([], |row| row.get(0))?;
                Ok::<_, rusqlite::Error>(count)
            }).await {
                Ok(count) => count.to_string(),
                Err(_) => "Unknown".to_string(),
            }
        } else {
            "Database not available".to_string()
        };
        
        // Get memory usage (approximate)
        let memory_usage = match std::process::Command::new("ps")
            .args(&["-o", "rss=", "-p", &std::process::id().to_string()])
            .output() {
                Ok(output) => {
                    let rss = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if rss.is_empty() {
                        "Unknown".to_string()
                    } else {
                        // Convert from KB to MB
                        match rss.parse::<f64>() {
                            Ok(kb) => format!("{:.2} MB", kb / 1024.0),
                            Err(_) => "Unknown".to_string(),
                        }
                    }
                },
                Err(_) => "Unknown".to_string(),
            };
        
        // Count followed channels
        let channel_count = self.followed_channels.len();
        
        // Build the info message
        let mut info = format!(
            "**{} Bot Info**\n\n", 
            self.bot_name
        );
        
        info.push_str(&format!("**Uptime:** {}\n", uptime_str));
        info.push_str(&format!("**Messages in database:** {}\n", message_count));
        info.push_str(&format!("**Memory usage:** {}\n", memory_usage));
        info.push_str(&format!("**Following {} channels**\n", channel_count));
        
        // Add feature status
        info.push_str("\n**Features:**\n");
        info.push_str(&format!("- Google search: {}\n", if self.google_search_enabled { "Enabled" } else { "Disabled" }));
        info.push_str(&format!("- AI responses: {}\n", if self.gemini_client.is_some() { "Enabled" } else { "Disabled" }));
        info.push_str(&format!("- Image generation: {}\n", if !self.imagine_channels.is_empty() { 
            format!("Enabled in {} channels", self.imagine_channels.len()) 
        } else { 
            "Disabled".to_string() 
        }));
        
        // Send the info message
        if let Err(e) = msg.channel_id.say(&ctx.http, info).await {
            error!("Error sending info message: {:?}", e);
        }
        
        Ok(())
    }
    
    // Generate a crime fighting duo description
    async fn generate_crime_fighting_duo(&self, ctx: &Context, msg: &Message) -> Result<String> {
        // Try to get the list of recent speakers, but use defaults if anything fails
        let data = ctx.data.read().await;
        
        // Default names to use if we can't get real speakers
        let default_speaker1 = "The Mysterious Stranger".to_string();
        let default_speaker2 = "The Unknown Vigilante".to_string();
        
        // Get the username of the person who invoked the command
        let invoker_username = msg.author.name.clone();
        
        // Try to get real speaker names, but fall back to defaults at any error
        let (speaker1, speaker2) = match data.get::<RecentSpeakersKey>() {
            Some(recent_speakers_lock) => {
                match recent_speakers_lock.try_read() {
                    Ok(recent_speakers) => {
                        // Only log speakers at debug level
                        if tracing::level_enabled!(tracing::Level::DEBUG) {
                            let all_speakers: Vec<String> = recent_speakers.iter().map(|(_, display)| display.clone()).collect();
                            debug!("All speakers before filtering: {:?}", all_speakers);
                        }
                        
                        // Log the raw speakers list to help debug
                        info!("Raw speakers list: {:?}", recent_speakers.iter().map(|(name, display)| format!("{}:{}", name, display)).collect::<Vec<String>>());
                        
                        // Filter out the invoker from the list of potential speakers
                        let filtered_speakers: Vec<(String, String)> = recent_speakers
                            .iter()
                            .filter(|(username, _)| username != &invoker_username)
                            .cloned()
                            .collect();
                        
                        // Only log filtered speakers at debug level
                        if tracing::level_enabled!(tracing::Level::DEBUG) {
                            let filtered_names: Vec<String> = filtered_speakers.iter().map(|(_, display)| display.clone()).collect();
                            debug!("Filtered speakers (excluding invoker): {:?}", filtered_names);
                        }
                        
                        if filtered_speakers.len() >= 2 {
                            // Get the last two speakers (most recent first)
                            // Since VecDeque is ordered with most recent at the back,
                            // we take the last two elements from our filtered list
                            let last_idx = filtered_speakers.len() - 1;
                            let second_last_idx = filtered_speakers.len() - 2;
                            
                            let speaker1_name = filtered_speakers[last_idx].1.clone();
                            let speaker2_name = filtered_speakers[second_last_idx].1.clone();
                            
                            info!("Selected speakers: {} and {}", speaker1_name, speaker2_name);
                            
                            // Use display names of the last two speakers who aren't the invoker
                            (speaker1_name, speaker2_name)
                        } else if filtered_speakers.len() == 1 && recent_speakers.len() >= 2 {
                            // If we have only one filtered speaker but at least two total speakers,
                            // use the filtered speaker and the invoker
                            let speaker1_name = filtered_speakers[0].1.clone();
                            let speaker2_name = get_best_display_name(ctx, msg).await;
                            
                            info!("Using one filtered speaker and invoker: {} and {}", speaker1_name, speaker2_name);
                            
                            (speaker1_name, speaker2_name)
                        } else {
                            info!("Not enough speakers excluding invoker, using default names for crime fighting duo");
                            (default_speaker1, default_speaker2)
                        }
                    },
                    Err(e) => {
                        error!("Could not read recent speakers lock, using default names: {:?}", e);
                        (default_speaker1, default_speaker2)
                    }
                }
            },
            None => {
                error!("RecentSpeakersKey not found in context data, using default names");
                (default_speaker1, default_speaker2)
            }
        };
        
        // Use our crime fighting generator
        self.crime_generator.generate_duo(&speaker1, &speaker2)
    }
}

impl Bot {
    // Handle the !slogan command
    async fn handle_slogan_command(&self, http: &Http, msg: &Message, search_term: Option<String>) -> Result<()> {
        // Log the slogan request
        if let Some(term) = &search_term {
            info!("Slogan request with search term: {}", term);
        } else {
            info!("Slogan request with no search term");
        }
        
        self.db_manager.query_random_entry(http, msg, search_term, None, "slogan").await
    }
    
    // Handle the !quote command
    async fn handle_quote_command(&self, http: &Http, msg: &Message, args: Vec<&str>) -> Result<()> {
        // Parse arguments
        let mut search_term = None;
        let mut show_name = None;
        
        let mut i = 0;
        while i < args.len() {
            if args[i] == "-show" && i + 1 < args.len() {
                // Collect all words after -show until the next flag or end
                let mut show = Vec::new();
                i += 1;
                while i < args.len() && !args[i].starts_with('-') {
                    show.push(args[i]);
                    i += 1;
                }
                show_name = Some(show.join(" "));
            } else if !args[i].starts_with('-') {
                // If not a flag, treat as search term
                if search_term.is_none() {
                    let mut terms = Vec::new();
                    while i < args.len() && !args[i].starts_with('-') {
                        terms.push(args[i]);
                        i += 1;
                    }
                    search_term = Some(terms.join(" "));
                } else {
                    i += 1;
                }
            } else {
                // Skip unknown flags
                i += 1;
            }
        }
        
        // Log the quote request
        if let Some(term) = &search_term {
            info!("Quote request with search term: {}", term);
        } else {
            info!("Quote request with no search term");
        }
        
        if let Some(show) = &show_name {
            info!("Quote request filtered by show: {}", show);
        }
        
        // Pass both search term and show name to the database manager
        self.db_manager.query_random_entry(http, msg, search_term, show_name, "quote").await
    }
    
    // Handle the !quote -dud command (quote a user)
    async fn handle_quote_dud_command(&self, http: &Http, msg: &Message, username: Option<String>) -> Result<()> {
        // Check if we have a database connection
        if let Some(db) = &self.message_db {
            let db_clone = db.clone();
            
            // Build the query based on whether a username was provided
            let messages = if let Some(user) = &username {
                let user_clone = user.clone();
                info!("Quote -dud request for user: {}", user_clone);
                
                // Query the database for messages from this user
                db_clone.lock().await.call(move |conn| {
                    // First check if display_name column exists
                    let has_display_name = conn
                        .prepare("PRAGMA table_info(messages)")
                        .and_then(|mut stmt| {
                            let rows = stmt.query_map([], |row| {
                                let name: String = row.get(1)?;
                                Ok(name)
                            })?;
                            
                            for name_result in rows {
                                if let Ok(name) = name_result {
                                    if name == "display_name" {
                                        return Ok(true);
                                    }
                                }
                            }
                            Ok(false)
                        })
                        .unwrap_or(false);
                    
                    let mut result = Vec::new();
                    
                    if has_display_name {
                        let query = "SELECT author, display_name, content FROM messages WHERE author = ? OR display_name LIKE ? ORDER BY RANDOM() LIMIT 1";
                        let mut stmt = conn.prepare(query)?;
                        let search_pattern = format!("%{}%", &user_clone);
                        let rows = stmt.query_map([&user_clone, &search_pattern], |row| {
                            Ok((
                                row.get::<_, String>(0)?, 
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?
                            ))
                        })?;
                        
                        for row in rows {
                            result.push(row?);
                        }
                    } else {
                        let query = "SELECT author, author as display_name, content FROM messages WHERE author = ? ORDER BY RANDOM() LIMIT 1";
                        let mut stmt = conn.prepare(query)?;
                        let rows = stmt.query_map([&user_clone], |row| {
                            Ok((
                                row.get::<_, String>(0)?, 
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?
                            ))
                        })?;
                        
                        for row in rows {
                            result.push(row?);
                        }
                    }
                    
                    Ok::<_, rusqlite::Error>(result)
                }).await?
            } else {
                info!("Quote -dud request for random user");
                
                // Query the database for a random message from any user
                db_clone.lock().await.call(move |conn| {
                    // First check if display_name column exists
                    let has_display_name = conn
                        .prepare("PRAGMA table_info(messages)")
                        .and_then(|mut stmt| {
                            let rows = stmt.query_map([], |row| {
                                let name: String = row.get(1)?;
                                Ok(name)
                            })?;
                            
                            for name_result in rows {
                                if let Ok(name) = name_result {
                                    if name == "display_name" {
                                        return Ok(true);
                                    }
                                }
                            }
                            Ok(false)
                        })
                        .unwrap_or(false);
                    
                    let query = if has_display_name {
                        "SELECT author, display_name, content FROM messages ORDER BY RANDOM() LIMIT 1"
                    } else {
                        "SELECT author, author as display_name, content FROM messages ORDER BY RANDOM() LIMIT 1"
                    };
                    
                    let mut stmt = conn.prepare(query)?;
                    
                    let rows = stmt.query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?
                        ))
                    })?;
                    
                    let mut result = Vec::new();
                    for row in rows {
                        result.push(row?);
                    }
                    
                    Ok::<_, rusqlite::Error>(result)
                }).await?
            };
            
            // If we found a message, send it
            if let Some((author, display_name, content)) = messages.first() {
                // Use the display name if available, otherwise fall back to author name
                let name_to_use = if !display_name.is_empty() {
                    display_name
                } else {
                    author
                };
                
                // Use the display_name::clean_display_name function for consistency
                // This will also strip angle brackets if the name is in gateway format
                let clean_display_name = display_name::clean_display_name(name_to_use);
                
                msg.channel_id.say(http, format!("<{}> {}", clean_display_name, content)).await?;
            } else {
                // No messages found
                if let Some(user) = username {
                    msg.channel_id.say(http, format!("No messages found from user {}", user)).await?;
                } else {
                    msg.channel_id.say(http, "No messages found in the database").await?;
                }
            }
        } else {
            // No database connection
            msg.channel_id.say(http, "Message history database is not available").await?;
        }
        
        Ok(())
    }
}
impl Bot {
    // Function to check if the bot is being addressed
    fn is_bot_addressed(&self, content: &str) -> bool {
        let bot_name = &self.bot_name.to_lowercase();
        let content_lower = content.to_lowercase();
        
        // Direct mention at the start - the message must start with the bot's name
        // followed by a space, punctuation, or end of string
        if content_lower.starts_with(bot_name) {
            // Check what comes after the bot name
            let remainder = &content_lower[bot_name.len()..];
            if remainder.is_empty() || remainder.starts_with(' ') || 
               remainder.starts_with('?') || remainder.starts_with('!') || 
               remainder.starts_with(',') || remainder.starts_with(':') {
                info!("Bot addressed: name at beginning of message");
                return true;
            }
        }
        
        // Common address patterns - these are explicit ways to address the bot
        let address_patterns = [
            format!("hey {}", bot_name),
            format!("hi {}", bot_name),
            format!("hello {}", bot_name),
            format!("ok {}", bot_name),
            format!("hey, {}", bot_name),
            format!("hi, {}", bot_name),
            format!("hello, {}", bot_name),
            format!("ok, {}", bot_name),
            format!("{}, ", bot_name),     // When name is used with a comma
            format!("@{}", bot_name),      // Informal mention
            format!("excuse me, {}", bot_name),
            format!("by the way, {}", bot_name),
            format!("btw, {}", bot_name),
        ];
        
        for pattern in &address_patterns {
            if content_lower.contains(pattern) {
                info!("Bot addressed: matched pattern '{}'", pattern);
                return true;
            }
        }
        
        // We don't have direct access to the bot's ID here, so we'll rely on other methods
        // to detect mentions. The actual mention detection happens in the message handler
        // where we check if the bot is mentioned in the message.
        
        // Use regex with word boundaries to avoid false positives
        // This prevents matching when the bot name is part of another word
        let name_with_word_boundary = format!(r"\b{}\b", regex::escape(bot_name));
        if let Ok(re) = regex::Regex::new(&name_with_word_boundary) {
            if re.is_match(&content_lower) {
                // The bot name appears as a complete word
                
                // Check for negative patterns - these are phrases where the bot name appears
                // but is not being directly addressed
                let negative_patterns = [
                    format!(r"than {}\b", bot_name),   // "other than Crow"
                    format!(r"like {}\b", bot_name),   // "like Crow"
                    format!(r"about {}\b", bot_name),  // "about Crow"
                    format!(r"with {}\b", bot_name),   // "with Crow"
                    format!(r"and {}\b", bot_name),    // "and Crow"
                    format!(r"or {}\b", bot_name),     // "or Crow"
                    format!(r"for {}\b", bot_name),    // "for Crow"
                    format!(r"the {}\b", bot_name),    // "the Crow"
                    format!(r"a {}\b", bot_name),      // "a Crow"
                    format!(r"an {}\b", bot_name),     // "an Crow"
                    format!(r"this {}\b", bot_name),   // "this Crow"
                    format!(r"that {}\b", bot_name),   // "that Crow"
                    format!(r"my {}\b", bot_name),     // "my Crow"
                    format!(r"your {}\b", bot_name),   // "your Crow"
                    format!(r"our {}\b", bot_name),    // "our Crow"
                    format!(r"their {}\b", bot_name),  // "their Crow"
                    format!(r"his {}\b", bot_name),    // "his Crow"
                    format!(r"her {}\b", bot_name),    // "her Crow"
                    format!(r"its {}\b", bot_name),    // "its Crow"
                    format!(r"picked {}\b", bot_name), // "picked Crow"
                    format!(r"chose {}\b", bot_name),  // "chose Crow"
                    format!(r"selected {}\b", bot_name), // "selected Crow"
                    format!(r"named {}\b", bot_name),  // "named Crow"
                    format!(r"called {}\b", bot_name), // "called Crow"
                    format!(r"{} is\b", bot_name),     // "Crow is"
                    format!(r"{} was\b", bot_name),    // "Crow was"
                    format!(r"{} has\b", bot_name),    // "Crow has"
                    format!(r"{} isn't\b", bot_name),  // "Crow isn't"
                    format!(r"{} doesn't\b", bot_name), // "Crow doesn't"
                    format!(r"{} didn't\b", bot_name), // "Crow didn't"
                    format!(r"{} won't\b", bot_name),  // "Crow won't"
                    format!(r"{} can't\b", bot_name),  // "Crow can't"
                    // Additional negative patterns for rhyming and comparison cases
                    format!(r"{} rhymes", bot_name),   // "Crow rhymes"
                    format!(r"rhymes with {}", bot_name), // "rhymes with Crow"
                    format!(r"{} and", bot_name),      // "Crow and"
                    format!(r"more of a {}", bot_name), // "more of a Crow"
                    format!(r"less of a {}", bot_name), // "less of a Crow"
                    format!(r"kind of {}", bot_name),  // "kind of Crow"
                    format!(r"sort of {}", bot_name),  // "sort of Crow"
                    format!(r"type of {}", bot_name),  // "type of Crow"
                ];
                
                for pattern in &negative_patterns {
                    if let Ok(re) = regex::Regex::new(pattern) {
                        if re.is_match(&content_lower) {
                            info!("Bot NOT addressed: matched negative pattern '{}'", pattern);
                            return false;
                        }
                    }
                }
                
                // Check for positive patterns - these are phrases that directly address the bot
                let positive_patterns = [
                    format!(r"{}\?", bot_name),        // "Crow?"
                    format!(r"{}!", bot_name),         // "Crow!"
                    format!(r"{},", bot_name),         // "Crow,"
                    format!(r"{}:", bot_name),         // "Crow:"
                    format!(r"{} can you", bot_name),  // "Crow can you"
                    format!(r"{} could you", bot_name), // "Crow could you"
                    format!(r"{} will you", bot_name), // "Crow will you"
                    format!(r"{} would you", bot_name), // "Crow would you"
                    format!(r"{} please", bot_name),   // "Crow please"
                    format!(r"ask {}", bot_name),      // "ask Crow"
                    format!(r"tell {}", bot_name),     // "tell Crow"
                    format!(r", {}", bot_name),        // ", Crow" - for cases like "No you weren't, Crow"
                    format!(r" {}\.", bot_name),       // " Crow." - for cases ending with the bot's name
                ];
                
                for pattern in &positive_patterns {
                    if let Ok(re) = regex::Regex::new(&format!(r"\b{}\b", pattern)) {
                        if re.is_match(&content_lower) {
                            info!("Bot addressed: matched positive pattern '{}'", pattern);
                            return true;
                        }
                    }
                }
                
                // If the bot name is at the beginning or end of the message, it's likely being addressed
                if content_lower.trim().starts_with(bot_name) || content_lower.trim().ends_with(bot_name) {
                    info!("Bot addressed: name at beginning or end of trimmed message");
                    return true;
                }
                
                // If we've made it this far, the bot name is used as a standalone word
                // but doesn't match our positive or negative patterns
                // We'll be conservative and assume it's NOT being addressed
                info!("Bot name found as standalone word, but not clearly addressed");
                return false;
            }
        }
        
        false
    }
    
    // Process a message
    async fn process_message(&self, ctx: &Context, msg: &Message) -> Result<()> {
        // Note: Message is already stored in the database in the message() event handler
        // No need to store it again here
        
        // Update the in-memory message history
        let data = ctx.data.read().await;
        if let Some(message_history) = data.get::<MessageHistoryKey>() {
            let mut history = message_history.write().await;
            if history.len() >= self.message_history_limit {
                history.pop_front();
            }
            history.push_back(msg.clone());
        }
        
        // IMPORTANT: Process all explicit triggers first, before any random interjections
        
        // Check for commands (messages starting with !)
        if msg.content.starts_with('!') {
            let parts: Vec<&str> = msg.content[1..].split_whitespace().collect();
            if !parts.is_empty() {
                let command = parts[0].to_lowercase();
                
                if command == "hello" {
                    // Simple hello command
                    if let Err(e) = msg.channel_id.say(&ctx.http, "world!").await {
                        error!("Error sending hello response: {:?}", e);
                    }
                } else if command == "trump" {
                    // Generate a Trump insult
                    let insult = self.trump_insult_generator.generate_insult();
                    if let Err(e) = msg.channel_id.say(&ctx.http, insult).await {
                        error!("Error sending Trump insult: {:?}", e);
                    }
                } else if command == "bandname" {
                    // Generate a band genre
                    if parts.len() > 1 {
                        let band_name = parts[1..].join(" ");
                        let genre = self.band_genre_generator.generate_genre(&band_name);
                        if let Err(e) = msg.channel_id.say(&ctx.http, genre).await {
                            error!("Error sending band genre: {:?}", e);
                        }
                    } else {
                        if let Err(e) = msg.reply(&ctx.http, "Please provide a band name.").await {
                            error!("Error sending usage message: {:?}", e);
                        }
                    }
                } else if command == "imagine" && !self.imagine_channels.is_empty() {
                    // Extract the image prompt
                    if parts.len() > 1 {
                        let prompt = parts[1..].join(" ");
                        if let Some(gemini_client) = &self.gemini_client {
                            if let Err(e) = handle_imagine_command(ctx, msg, gemini_client, &prompt, &self.imagine_channels).await {
                                error!("Error handling imagine command: {:?}", e);
                            }
                        } else {
                            if let Err(e) = msg.reply(&ctx.http, "Sorry, image generation is not available (Gemini API not configured).").await {
                                error!("Error sending API not configured message: {:?}", e);
                            }
                        }
                    } else {
                        if let Err(e) = msg.reply(&ctx.http, "Please provide a description of what you want me to show you.").await {
                            error!("Error sending usage message: {:?}", e);
                        }
                    }
                } else if command == "alive" {
                    // Check if a celebrity name was provided
                    if parts.len() > 1 {
                        let celebrity_name = parts[1..].join(" ");
                        if let Err(e) = handle_aliveordead_command(&ctx.http, &msg, &celebrity_name).await {
                            error!("Error handling alive command: {:?}", e);
                            if let Err(e) = msg.channel_id.say(&ctx.http, "Error checking celebrity status").await {
                                error!("Error sending error message: {:?}", e);
                            }
                        }
                    } else {
                        if let Err(e) = msg.reply(&ctx.http, "Please provide a celebrity name.").await {
                            error!("Error sending usage message: {:?}", e);
                        }
                    }
                } else if command == "help" {
                    // Help command - use the help message from our commands HashMap
                    if let Some(help_text) = self.commands.get("help") {
                        if let Err(e) = msg.channel_id.say(&ctx.http, help_text).await {
                            error!("Error sending help message: {:?}", e);
                        }
                    }
                } else if command == "info" {
                    // Handle the info command
                    if let Err(e) = self.handle_info_command(ctx, msg).await {
                        error!("Error handling info command: {:?}", e);
                    }
                } else if command == "slogan" {
                    // Extract search term if provided
                    let search_term = if parts.len() > 1 {
                        Some(parts[1..].join(" "))
                    } else {
                        None
                    };
                    
                    // Generate a slogan response
                    if let Err(e) = self.handle_slogan_command(&ctx.http, &msg, search_term).await {
                        error!("Error handling slogan command: {:?}", e);
                        if let Err(e) = msg.channel_id.say(&ctx.http, "Error accessing slogan database").await {
                            error!("Error sending error message: {:?}", e);
                        }
                    }
                } else if command == "quote" {
                    // Extract all arguments after the command
                    let args: Vec<&str> = if parts.len() > 1 {
                        parts[1..].to_vec()
                    } else {
                        Vec::new()
                    };
                    
                    // Check if this is a -dud request (quote a user)
                    if args.contains(&"-dud") {
                        let username_index = args.iter().position(|&r| r == "-dud").unwrap() + 1;
                        let username = if username_index < args.len() {
                            Some(args[username_index].to_string())
                        } else {
                            None
                        };
                        
                        if let Err(e) = self.handle_quote_dud_command(&ctx.http, &msg, username).await {
                            error!("Error handling quote -dud command: {:?}", e);
                            if let Err(e) = msg.channel_id.say(&ctx.http, "Error retrieving user quotes").await {
                                error!("Error sending error message: {:?}", e);
                            }
                        }
                    } else {
                        // Regular quote command with possible -show flag
                        if let Err(e) = self.handle_quote_command(&ctx.http, &msg, args).await {
                            error!("Error handling quote command: {:?}", e);
                            if let Err(e) = msg.channel_id.say(&ctx.http, "Error accessing quote database").await {
                                error!("Error sending error message: {:?}", e);
                            }
                        }
                    }
                } else if command == "fightcrime" {
                    match self.generate_crime_fighting_duo(&ctx, &msg).await {
                        Ok(duo) => {
                            if let Err(e) = msg.channel_id.say(&ctx.http, duo).await {
                                error!("Error sending crime fighting duo: {:?}", e);
                            }
                        },
                        Err(e) => {
                            error!("Error handling fightcrime command: {:?}", e);
                            if let Err(e) = msg.channel_id.say(&ctx.http, "Error generating crime fighting duo").await {
                                error!("Error sending error message: {:?}", e);
                            }
                        }
                    }
                } else if command == "buzz" {
                    // Handle the buzz command
                    if let Err(e) = handle_buzz_command(&ctx.http, &msg).await {
                        error!("Error handling buzz command: {:?}", e);
                        if let Err(e) = msg.channel_id.say(&ctx.http, "Error generating buzzword").await {
                            error!("Error sending error message: {:?}", e);
                        }
                    }
                } else if command == "lastseen" {
                    // Extract name to search for
                    let name = if parts.len() > 1 {
                        parts[1..].join(" ")
                    } else {
                        String::new()
                    };
                    
                    // Handle the lastseen command
                    if let Err(e) = handle_lastseen_command(&ctx.http, &msg, &name, &self.message_db).await {
                        error!("Error handling lastseen command: {:?}", e);
                        if let Err(e) = msg.channel_id.say(&ctx.http, "Error searching message history").await {
                            error!("Error sending error message: {:?}", e);
                        }
                    }
                } else if command == "frinkiac" {
                    // Extract search term if provided
                    let args = if parts.len() > 1 {
                        Some(parts[1..].join(" "))
                    } else {
                        None
                    };
                    
                    // Handle the frinkiac command
                    if let Err(e) = handle_frinkiac_command(
                        &ctx.http, 
                        &msg, 
                        args, 
                        &self.frinkiac_client,
                        self.gemini_client.as_ref()
                    ).await {
                        error!("Error handling frinkiac command: {:?}", e);
                        if let Err(e) = msg.channel_id.say(&ctx.http, "Error searching Frinkiac").await {
                            error!("Error sending error message: {:?}", e);
                        }
                    }
                } else if command == "morbotron" {
                    // Extract search term if provided
                    let search_term = if parts.len() > 1 {
                        Some(parts[1..].join(" "))
                    } else {
                        None
                    };
                    
                    // Handle the morbotron command
                    if let Err(e) = handle_morbotron_command(
                        &ctx.http, 
                        &msg, 
                        search_term, 
                        &self.morbotron_client,
                        self.gemini_client.as_ref()
                    ).await {
                        error!("Error handling morbotron command: {:?}", e);
                        if let Err(e) = msg.channel_id.say(&ctx.http, "Error searching Morbotron").await {
                            error!("Error sending error message: {:?}", e);
                        }
                    }
                } else if command == "masterofallscience" {
                    // Extract search term if provided
                    let search_term = if parts.len() > 1 {
                        Some(parts[1..].join(" "))
                    } else {
                        None
                    };
                    
                    // Handle the masterofallscience command
                    if let Err(e) = handle_masterofallscience_command(
                        &ctx.http, 
                        &msg, 
                        search_term, 
                        &self.masterofallscience_client,
                        self.gemini_client.as_ref()
                    ).await {
                        error!("Error handling masterofallscience command: {:?}", e);
                        if let Err(e) = msg.channel_id.say(&ctx.http, "Error searching Master of All Science").await {
                            error!("Error sending error message: {:?}", e);
                        }
                    }
                } else if let Some(response) = self.commands.get(&command) {
                    if let Err(e) = msg.channel_id.say(&ctx.http, response).await {
                        error!("Error sending command response: {:?}", e);
                    }
                } else if let Some(gemini_client) = &self.gemini_client {
                    // Handle unknown command with Gemini API
                    if let Err(e) = handle_unknown_command(&ctx.http, &msg, &command, gemini_client, ctx).await {
                        error!("Error handling unknown command: {:?}", e);
                    }
                }
            }
            return Ok(());
        }
        
        // Check for Google search (messages starting with "google")
        if self.google_search_enabled && msg.content.to_lowercase().starts_with("google ") && msg.content.len() > 7 {
            let query = &msg.content[7..];
            
            if let Some(google_client) = &self.google_client {
                if let Err(e) = msg.channel_id.say(&ctx.http, format!("Searching for: {}", query)).await {
                    error!("Error sending search confirmation: {:?}", e);
                }
                
                // Perform the search
                match google_client.search(query).await {
                    Ok(Some(result)) => {
                        // Clean up the title by removing extra whitespace
                        let title = result.title.trim().replace("\n", " ").replace("  ", " ");
                        
                        // Format and send the result
                        let response = format!("**{}**\n{}\n{}", title, result.url, result.snippet);
                        if let Err(e) = msg.channel_id.say(&ctx.http, response).await {
                            error!("Error sending search result: {:?}", e);
                        }
                    },
                    Ok(None) => {
                        if let Err(e) = msg.channel_id.say(&ctx.http, "No search results found.").await {
                            error!("Error sending no results message: {:?}", e);
                        }
                    },
                    Err(e) => {
                        error!("Error performing search: {:?}", e);
                        if let Err(e) = msg.channel_id.say(&ctx.http, "Error performing search.").await {
                            error!("Error sending error message: {:?}", e);
                        }
                    }
                }
            } else {
                if let Err(e) = msg.channel_id.say(&ctx.http, "Search is not configured.").await {
                    error!("Error sending search error: {:?}", e);
                }
            }
            return Ok(());
        }
        
        // Check if the bot is being addressed using our new function
        if self.is_bot_addressed(&msg.content) {
            // Use the full message content including the bot's name
            let content = msg.content.trim().to_string();
            let content_lower = content.to_lowercase();
            
            // Check if the message contains "who fights crime" when the bot is addressed
            if content_lower.contains("who fights crime") {
                info!("Bot addressed with 'who fights crime' question");
                match self.generate_crime_fighting_duo(&ctx, &msg).await {
                    Ok(duo) => {
                        if let Err(e) = msg.channel_id.say(&ctx.http, duo).await {
                            error!("Error sending crime fighting duo: {:?}", e);
                        }
                        return Ok(());
                    },
                    Err(e) => {
                        error!("Error generating crime fighting duo: {:?}", e);
                        // Continue with normal response if crime fighting duo generation fails
                    }
                }
            }
            
            if !content.is_empty() {
                if let Some(gemini_client) = &self.gemini_client {
                    // Get and clean the display name
                    let display_name = get_best_display_name(ctx, msg).await;
                    let clean_display_name = clean_display_name(&display_name);
                    
                    // Extract pronouns from the original display name
                    let display_name = get_best_display_name(ctx, msg).await;
                    let user_pronouns = crate::display_name::extract_pronouns(&display_name);
                    
                    // Start typing indicator before making API call
                    if let Err(e) = msg.channel_id.broadcast_typing(&ctx.http).await {
                        error!("Failed to send typing indicator: {:?}", e);
                    }
                    
                    // Get recent messages for context
                    let context_messages = if let Some(db) = &self.message_db {
                        // Get the last self.gemini_context_messages messages from the database
                        match db_utils::get_recent_messages(db.clone(), self.gemini_context_messages, Some(msg.channel_id.to_string().as_str())).await {
                            Ok(messages) => messages,
                            Err(e) => {
                                error!("Error retrieving recent messages: {:?}", e);
                                Vec::new()
                            }
                        }
                    } else {
                        Vec::new()
                    };
                    
                    // Call the Gemini API with context and pronouns
                    match gemini_client.generate_response_with_context(
                        &content, 
                        &clean_display_name, 
                        &context_messages,
                        user_pronouns.as_deref()
                    ).await {
                        Ok(response) => {
                            // Apply realistic typing delay based on response length
                            apply_realistic_delay(&response, ctx, msg.channel_id).await;
                            
                            // Create a message reference for replying
                            let message_reference = MessageReference::from(msg);
                            let create_message = CreateMessage::new()
                                .content(response.clone())
                                .reference_message(message_reference);
                            
                            if let Err(e) = msg.channel_id.send_message(&ctx.http, create_message).await {
                                error!("Error sending Gemini response as reply: {:?}", e);
                                // Fallback to regular message if reply fails
                                if let Err(e) = msg.channel_id.say(&ctx.http, response).await {
                                    error!("Error sending fallback Gemini response: {:?}", e);
                                }
                            }
                        },
                        Err(e) => {
                            error!("Error calling Gemini API: {:?}", e);
                            
                            // Create a message reference for replying
                            let message_reference = MessageReference::from(msg);
                            let create_message = CreateMessage::new()
                                .content(format!("Sorry, I encountered an error: {}", e))
                                .reference_message(message_reference);
                            
                            if let Err(e) = msg.channel_id.send_message(&ctx.http, create_message).await {
                                error!("Error sending error message as reply: {:?}", e);
                                // Fallback to regular message if reply fails
                                if let Err(e) = msg.channel_id.say(&ctx.http, format!("Sorry, I encountered an error: {}", e)).await {
                                    error!("Error sending fallback error message: {:?}", e);
                                }
                            }
                        }
                    }
                } else {
                    // No Gemini API configured, use a simple response
                    if let Err(e) = msg.reply(&ctx.http, "Sorry, I'm not configured to respond to messages yet.").await {
                        error!("Error sending simple response: {:?}", e);
                    }
                }
                return Ok(());
            }
        }
        
        // Now process random interjections only if no explicit triggers were matched
        
        // Get the current user (bot) ID
        let current_user_id = ctx.http.get_current_user().await.map(|user| user.id).unwrap_or_else(|_| UserId::new(0));
        
        // Get the probability multiplier based on channel inactivity
        let silence_multiplier = self.fill_silence_manager.get_probability_multiplier(msg.channel_id, current_user_id).await;
        
        // MST3K Quote interjection
        let adjusted_mst3k_probability = self.interjection_mst3k_probability * silence_multiplier;
        if rand::thread_rng().gen_bool(adjusted_mst3k_probability) {
            let probability_percent = self.interjection_mst3k_probability * 100.0;
            let adjusted_percent = adjusted_mst3k_probability * 100.0;
            let odds = if self.interjection_mst3k_probability > 0.0 {
                format!("1 in {:.0}", 1.0 / self.interjection_mst3k_probability)
            } else {
                "disabled".to_string()
            };
            
            info!("Triggered MST3K quote interjection (base: {:.2}% chance, adjusted: {:.2}%, silence multiplier: {:.2}x, {})", 
                  probability_percent, adjusted_percent, silence_multiplier, odds);
            
            // Try to get a quote from the database first
            if self.db_manager.is_configured() {
                // Use a separate task to query the database
                let db_manager = self.db_manager.clone();
                
                info!("Attempting to query database for MST3K quotes");
                
                let task_result = tokio::task::spawn(async move {
                    // Query for MST3K quotes specifically
                    let mut result = None;
                    
                    // Get a connection from the pool
                    if let Some(pool) = db_manager.pool.as_ref() {
                        info!("Got database pool for MST3K quotes");
                        if let Ok(mut conn) = pool.get_conn() {
                            info!("Successfully connected to database for MST3K quotes");
                            use mysql::prelude::Queryable;
                            
                            // Build the show clause for MST3K
                            let show_clause = "%MST3K%";  // Use the actual show name in the database
                            
                            // Count total matching quotes
                            let count_query = "SELECT COUNT(*) FROM masterlist_quotes, masterlist_episodes, masterlist_shows \
                                              WHERE masterlist_episodes.show_id = masterlist_shows.show_id \
                                              AND masterlist_quotes.show_id = masterlist_shows.show_id \
                                              AND masterlist_quotes.show_ep = masterlist_episodes.show_ep \
                                              AND show_title LIKE ?";
                            
                            info!("Executing count query for MST3K quotes with show_clause: {}", show_clause);
                            
                            match conn.exec_first::<i64, _, _>(count_query, (show_clause,)) {
                                Ok(Some(total_entries)) => {
                                    info!("Found {} MST3K quotes in database", total_entries);
                                    if total_entries > 0 {
                                        // Get a random quote
                                        let random_index = rand::thread_rng().gen_range(0..total_entries);
                                        info!("Selected random index {} of {} for MST3K quotes", random_index, total_entries);
                                        
                                        let select_query = "SELECT quote FROM masterlist_quotes, masterlist_episodes, masterlist_shows \
                                                           WHERE masterlist_episodes.show_id = masterlist_shows.show_id \
                                                           AND masterlist_quotes.show_id = masterlist_shows.show_id \
                                                           AND masterlist_quotes.show_ep = masterlist_episodes.show_ep \
                                                           AND show_title LIKE ? \
                                                           LIMIT ?, 1";
                                        
                                        info!("Executing select query for MST3K quote");
                                        match conn.exec_first::<String, _, _>(select_query, (show_clause, random_index)) {
                                            Ok(Some(quote_text)) => {
                                                info!("Successfully retrieved MST3K quote: {}", quote_text);
                                                // Clean up HTML entities
                                                let clean_quote = html_escape::decode_html_entities(&quote_text);
                                                
                                                // Extract a character name and their quote if possible
                                                if let Some(colon_pos) = clean_quote.find(':') {
                                                    if colon_pos > 0 && colon_pos < clean_quote.len() - 1 {
                                                        let character = clean_quote[0..colon_pos].trim();
                                                        let character_quote = clean_quote[colon_pos+1..].trim();
                                                        
                                                        // Only use if we have both a character and a quote
                                                        if !character.is_empty() && !character_quote.is_empty() {
                                                            info!("Extracted character '{}' and quote '{}'", character, character_quote);
                                                            result = Some((character.to_string(), character_quote.to_string()));
                                                        } else {
                                                            info!("Character or quote was empty after parsing");
                                                        }
                                                    } else {
                                                        info!("Colon position invalid: {}", colon_pos);
                                                    }
                                                } else {
                                                    info!("No colon found in quote: {}", clean_quote);
                                                }
                                                
                                                // If we couldn't extract a character quote, use the whole quote
                                                if result.is_none() {
                                                    info!("Using whole quote as MST3K quote");
                                                    result = Some(("MST3K".to_string(), clean_quote.to_string()));
                                                }
                                            },
                                            Ok(None) => {
                                                error!("No quote found at index {} despite count being {}", random_index, total_entries);
                                            },
                                            Err(e) => {
                                                error!("Error executing select query for MST3K quote: {:?}", e);
                                            }
                                        }
                                    } else {
                                        info!("No MST3K quotes found in database");
                                    }
                                },
                                Ok(None) => {
                                    error!("Count query returned None for MST3K quotes");
                                },
                                Err(e) => {
                                    error!("Error executing count query for MST3K quotes: {:?}", e);
                                }
                            }
                        } else {
                            error!("Failed to get database connection for MST3K quotes");
                        }
                    } else {
                        error!("Database pool is None for MST3K quotes");
                    }
                    
                    result
                }).await;
                
                if let Ok(Some((character, quote))) = task_result {
                    // Extract individual lines from the quote
                    // Format: "<Speaker> Line <Speaker> Line"
                    let re = regex::Regex::new(r"<([^>]+)>\s*([^<]+)").unwrap_or_else(|_| {
                        error!("Failed to compile regex for MST3K quote parsing");
                        regex::Regex::new(r".*").unwrap() // Fallback regex that matches everything
                    });
                    
                    // Find all speaker-line pairs
                    let mut lines = Vec::new();
                    for cap in re.captures_iter(&quote) {
                        if let (Some(_speaker), Some(line_match)) = (cap.get(1), cap.get(2)) {
                            let line = line_match.as_str().trim();
                            if !line.is_empty() {
                                lines.push(line.to_string());
                            }
                        }
                    }
                    
                    // If we found any lines, pick one randomly
                    let formatted_quote = if !lines.is_empty() {
                        lines.choose(&mut rand::thread_rng())
                            .unwrap_or(&quote)
                            .clone()
                    } else {
                        // If no lines were extracted, use the whole quote as fallback
                        quote
                    };
                    
                    // Send the quote
                    let quote_for_log = formatted_quote.clone(); // Clone for logging
                    if let Err(e) = msg.channel_id.say(&ctx.http, formatted_quote).await {
                        error!("Error sending MST3K database quote: {:?}", e);
                        
                        // Fall back to hardcoded quotes if sending fails
                        fallback_mst3k_quote(ctx, msg).await?;
                    } else {
                        info!("MST3K database quote sent: {} (from character: {})", quote_for_log, character);
                        return Ok(());
                    }
                } else {
                    // Fall back to hardcoded quotes if database query fails
                    fallback_mst3k_quote(ctx, msg).await?;
                }
            } else {
                // Database not configured, use hardcoded quotes
                fallback_mst3k_quote(ctx, msg).await?;
            }
        }
        
        // Memory interjection
        let adjusted_memory_probability = self.interjection_memory_probability * silence_multiplier;
        if rand::thread_rng().gen_bool(adjusted_memory_probability) {
            let probability_percent = self.interjection_memory_probability * 100.0;
            let adjusted_percent = adjusted_memory_probability * 100.0;
            let odds = if self.interjection_memory_probability > 0.0 {
                format!("1 in {:.0}", 1.0 / self.interjection_memory_probability)
            } else {
                "disabled".to_string()
            };
            
            info!("Triggered memory interjection (base: {:.2}% chance, adjusted: {:.2}%, silence multiplier: {:.2}x, {})", 
                  probability_percent, adjusted_percent, silence_multiplier, odds);
            
            if let (Some(db), Some(gemini_client)) = (&self.message_db, &self.gemini_client) {
                let db_clone = Arc::clone(db);
                
                // We'll start the typing indicator only after we decide to send a message
                
                // Query the database for a random message with minimum length of 20 characters
                let result = db_clone.lock().await.call(|conn| {
                    let query = "SELECT content, author, display_name FROM messages WHERE length(content) >= 20 ORDER BY RANDOM() LIMIT 1";
                    let mut stmt = conn.prepare(query)?;
                    
                    let rows = stmt.query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?
                        ))
                    })?;
                    
                    let mut result = Vec::new();
                    for row in rows {
                        result.push(row?);
                    }
                    
                    Ok::<_, rusqlite::Error>(result)
                }).await;
                
                // Get recent context from the channel
                let builder = serenity::builder::GetMessages::default().limit(3);
                let context = match msg.channel_id.messages(&ctx.http, builder).await {
                    Ok(messages) => messages,
                    Err(e) => {
                        error!("Error retrieving recent messages for memory context: {:?}", e);
                        Vec::new()
                    }
                };
                
                let context_text = context.iter()
                    .map(|m| format!("{}: {}", m.author.name, m.content))
                    .collect::<Vec<_>>()
                    .join("\n");
                
                match result {
                    Ok(messages) => {
                        if let Some((content, _, _)) = messages.first() {
                            let memory_prompt = format!(
                                "You are {}, a witty Discord bot. You've found this message in your memory: \"{}\". \
                                Here's what's currently being discussed:\n{}\n\n\
                                Please contribute to the conversation:\n\
                                1. Keep it short and natural\n\
                                2. Don't quote or reference the memory - just say what you want to say\n\
                                3. Don't identify yourself or explain what you're doing\n\
                                4. If you can't make it work naturally, respond with 'pass'\n\
                                5. Correct any obvious typos but preserve the message's character\n\
                                6. NEVER reference the movie \"Manos: The Hands of Fate\"\n\
                                7. NEVER make jokes about dating, relationships, or sexual topics\n\
                                8. ALWAYS use a person's correct pronouns when addressing or referring to them. If someone has specified their pronouns \
                                (e.g., in their username like \"name (she/her)\"), ALWAYS use those pronouns. If pronouns aren't specified, take cues from \
                                the conversation context or use gender-neutral language (they/them) to avoid misgendering.\n\
                                9. If you're unsure if a response is appropriate, respond with ONLY the word \"pass\"\n\n\
                                Remember: Be natural and direct - no meta-commentary. \
                                If you can't make it feel natural, just pass.",
                                self.bot_name, content, context_text
                            );
                            
                            // Process with Gemini API
                            match gemini_client.generate_content(&memory_prompt).await {
                                Ok(response) => {
                                    let response = response.trim();
                                    
                                    // Check if we should skip this one
                                    if response.to_lowercase() == "pass" {
                                        info!("Memory interjection evaluation: decided to PASS");
                                        return Ok(());
                                    }
                                    
                                    // Start typing indicator now that we've decided to send a message
                                    if let Err(e) = msg.channel_id.broadcast_typing(&ctx.http).await {
                                        error!("Failed to send typing indicator for memory interjection: {:?}", e);
                                    }
                                    
                                    // Send the processed memory
                                    if let Err(e) = msg.channel_id.say(&ctx.http, response).await {
                                        error!("Error sending enhanced memory interjection: {:?}", e);
                                    } else {
                                        info!("Enhanced memory interjection sent: {}", response);
                                    }
                                },
                                Err(e) => {
                                    error!("Error processing memory with Gemini API: {:?}", e);
                                }
                            }
                        }
                    },
                    Err(e) => {
                        error!("Error querying database for random message: {:?}", e);
                    }
                }
            }
        }
        
        // Pondering interjection
        let adjusted_pondering_probability = self.interjection_pondering_probability * silence_multiplier;
        if rand::thread_rng().gen_bool(adjusted_pondering_probability) {
            let probability_percent = self.interjection_pondering_probability * 100.0;
            let adjusted_percent = adjusted_pondering_probability * 100.0;
            let odds = if self.interjection_pondering_probability > 0.0 {
                format!("1 in {:.0}", 1.0 / self.interjection_pondering_probability)
            } else {
                "disabled".to_string()
            };
            
            info!("Triggered pondering interjection (base: {:.2}% chance, adjusted: {:.2}%, silence multiplier: {:.2}x, {})", 
                  probability_percent, adjusted_percent, silence_multiplier, odds);
            
            // Use Gemini API for pondering if available
            if let Some(gemini_client) = &self.gemini_client {
                // Get recent messages for context
                let recent_messages = if let Some(db) = &self.message_db {
                    match db_utils::get_recent_messages(
                        db.clone(),
                        5, // Get last 5 messages for context
                        Some(&msg.channel_id.to_string())
                    ).await {
                        Ok(messages) => messages,
                        Err(e) => {
                            error!("Error retrieving recent messages for pondering interjection: {:?}", e);
                            Vec::new()
                        }
                    }
                } else {
                    Vec::new()
                };
                
                // Format messages for context
                let context = if !recent_messages.is_empty() {
                    let mut formatted_messages = Vec::new();
                    for (_, _, content) in recent_messages {
                        // Skip empty messages
                        if content.trim().is_empty() {
                            continue;
                        }
                        
                        // Format the message (simplified since we don't have author info)
                        formatted_messages.push(format!("Message: {}", content));
                    }
                    
                    formatted_messages.join("\n")
                } else {
                    info!("No context available for pondering interjection in channel_id: {}", msg.channel_id);
                    "".to_string()
                };
                
                // Create a pondering-specific prompt
                let pondering_prompt = format!(
                    r#"You are {}, a Discord bot. Based on the conversation context, generate a very brief thoughtful comment or question.

{}

Requirements:
- Be extremely brief (maximum 1 short sentence)
- Sound natural and conversational
- Don't use phrases like "I wonder" or "I was thinking"
- Don't introduce yourself or explain your reasoning
- If you have nothing valuable to add, just respond with ONLY the word "pass" - nothing else
- If you include a reference to MST3K, it should be a direct quote that fits naturally in context (like "Watch out for snakes!"), not a forced reference (like "Even Tom Servo would find that interesting!")

Example good responses:
"That's an interesting perspective."
"Hmm, never thought of it that way."
"Fascinating approach to the problem."

Example bad responses:
"I was just thinking about how interesting that is."
"I wonder if there's more to consider about this topic."
"As someone interested in this conversation, I find that fascinating."

Keep it extremely brief and natural, as if you're just briefly pondering the conversation."#,
                    self.bot_name,
                    context
                );
                
                // Call Gemini API
                match gemini_client.generate_content(&pondering_prompt).await {
                    Ok(response) => {
                        // Check if the response starts with "pass" (case-insensitive) - if so, don't send anything
                        if response.trim().to_lowercase().starts_with("pass") {
                            info!("Pondering interjection evaluation: decided to PASS - no response sent");
                            return Ok(());
                        }
                        
                        // Check if the response contains parts of the prompt (API error)
                        if response.contains("You are") || 
                           response.contains("Requirements:") || 
                           response.contains("Example good responses:") {
                            error!("Pondering interjection error: API returned the prompt instead of a response");
                            // Log the issue but don't send any message to the channel
                            error!("Suppressing fallback pondering message as configured");
                            return Ok(());
                        }
                        
                        // Start typing indicator
                        if let Err(e) = msg.channel_id.broadcast_typing(&ctx.http).await {
                            error!("Failed to send typing indicator for pondering interjection: {:?}", e);
                        }
                        
                        // Calculate a realistic typing delay (0.2 seconds per word, min 1s, max 3s)
                        let word_count = response.split_whitespace().count();
                        let typing_delay = std::cmp::min(
                            std::cmp::max(word_count as u64 * 200, 1000),
                            3000
                        );
                        tokio::time::sleep(Duration::from_millis(typing_delay)).await;
                        
                        // Send the response
                        let response_text = response.clone(); // Clone for logging
                        if let Err(e) = msg.channel_id.say(&ctx.http, response).await {
                            error!("Error sending pondering interjection: {:?}", e);
                        } else {
                            info!("Pondering interjection sent: {}", response_text);
                        }
                    },
                    Err(e) => {
                        error!("Pondering interjection error: {:?}", e);
                        // Log the issue but don't send any message to the channel
                        error!("Suppressing fallback pondering message as configured");
                    }
                }
            } else {
                // If Gemini API is not configured, just log and don't send anything
                error!("Pondering interjection not available (Gemini API not configured) - no response sent");
                return Ok(());
            }
        }
        
        // AI interjection
        let adjusted_ai_probability = self.interjection_ai_probability * silence_multiplier;
        if rand::thread_rng().gen_bool(adjusted_ai_probability) {
            let probability_percent = self.interjection_ai_probability * 100.0;
            let adjusted_percent = adjusted_ai_probability * 100.0;
            let odds = if self.interjection_ai_probability > 0.0 {
                format!("1 in {:.0}", 1.0 / self.interjection_ai_probability)
            } else {
                "disabled".to_string()
            };
            
            info!("Triggered AI interjection (base: {:.2}% chance, adjusted: {:.2}%, silence multiplier: {:.2}x, {})", 
                  probability_percent, adjusted_percent, silence_multiplier, odds);
            
            if let Some(gemini_client) = &self.gemini_client {
                if let Some(interjection_prompt) = &self.gemini_interjection_prompt {
                    info!("Processing AI interjection with custom prompt");
                    
                    // We'll start typing indicator only after we decide to send a message
                    
                    // Get recent messages for context - use more messages for better context
                    let context_messages = if let Some(db) = &self.message_db {
                        match db_utils::get_recent_messages(db.clone(), self.gemini_context_messages, Some(msg.channel_id.to_string().as_str())).await {
                            Ok(messages) => messages,
                            Err(e) => {
                                error!("Error retrieving recent messages for AI interjection: {:?}", e);
                                Vec::new()
                            }
                        }
                    } else {
                        Vec::new()
                    };
                    
                    // Format context for the prompt
                    let context_text = if !context_messages.is_empty() {
                        // Reverse the messages to get chronological order (oldest first)
                        let mut chronological_messages = context_messages.clone();
                        chronological_messages.reverse();
                        
                        let formatted_messages: Vec<String> = chronological_messages.iter()
                            .map(|(_author, display_name, content)| format!("{}: {}", display_name, content))
                            .collect();
                        formatted_messages.join("\n")
                    } else {
                        info!("No context available for AI interjection in channel_id: {}", msg.channel_id);
                        // Use empty string instead of "No recent messages" to avoid showing this in logs
                        "".to_string()
                    };
                    
                    // Replace placeholders in the custom prompt
                    let prompt = interjection_prompt
                        .replace("{bot_name}", &self.bot_name)
                        .replace("{context}", &context_text);
                    
                    // Call Gemini API with the custom prompt - use bot name as user name
                    match gemini_client.generate_response_with_context(&prompt, &self.bot_name, &context_messages, None).await {
                        Ok(response) => {
                            // Check if the response starts with "pass" (case-insensitive) - if so, don't send anything
                            if response.trim().to_lowercase().starts_with("pass") {
                                info!("AI interjection evaluation: decided to PASS - no response sent");
                                return Ok(());
                            }
                            
                            // Check if the response looks like the prompt itself (API error)
                            if response.contains("{bot_name}") || 
                               response.contains("{context}") || 
                               response.contains("You should ONLY respond with an interjection if") ||
                               response.contains("For criterion #2") ||
                               response.contains("If none of these criteria are met") {
                                error!("AI interjection error: API returned the prompt instead of a response");
                                return Ok(());
                            }
                            
                            // Start typing indicator now that we've decided to send a message
                            if let Err(e) = msg.channel_id.broadcast_typing(&ctx.http).await {
                                error!("Failed to send typing indicator for AI interjection: {:?}", e);
                            }
                            
                            // Apply realistic typing delay
                            apply_realistic_delay(&response, ctx, msg.channel_id).await;
                            
                            // Send the response
                            let response_text = response.clone(); // Clone for logging
                            if let Err(e) = msg.channel_id.say(&ctx.http, response).await {
                                error!("Error sending AI interjection: {:?}", e);
                            } else {
                                info!("AI interjection evaluation: SENT response - {}", response_text);
                            }
                        },
                        Err(e) => {
                            error!("AI interjection evaluation: ERROR - {:?}", e);
                        }
                    }
                } else {
                    // If Gemini API is configured but interjection prompt is missing
                    info!("AI Interjection not available (GEMINI_INTERJECTION_PROMPT not configured) - no response sent");
                }
            } else {
                // If Gemini API is not configured
                info!("AI Interjection not available (Gemini API not configured) - no response sent");
            }
        }
        
        // Fact interjection
        let adjusted_fact_probability = self.interjection_fact_probability * silence_multiplier;
        if rand::thread_rng().gen_bool(adjusted_fact_probability) {
            let probability_percent = self.interjection_fact_probability * 100.0;
            let adjusted_percent = adjusted_fact_probability * 100.0;
            let odds = if self.interjection_fact_probability > 0.0 {
                format!("1 in {:.0}", 1.0 / self.interjection_fact_probability)
            } else {
                "disabled".to_string()
            };
            
            info!("Triggered fact interjection (base: {:.2}% chance, adjusted: {:.2}%, silence multiplier: {:.2}x, {})", 
                  probability_percent, adjusted_percent, silence_multiplier, odds);
            
            if let Some(gemini_client) = &self.gemini_client {
                // We'll use our dedicated fact interjection module
                if let Err(e) = fact_interjection::handle_fact_interjection(
                    ctx, 
                    msg, 
                    gemini_client, 
                    &self.message_db, 
                    &self.bot_name, 
                    self.gemini_context_messages
                ).await {
                    error!("Error handling fact interjection: {:?}", e);
                }
            } else {
                // If Gemini API is not configured
                info!("Fact Interjection not available (Gemini API not configured) - no response sent");
            }
        }
        
        // News interjection
        let adjusted_news_probability = self.interjection_news_probability * silence_multiplier;
        if rand::thread_rng().gen_bool(adjusted_news_probability) {
            let probability_percent = self.interjection_news_probability * 100.0;
            let adjusted_percent = adjusted_news_probability * 100.0;
            let odds = if self.interjection_news_probability > 0.0 {
                format!("1 in {:.0}", 1.0 / self.interjection_news_probability)
            } else {
                "disabled".to_string()
            };
            
            info!("Triggered news interjection (base: {:.2}% chance, adjusted: {:.2}%, silence multiplier: {:.2}x, {})", 
                  probability_percent, adjusted_percent, silence_multiplier, odds);
            
            if let Some(gemini_client) = &self.gemini_client {
                // Call the news interjection handler
                if let Err(e) = handle_news_interjection(
                    ctx, 
                    msg, 
                    gemini_client, 
                    &self.message_db, 
                    &self.bot_name, 
                    self.gemini_context_messages
                ).await {
                    error!("Error in news interjection: {:?}", e);
                }
            } else {
                // If Gemini API is not configured
                info!("News Interjection not available (Gemini API not configured) - no response sent");
            }
        }
        
        // Note: Message is already stored in the database in the message() event handler
        // No need to store it again here
        
        // Update recent speakers list
        {
            let data = ctx.data.read().await;
            if let Some(recent_speakers) = data.get::<RecentSpeakersKey>() {
                let mut speakers = recent_speakers.write().await;
                let username = msg.author.name.clone();
                // Use the best display name available
                let display_name = get_best_display_name(ctx, msg).await;
                
                // Clean up display name - remove <> brackets and [irc] tag
                let display_name = display_name::clean_display_name(&display_name);
                
                // Always update the list with the current speaker
                // Remove the user if they're already in the list
                if let Some(pos) = speakers.iter().position(|(name, _)| name == &username) {
                    speakers.remove(pos);
                }
                
                // Add the user to the end (most recent position)
                if speakers.len() >= 5 {
                    speakers.pop_front();
                }
                speakers.push_back((username, display_name));
                
                // Only log speakers list at debug level
                if tracing::level_enabled!(tracing::Level::DEBUG) {
                    let speakers_list: Vec<String> = speakers.iter().map(|(_, display)| display.clone()).collect();
                    debug!("Current speakers list: {:?}", speakers_list);
                }
            }
        }
        
        // Update message history
        {
            let data = ctx.data.read().await;
            if let Some(message_history) = data.get::<MessageHistoryKey>() {
                let mut history = message_history.write().await;
                if history.len() >= self.message_history_limit {
                    history.pop_front();
                }
                history.push_back(msg.clone());
            }
        }
        
        // Check for commands (messages starting with !)
        if msg.content.starts_with('!') {
            let parts: Vec<&str> = msg.content[1..].split_whitespace().collect();
            if !parts.is_empty() {
                let command = parts[0].to_lowercase();
                
                if command == "hello" {
                    // Simple hello command
                    if let Err(e) = msg.channel_id.say(&ctx.http, "world!").await {
                        error!("Error sending hello response: {:?}", e);
                    }
                } else if command == "trump" {
                    // Generate a Trump insult
                    let insult = self.trump_insult_generator.generate_insult();
                    if let Err(e) = msg.channel_id.say(&ctx.http, insult).await {
                        error!("Error sending Trump insult: {:?}", e);
                    }
                } else if command == "bandname" {
                    // Generate a band genre
                    if parts.len() > 1 {
                        let band_name = parts[1..].join(" ");
                        let genre = self.band_genre_generator.generate_genre(&band_name);
                        if let Err(e) = msg.channel_id.say(&ctx.http, genre).await {
                            error!("Error sending band genre: {:?}", e);
                        }
                    } else {
                        if let Err(e) = msg.reply(&ctx.http, "Please provide a band name.").await {
                            error!("Error sending usage message: {:?}", e);
                        }
                    }
                } else if command == "imagine" && !self.imagine_channels.is_empty() {
                    // Extract the image prompt
                    if parts.len() > 1 {
                        let prompt = parts[1..].join(" ");
                        if let Some(gemini_client) = &self.gemini_client {
                            if let Err(e) = handle_imagine_command(ctx, msg, gemini_client, &prompt, &self.imagine_channels).await {
                                error!("Error handling imagine command: {:?}", e);
                            }
                        } else {
                            if let Err(e) = msg.reply(&ctx.http, "Sorry, image generation is not available (Gemini API not configured).").await {
                                error!("Error sending API not configured message: {:?}", e);
                            }
                        }
                    } else {
                        if let Err(e) = msg.reply(&ctx.http, "Please provide a description of what you want me to show you.").await {
                            error!("Error sending usage message: {:?}", e);
                        }
                    }
                } else if command == "alive" {
                    // Check if a celebrity name was provided
                    if parts.len() > 1 {
                        let celebrity_name = parts[1..].join(" ");
                        if let Err(e) = handle_aliveordead_command(&ctx.http, &msg, &celebrity_name).await {
                            error!("Error handling alive command: {:?}", e);
                            if let Err(e) = msg.channel_id.say(&ctx.http, "Error checking celebrity status").await {
                                error!("Error sending error message: {:?}", e);
                            }
                        }
                    } else {
                        if let Err(e) = msg.reply(&ctx.http, "Please provide a celebrity name.").await {
                            error!("Error sending usage message: {:?}", e);
                        }
                    }
                } else if command == "help" {
                    // Help command - use the help message from our commands HashMap
                    if let Some(help_text) = self.commands.get("help") {
                        if let Err(e) = msg.channel_id.say(&ctx.http, help_text).await {
                            error!("Error sending help message: {:?}", e);
                        }
                    }
                } else if command == "info" {
                    // Handle the info command
                    if let Err(e) = self.handle_info_command(ctx, msg).await {
                        error!("Error handling info command: {:?}", e);
                    }
                } else if command == "slogan" {
                    // Extract search term if provided
                    let search_term = if parts.len() > 1 {
                        Some(parts[1..].join(" "))
                    } else {
                        None
                    };
                    
                    // Generate a slogan response
                    if let Err(e) = self.handle_slogan_command(&ctx.http, &msg, search_term).await {
                        error!("Error handling slogan command: {:?}", e);
                        if let Err(e) = msg.channel_id.say(&ctx.http, "Error accessing slogan database").await {
                            error!("Error sending error message: {:?}", e);
                        }
                    }
                } else if command == "quote" {
                    // Extract all arguments after the command
                    let args: Vec<&str> = if parts.len() > 1 {
                        parts[1..].to_vec()
                    } else {
                        Vec::new()
                    };
                    
                    // Check if this is a -dud request (quote a user)
                    if args.contains(&"-dud") {
                        let username_index = args.iter().position(|&r| r == "-dud").unwrap() + 1;
                        let username = if username_index < args.len() {
                            Some(args[username_index].to_string())
                        } else {
                            None
                        };
                        
                        if let Err(e) = self.handle_quote_dud_command(&ctx.http, &msg, username).await {
                            error!("Error handling quote -dud command: {:?}", e);
                            if let Err(e) = msg.channel_id.say(&ctx.http, "Error retrieving user quotes").await {
                                error!("Error sending error message: {:?}", e);
                            }
                        }
                    } else {
                        // Regular quote command with possible -show flag
                        if let Err(e) = self.handle_quote_command(&ctx.http, &msg, args).await {
                            error!("Error handling quote command: {:?}", e);
                            if let Err(e) = msg.channel_id.say(&ctx.http, "Error accessing quote database").await {
                                error!("Error sending error message: {:?}", e);
                            }
                        }
                    }
                } else if command == "fightcrime" {
                    match self.generate_crime_fighting_duo(&ctx, &msg).await {
                        Ok(duo) => {
                            if let Err(e) = msg.channel_id.say(&ctx.http, duo).await {
                                error!("Error sending crime fighting duo: {:?}", e);
                            }
                        },
                        Err(e) => {
                            error!("Error handling fightcrime command: {:?}", e);
                            if let Err(e) = msg.channel_id.say(&ctx.http, "Error generating crime fighting duo").await {
                                error!("Error sending error message: {:?}", e);
                            }
                        }
                    }
                } else if command == "buzz" {
                    // Handle the buzz command
                    if let Err(e) = handle_buzz_command(&ctx.http, &msg).await {
                        error!("Error handling buzz command: {:?}", e);
                        if let Err(e) = msg.channel_id.say(&ctx.http, "Error generating buzzword").await {
                            error!("Error sending error message: {:?}", e);
                        }
                    }
                } else if command == "lastseen" {
                    // Extract name to search for
                    let name = if parts.len() > 1 {
                        parts[1..].join(" ")
                    } else {
                        String::new()
                    };
                    
                    // Handle the lastseen command
                    if let Err(e) = handle_lastseen_command(&ctx.http, &msg, &name, &self.message_db).await {
                        error!("Error handling lastseen command: {:?}", e);
                        if let Err(e) = msg.channel_id.say(&ctx.http, "Error searching message history").await {
                            error!("Error sending error message: {:?}", e);
                        }
                    }
                } else if command == "frinkiac" {
                    // Extract args if provided
                    let args = if parts.len() > 1 {
                        Some(parts[1..].join(" "))
                    } else {
                        None
                    };
                    
                    // Handle the frinkiac command
                    if let Err(e) = handle_frinkiac_command(
                        &ctx.http, 
                        &msg, 
                        args, 
                        &self.frinkiac_client,
                        self.gemini_client.as_ref()
                    ).await {
                        error!("Error handling frinkiac command: {:?}", e);
                        if let Err(e) = msg.channel_id.say(&ctx.http, "Error searching Frinkiac").await {
                            error!("Error sending error message: {:?}", e);
                        }
                    }
                } else if command == "morbotron" {
                    // Extract search term if provided
                    let search_term = if parts.len() > 1 {
                        Some(parts[1..].join(" "))
                    } else {
                        None
                    };
                    
                    // Handle the morbotron command
                    if let Err(e) = handle_morbotron_command(
                        &ctx.http, 
                        &msg, 
                        search_term, 
                        &self.morbotron_client,
                        self.gemini_client.as_ref()
                    ).await {
                        error!("Error handling morbotron command: {:?}", e);
                        if let Err(e) = msg.channel_id.say(&ctx.http, "Error searching Morbotron").await {
                            error!("Error sending error message: {:?}", e);
                        }
                    }
                } else if command == "masterofallscience" {
                    // Extract search term if provided
                    let search_term = if parts.len() > 1 {
                        Some(parts[1..].join(" "))
                    } else {
                        None
                    };
                    
                    // Handle the masterofallscience command
                    if let Err(e) = handle_masterofallscience_command(
                        &ctx.http, 
                        &msg, 
                        search_term, 
                        &self.masterofallscience_client,
                        self.gemini_client.as_ref()
                    ).await {
                        error!("Error handling masterofallscience command: {:?}", e);
                        if let Err(e) = msg.channel_id.say(&ctx.http, "Error searching Master of All Science").await {
                            error!("Error sending error message: {:?}", e);
                        }
                    }
                } else if let Some(response) = self.commands.get(&command) {
                    if let Err(e) = msg.channel_id.say(&ctx.http, response).await {
                        error!("Error sending command response: {:?}", e);
                    }
                }
            }
            return Ok(());
        }
        
        // Check for Google search (messages starting with "google")
        if self.google_search_enabled && msg.content.to_lowercase().starts_with("google ") && msg.content.len() > 7 {
            let query = &msg.content[7..];
            
            if let Some(google_client) = &self.google_client {
                if let Err(e) = msg.channel_id.say(&ctx.http, format!("Searching for: {}", query)).await {
                    error!("Error sending search confirmation: {:?}", e);
                }
                
                // Perform the search
                match google_client.search(query).await {
                    Ok(Some(result)) => {
                        // Clean up the title by removing extra whitespace
                        let title = result.title.trim().replace("\n", " ").replace("  ", " ");
                        
                        // Format and send the result
                        let response = format!("**{}**\n{}\n{}", title, result.url, result.snippet);
                        if let Err(e) = msg.channel_id.say(&ctx.http, response).await {
                            error!("Error sending search result: {:?}", e);
                        }
                    },
                    Ok(None) => {
                        if let Err(e) = msg.channel_id.say(&ctx.http, "No search results found.").await {
                            error!("Error sending no results message: {:?}", e);
                        }
                    },
                    Err(e) => {
                        error!("Error performing search: {:?}", e);
                        if let Err(e) = msg.channel_id.say(&ctx.http, "Error performing search.").await {
                            error!("Error sending error message: {:?}", e);
                        }
                    }
                }
            } else {
                if let Err(e) = msg.channel_id.say(&ctx.http, "Search is not configured.").await {
                    error!("Error sending search error: {:?}", e);
                }
            }
            return Ok(());
        }
        
        // Check if the bot is being addressed using our new function
        if self.is_bot_addressed(&msg.content) {
            // Use the full message content including the bot's name
            let content = msg.content.trim().to_string();
            let content_lower = content.to_lowercase();
            
            // Check if the message contains "who fights crime" when the bot is addressed
            if content_lower.contains("who fights crime") {
                info!("Bot addressed with 'who fights crime' question");
                match self.generate_crime_fighting_duo(&ctx, &msg).await {
                    Ok(duo) => {
                        if let Err(e) = msg.channel_id.say(&ctx.http, duo).await {
                            error!("Error sending crime fighting duo: {:?}", e);
                        }
                        return Ok(());
                    },
                    Err(e) => {
                        error!("Error generating crime fighting duo: {:?}", e);
                        // Continue with normal response if crime fighting duo generation fails
                    }
                }
            }
            
            if !content.is_empty() {
                if let Some(gemini_client) = &self.gemini_client {
                    // Get and clean the display name
                    let display_name = get_best_display_name(ctx, msg).await;
                    let clean_display_name = clean_display_name(&display_name);
                    
                    // Extract pronouns from the original display name
                    let display_name = get_best_display_name(ctx, msg).await;
                    let user_pronouns = crate::display_name::extract_pronouns(&display_name);
                    
                    // Start typing indicator before making API call
                    if let Err(e) = msg.channel_id.broadcast_typing(&ctx.http).await {
                        error!("Failed to send typing indicator: {:?}", e);
                    }
                    
                    // Get recent messages for context
                    let context_messages = if let Some(db) = &self.message_db {
                        // Debug: Log the channel ID we are using
                        info!("Requesting context messages for channel ID: {}", msg.channel_id);
                        
                        // Get the last self.gemini_context_messages messages from the database
                        match db_utils::get_recent_messages(db.clone(), self.gemini_context_messages, Some(msg.channel_id.to_string().as_str())).await {
                            Ok(messages) => {
                                if messages.is_empty() {
                                    info!("No context messages found for channel {}", msg.channel_id);
                                } else {
                                    info!("Found {} context messages for channel {}", messages.len(), msg.channel_id);
                                }
                                messages
                            },
                            Err(e) => {
                                error!("Error retrieving recent messages: {:?}", e);
                                Vec::new()
                            }
                        }
                    } else {
                        Vec::new()
                    };
                    
                    // Call the Gemini API with context and pronouns
                    match gemini_client.generate_response_with_context(
                        &content, 
                        &clean_display_name, 
                        &context_messages,
                        user_pronouns.as_deref()
                    ).await {
                        Ok(response) => {
                            // Apply realistic typing delay based on response length
                            apply_realistic_delay(&response, ctx, msg.channel_id).await;
                            
                            // Create a message reference for replying
                            let message_reference = MessageReference::from(msg);
                            let create_message = CreateMessage::new()
                                .content(response.clone())
                                .reference_message(message_reference);
                            
                            if let Err(e) = msg.channel_id.send_message(&ctx.http, create_message).await {
                                error!("Error sending Gemini response as reply: {:?}", e);
                                // Fallback to regular message if reply fails
                                if let Err(e) = msg.channel_id.say(&ctx.http, response).await {
                                    error!("Error sending fallback Gemini response: {:?}", e);
                                }
                            }
                        },
                        Err(e) => {
                            error!("Error calling Gemini API: {:?}", e);
                            
                            // Create a message reference for replying
                            let message_reference = MessageReference::from(msg);
                            let create_message = CreateMessage::new()
                                .content(format!("Sorry, I encountered an error: {}", e))
                                .reference_message(message_reference);
                            
                            if let Err(e) = msg.channel_id.send_message(&ctx.http, create_message).await {
                                error!("Error sending error message as reply: {:?}", e);
                                // Fallback to regular message if reply fails
                                if let Err(e) = msg.channel_id.say(&ctx.http, format!("Sorry, I encountered an error: {}", e)).await {
                                    error!("Error sending fallback error message: {:?}", e);
                                }
                            }
                        }
                    }
                } else {
                    // Fallback if Gemini API is not configured
                    let display_name = get_best_display_name(ctx, msg).await;
                    
                    // Extract pronouns from the display name if present
                    let pronouns_info = if let Some(pronouns) = crate::display_name::extract_pronouns(&display_name) {
                        format!(" (I see you use {} pronouns)", pronouns)
                    } else {
                        String::new()
                    };
                    
                    if let Err(e) = msg.channel_id.say(&ctx.http, format!("Hello {}, you called my name! I'm {}!{} (Gemini API is not configured)", 
                        clean_display_name(&display_name), 
                        self.bot_name,
                        pronouns_info
                    )).await {
                        error!("Error sending name response: {:?}", e);
                    }
                }
                return Ok(());
            }
        }
        
        // Check for keyword triggers
        let content_lower = msg.content.to_lowercase();
        
        // First check for exact phrase matches (case insensitive)
        if content_lower.contains("who fights crime") {
            match self.generate_crime_fighting_duo(&ctx, &msg).await {
                Ok(duo) => {
                    if let Err(e) = msg.channel_id.say(&ctx.http, duo).await {
                        error!("Error sending crime fighting duo: {:?}", e);
                    }
                },
                Err(e) => {
                    error!("Error generating crime fighting duo: {:?}", e);
                    if let Err(e) = msg.channel_id.say(&ctx.http, "Error generating crime fighting duo").await {
                        error!("Error sending error message: {:?}", e);
                    }
                }
            }
            return Ok(());
        }
        
        if content_lower.contains("lisa needs braces") {
            if let Err(e) = msg.channel_id.say(&ctx.http, "DENTAL PLAN!").await {
                error!("Error sending response: {:?}", e);
            }
            return Ok(());
        }
        
        if content_lower.contains("my spoon is too big") {
            if let Err(e) = msg.channel_id.say(&ctx.http, "I am a banana!").await {
                error!("Error sending response: {:?}", e);
            }
            return Ok(());
        }
        
        // Then check for keyword-based triggers (words can be anywhere in message)
        for (keywords, response) in &self.keyword_triggers {
            if keywords.iter().all(|keyword| content_lower.contains(&keyword.to_lowercase())) {
                if let Err(e) = msg.channel_id.say(&ctx.http, response).await {
                    error!("Error sending keyword response: {:?}", e);
                }
                return Ok(());
            }
        }
        
        // Check for direct mentions of the bot
        let current_user_id = ctx.http.get_current_user().await.map(|u| u.id).unwrap_or_default();
        if msg.mentions_user_id(current_user_id) {
            // Extract the message content without the mention
            let content = msg.content.replace(&format!("<@{}>", current_user_id), "").trim().to_string();
            
            if !content.is_empty() {
                if let Some(gemini_client) = &self.gemini_client {
                    // Get and clean the display name
                    let display_name = get_best_display_name(ctx, msg).await;
                    let clean_display_name = clean_display_name(&display_name);
                    
                    // Extract pronouns from the original display name
                    let display_name = get_best_display_name(ctx, msg).await;
                    let user_pronouns = crate::display_name::extract_pronouns(&display_name);
                    
                    // Start typing indicator before making API call
                    if let Err(e) = msg.channel_id.broadcast_typing(&ctx.http).await {
                        error!("Failed to send typing indicator: {:?}", e);
                    }
                    
                    // Get recent messages for context
                    let context_messages = if let Some(db) = &self.message_db {
                        match db_utils::get_recent_messages(db.clone(), self.gemini_context_messages, Some(msg.channel_id.to_string().as_str())).await {
                            Ok(messages) => messages,
                            Err(e) => {
                                error!("Error retrieving recent messages: {:?}", e);
                                Vec::new()
                            }
                        }
                    } else {
                        Vec::new()
                    };
                        
                    match gemini_client.generate_response_with_context(
                        &content, 
                        &clean_display_name, 
                        &context_messages,
                        user_pronouns.as_deref()
                    ).await {
                        Ok(response) => {
                            // Apply realistic typing delay based on response length
                            apply_realistic_delay(&response, ctx, msg.channel_id).await;
                            
                            // Create a message reference for replying
                            let message_reference = MessageReference::from(msg);
                            let create_message = CreateMessage::new()
                                .content(response.clone())
                                .reference_message(message_reference);
                            
                            if let Err(e) = msg.channel_id.send_message(&ctx.http, create_message).await {
                                error!("Error sending Gemini response as reply: {:?}", e);
                                // Fallback to regular message if reply fails
                                if let Err(e) = msg.channel_id.say(&ctx.http, response).await {
                                    error!("Error sending fallback Gemini response: {:?}", e);
                                }
                            }
                        },
                        Err(e) => {
                            error!("Error calling Gemini API: {:?}", e);
                            
                            // Create a message reference for replying
                            let message_reference = MessageReference::from(msg);
                            let create_message = CreateMessage::new()
                                .content(format!("Sorry, I encountered an error: {}", e))
                                .reference_message(message_reference);
                            
                            if let Err(e) = msg.channel_id.send_message(&ctx.http, create_message).await {
                                error!("Error sending error message as reply: {:?}", e);
                                // Fallback to regular message if reply fails
                                if let Err(e) = msg.channel_id.say(&ctx.http, format!("Sorry, I encountered an error: {}", e)).await {
                                    error!("Error sending fallback error message: {:?}", e);
                                }
                            }
                        }
                    }
                } else {
                    // Fallback if Gemini API is not configured
                    let display_name = get_best_display_name(ctx, msg).await;
                    let clean_display_name = clean_display_name(&display_name);
                    if let Err(e) = msg.channel_id.say(&ctx.http, format!("Hello {}, you mentioned me! I'm {}! (Gemini API is not configured)", clean_display_name, self.bot_name)).await {
                        error!("Error sending mention response: {:?}", e);
                    }
                }
            }
        }
        
        Ok(())
    }
}
#[async_trait]
impl EventHandler for Bot {
    async fn message(&self, ctx: Context, msg: Message) {
        // Update the last activity time for this channel
        self.fill_silence_manager.update_activity(msg.channel_id, msg.author.id).await;
        
        // Mark that a user (not the bot) was the last speaker
        if msg.author.id != ctx.http.get_current_user().await.map(|u| u.id).unwrap_or_default() {
            self.fill_silence_manager.mark_user_as_last_speaker(msg.channel_id).await;
        }
        
        // Update the last seen message for this channel
        {
            let mut last_seen = self.last_seen_message.write().await;
            last_seen.insert(msg.channel_id, (msg.timestamp, msg.id));
        }
        
        // Store all messages in the database, including our own
        if let Some(db) = &self.message_db {
            // Get the display name
            let display_name = get_best_display_name(&ctx, &msg).await;
            
            // Check if this is a gateway bot message and extract the real username
            let (author_name, final_display_name) = if msg.author.bot {
                let bot_id = msg.author.id;
                
                // Check if this is a gateway bot
                if self.gateway_bot_ids.contains(&bot_id.get()) {
                    // Try to extract the gateway username from the message content or author name
                    if let Some(gateway_username) = crate::display_name::extract_gateway_username(&msg) {
                        // Cache the gateway username for future use
                        crate::display_name::cache_gateway_username(msg.author.id, &gateway_username);
                        
                        // Log the extraction for debugging
                        info!("📝 Extracted gateway username for bot {}: {}", bot_id, gateway_username);
                        
                        // Use the gateway username as both author and display name
                        (gateway_username.clone(), gateway_username)
                    } else {
                        // Fallback to the display name we got earlier
                        info!("📝 Could not extract gateway username for bot {}, using fallback", bot_id);
                        (msg.author.name.clone(), display_name)
                    }
                } else {
                    // Regular bot, use the display name we got earlier
                    (msg.author.name.clone(), display_name)
                }
            } else {
                // Regular user, use the display name we got earlier
                (msg.author.name.clone(), display_name)
            };
            
            // Save the message to the database
            if let Err(e) = db_utils::save_message(
                db.clone(),
                &author_name,
                &final_display_name,
                &msg.content,
                Some(&msg),
                None
            ).await {
                error!("Error saving message to database: {:?}", e);
            }
        }
        
        // Check if the message is from a bot
        if msg.author.bot {
            // Get the current bot's user ID
            let current_user_id = ctx.http.get_current_user().await.map(|u| u.id).unwrap_or_default();
            let bot_id = msg.author.id.get();
            
            // Check if this message is from the bot itself
            if msg.author.id == current_user_id {
                info!("🤖 Received message from SELF ({}): {}", msg.author.name, msg.content);
                // We still want to store our own messages in the database for context,
                // but we don't need to process them further
                return;
            }
            
            // Add detailed logging for other bot messages
            info!("📝 Received message from bot ID: {} ({})", bot_id, msg.author.name);
            info!("📝 Gateway bot IDs configured: {:?}", self.gateway_bot_ids);
            info!("📝 Is this bot in our gateway list? {}", self.gateway_bot_ids.contains(&bot_id));
            info!("📝 Message content: {}", msg.content);
            
            if !self.gateway_bot_ids.contains(&bot_id) {
                // Not in our gateway bot list, ignore the message for processing
                // (but we've already stored it in the database for context)
                info!("❌ Ignoring message from bot {} as it's not in our gateway bot list", bot_id);
                return;
            }
            // If it's in our gateway bot list, continue processing
            info!("✅ Processing message from gateway bot {}", bot_id);
        }
        
        // Only process messages in the followed channels
        if !self.followed_channels.contains(&msg.channel_id) {
            return;
        }
        
        // Special case: respond with "I know kung fu!" when someone says exactly "whoa"
        let trimmed_content = msg.content.trim().to_lowercase();
        if trimmed_content == "whoa" || trimmed_content == "woah" {
            info!("Responding to 'whoa' with Matrix reference");
            if let Err(e) = msg.channel_id.say(&ctx.http, "I know kung fu!").await {
                error!("Error sending kung fu response: {:?}", e);
            }
            return;
        }
        
        // Check for regex substitution (!s/, .s/, !/, ./)
        if msg.content.starts_with("!s/") || msg.content.starts_with(".s/") || 
           msg.content.starts_with("!/") || msg.content.starts_with("./") {
            if let Err(e) = handle_regex_substitution(&ctx, &msg).await {
                error!("Error handling regex substitution: {:?}", e);
            }
            return;
        }
        
        // Process the message
        if let Err(e) = self.process_message(&ctx, &msg).await {
            error!("Error processing message: {:?}", e);
        }
    }

    // Handle message updates (edits)
    async fn message_update(&self, ctx: Context, _old: Option<Message>, new: Option<Message>, _event: MessageUpdateEvent) {
        // Only process if we have the new message content
        if let Some(msg) = new {
            // Store the updated message in the database
            if let Some(db) = &self.message_db {
                // Get the display name
                let display_name = get_best_display_name(&ctx, &msg).await;
                
                // Save the message to the database (will update if it already exists)
                if let Err(e) = db_utils::save_message(
                    db.clone(),
                    &msg.author.name,
                    &display_name,
                    &msg.content,
                    Some(&msg),
                    None
                ).await {
                    error!("Error saving updated message to database: {:?}", e);
                }
            }
            
            // Only process messages in the followed channels
            if !self.followed_channels.contains(&msg.channel_id) {
                return;
            }
            
            // Process the updated message
            if let Err(e) = self.process_message(&ctx, &msg).await {
                error!("Error processing updated message: {:?}", e);
            }
        }
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("✅ {} ({}) is connected and following {} channels!", 
              self.bot_name, ready.user.name, self.followed_channels.len());
        
        // Log each followed channel
        for channel_id in &self.followed_channels {
            info!("Following channel: {}", channel_id);
        }
        
        info!("Bot is ready to respond to messages in the configured channels");
        
        // Load last seen messages from the database
        if let Some(db) = &self.message_db {
            match db_utils::get_last_messages_by_channel(db.clone()).await {
                Ok(last_seen_db) => {
                    info!("Loaded {} last seen messages from database", last_seen_db.len());
                    
                    // Update the in-memory last_seen_message map
                    let mut last_seen = self.last_seen_message.write().await;
                    for (channel_id, (timestamp, message_id)) in last_seen_db {
                        last_seen.insert(channel_id, (timestamp, message_id));
                        info!("Loaded last seen message for channel {}: {} at {}", 
                              channel_id, message_id, timestamp);
                    }
                },
                Err(e) => {
                    error!("Failed to load last seen messages from database: {}", e);
                }
            }
        }
        
        // Check for missed messages in each followed channel
        self.check_missed_messages(&ctx).await;
        
        // Log available commands
        let command_list = self.commands.keys()
            .map(|k| format!("!{}", k))
            .collect::<Vec<_>>()
            .join(", ");
        debug!("Available commands: {}", command_list);
        
        // Log keyword triggers
        debug!("Keyword triggers:");
        for (keywords, _) in &self.keyword_triggers {
            debug!("  - {}", keywords.join(" + "));
        }
    }
}

// Helper function to find channels by name
async fn find_channels_by_name(http: &Http, name: &str, server_name: Option<&str>) -> Vec<ChannelId> {
    // Get all the guilds (servers) the bot is in
    let guilds = match http.get_guilds(None, None).await {
        Ok(guilds) => guilds,
        Err(_) => return Vec::new(),
    };
    
    info!("Searching for channel '{}' across {} servers", name, guilds.len());
    
    let mut found_channels = Vec::new();
    
    // For each guild, try to find the channel
    for guild_info in guilds {
        let guild_id = guild_info.id;
        
        // If server_name is specified, check if this is the right server
        if let Some(server) = server_name {
            if let Ok(guild) = http.get_guild(guild_id).await {
                if guild.name != server {
                    info!("Skipping server '{}' as it doesn't match the specified server name '{}'", guild.name, server);
                    continue;  // Skip this server if name doesn't match
                }
                info!("Checking server '{}' for channel '{}'", guild.name, name);
            }
        } else if let Ok(guild) = http.get_guild(guild_id).await {
            info!("Checking server '{}' for channel '{}'", guild.name, name);
        }
        
        // Get all channels in this guild
        if let Ok(channels) = http.get_channels(guild_id).await {
            info!("Found {} channels in this server", channels.len());
            
            // Find the channel with the matching name
            for channel in channels {
                if channel.name == name {
                    info!("✅ Found matching channel '{}' (ID: {}) in server", channel.name, channel.id);
                    found_channels.push(channel.id);
                }
            }
            
            if found_channels.is_empty() {
                info!("No matching channel found in this server");
            }
        } else {
            info!("Could not retrieve channels for this server");
        }
    }
    
    if found_channels.is_empty() {
        info!("❌ Channel '{}' not found in any server", name);
    } else {
        info!("Found {} channels matching '{}'", found_channels.len(), name);
    }
    
    found_channels
}
#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    // Load configuration
    let config = load_config()?;
    
    // Get the discord token
    let token = &config.discord_token;
    
    // Parse config values
    let (bot_name, message_history_limit, db_trim_interval, gemini_rate_limit_minute, gemini_rate_limit_day, gateway_bot_ids, google_search_enabled, gemini_context_messages, interjection_mst3k_probability, interjection_memory_probability, interjection_pondering_probability, interjection_ai_probability, imagine_channels, interjection_news_probability, fill_silence_enabled, fill_silence_start_hours, fill_silence_max_hours) = 
        parse_config(&config);
        
    info!("News interjection probability: {}%", interjection_news_probability * 100.0);
        
    // Get fact interjection probability
    let interjection_fact_probability = config.interjection_fact_probability.clone()
        .unwrap_or_else(|| "0.005".to_string())
        .parse::<f64>()
        .unwrap_or(0.005);
    info!("Fact interjection probability: {}%", interjection_fact_probability * 100.0);
    
    // Parse interjection channel configuration
    let mut interjection_channel_ids = Vec::new();
    
    // Check for interjection channel IDs first
    if let Some(channel_ids_str) = &config.interjection_channel_ids {
        for id_str in channel_ids_str.split(',') {
            if let Ok(id) = id_str.trim().parse::<u64>() {
                interjection_channel_ids.push(ChannelId::new(id));
                info!("Added interjection channel ID: {}", id);
            } else {
                error!("Invalid interjection channel ID: {}", id_str);
            }
        }
    } else if let Some(channel_id_str) = &config.interjection_channel_id {
        if let Ok(id) = channel_id_str.trim().parse::<u64>() {
            interjection_channel_ids.push(ChannelId::new(id));
            info!("Added interjection channel ID: {}", id);
        } else {
            error!("Invalid interjection channel ID: {}", channel_id_str);
        }
    }
    
    // If no interjection channel IDs were specified, check for channel names
    if interjection_channel_ids.is_empty() {
        // We'll need to resolve channel names to IDs after connecting to Discord
        let mut interjection_channel_names = Vec::new();
        
        if let Some(names_str) = &config.interjection_channel_names {
            for name in names_str.split(',') {
                interjection_channel_names.push(name.trim().to_string());
                info!("Added interjection channel name: {}", name.trim());
            }
        } else if let Some(name) = &config.interjection_channel_name {
            interjection_channel_names.push(name.trim().to_string());
            info!("Added interjection channel name: {}", name.trim());
        }
        
        // If we have channel names, we'll need to resolve them after connecting
        if !interjection_channel_names.is_empty() {
            // Set gateway intents, which decides what events the bot will be notified about
            let temp_intents = GatewayIntents::GUILD_MESSAGES | GatewayIntents::MESSAGE_CONTENT | GatewayIntents::GUILDS;
            
            // Find the channel IDs first
            let temp_client = Client::builder(token, temp_intents).await?;
            
            // Get all guilds the bot is in
            let guilds = temp_client.http.get_guilds(None, None).await?;
            
            // Check each guild for the specified channel names
            for guild in &guilds {
                if let Ok(channels) = temp_client.http.get_channels(guild.id).await {
                    for channel in &channels {
                        if interjection_channel_names.contains(&channel.name) {
                            interjection_channel_ids.push(channel.id);
                            info!("Resolved interjection channel name '{}' to ID: {}", channel.name, channel.id);
                        }
                    }
                }
            }
        }
    }
    
    // Get Gemini API key
    let gemini_api_key = config.gemini_api_key.clone();
    if gemini_api_key.is_none() {
        error!("Gemini API key not found in config");
    } else {
        info!("Gemini API key loaded");
    }
    
    // Get Gemini logging setting
    let gemini_log_prompts = config.gemini_log_prompts.clone()
        .unwrap_or_else(|| "false".to_string())
        .parse::<bool>()
        .unwrap_or(false);
    info!("Gemini API prompt logging: {}", if gemini_log_prompts { "enabled" } else { "disabled" });
    
    // Get custom prompt wrapper if available
    let gemini_prompt_wrapper = config.gemini_prompt_wrapper.clone();
    if gemini_prompt_wrapper.is_some() {
        info!("Using custom Gemini prompt wrapper");
    } else {
        info!("Using default Gemini prompt wrapper");
    }
    
    // Get custom interjection prompt if available
    let gemini_interjection_prompt = config.gemini_interjection_prompt.clone().unwrap_or_else(|| {
        info!("No custom Gemini interjection prompt provided, using default");
        String::from(r#"You are {bot_name}, a Discord bot. Make a brief, relevant comment about the conversation if appropriate.

{context}

Only respond if you can:
1. Provide helpful clarification about the current topic
2. Answer a question that was asked but not answered
3. Add a brief, valuable comment to the conversation

Requirements:
- Be direct and concise (1-2 sentences maximum)
- Don't introduce yourself or explain your reasoning
- Don't use phrases like "I noticed" or "I see that"
- Don't reference this prompt or your role
- If you have nothing valuable to add, just respond with ONLY the word "pass" - nothing else
- If you include a reference to MST3K, it should be a direct quote that fits naturally in context (like "Watch out for snakes!"), not a forced reference (like "Even Tom Servo would find that interesting!")

Example good response: "The error message suggests a permissions issue with the file system."
Example bad response: "I noticed you're having trouble with file permissions. As a helpful bot, I can tell you that..."

Keep it brief and natural, as if you're just another participant in the conversation."#)
    });
    info!("Using Gemini interjection prompt");
    
    // Get custom personality description if available
    let gemini_personality_description = config.gemini_personality_description.clone();
    if gemini_personality_description.is_some() {
        info!("Using custom personality description");
    } else {
        info!("Using default personality description");
    }
    
    // Get custom Gemini API endpoint if available
    let gemini_api_endpoint = config.gemini_api_endpoint.clone();
    if let Some(endpoint) = &gemini_api_endpoint {
        info!("Using custom Gemini API endpoint: {}", endpoint);
    } else {
        info!("Using default Gemini API endpoint");
    }
    
    // Log configuration values
    info!("Configuration loaded:");
    info!("Bot name: {}", bot_name);
    info!("Message history limit: {}", message_history_limit);
    info!("Database trim interval: {} seconds", db_trim_interval);
    info!("Gemini rate limits: {} per minute, {} per day", gemini_rate_limit_minute, gemini_rate_limit_day);
    info!("Gemini context messages: {}", gemini_context_messages);
    info!("Google search enabled: {}", google_search_enabled);
    
    // Log channel configuration
    if let Some(channel_id) = &config.followed_channel_id {
        info!("Following channel ID: {}", channel_id);
    }
    if let Some(channel_name) = &config.followed_channel_name {
        info!("Following channel name: {}", channel_name);
    }
    if let Some(channel_ids) = &config.followed_channel_ids {
        info!("Following channel IDs: {}", channel_ids);
    }
    if let Some(channel_names) = &config.followed_channel_names {
        info!("Following channel names: {}", channel_names);
    }
    if let Some(server_name) = &config.followed_server_name {
        info!("Limiting to server: {}", server_name);
    }
    
    // Log interjection probabilities
    info!("MST3K interjection probability: {}%", interjection_mst3k_probability * 100.0);
    info!("Memory interjection probability: {}%", interjection_memory_probability * 100.0);
    info!("Pondering interjection probability: {}%", interjection_pondering_probability * 100.0);
    info!("AI interjection probability: {}%", interjection_ai_probability * 100.0);
    info!("Fact interjection probability: {}%", interjection_fact_probability * 100.0);
    info!("News interjection probability: {}%", interjection_news_probability * 100.0);

    // Log database configuration
    info!("Database configuration: host={:?}, db={:?}, user={:?}, password={}", 
          config.db_host, config.db_name, config.db_user, 
          if config.db_password.is_some() { "provided" } else { "not provided" });

    // Set gateway intents, which decides what events the bot will be notified about
    let intents = GatewayIntents::GUILD_MESSAGES | GatewayIntents::MESSAGE_CONTENT | GatewayIntents::GUILDS;

    // Initialize SQLite database for message history
    let db_path = "message_history.db";
    let message_db = match db_utils::initialize_database(db_path).await {
        Ok(conn) => {
            info!("Successfully connected to message history database");
            
            // Clean up duplicates and add unique index
            match db_utils::clean_up_duplicates(conn.clone()).await {
                Ok(count) => {
                    if count > 0 {
                        info!("Cleaned up {} duplicate messages in the database", count);
                    } else {
                        info!("No duplicate messages found in the database");
                    }
                },
                Err(e) => {
                    error!("Failed to clean up duplicate messages: {}", e);
                }
            }
            
            Some(conn) // Don't wrap in another Arc<Mutex>
        },
        Err(e) => {
            error!("Failed to initialize message database: {:?}", e);
            None
        }
    };
    
    // Find the channel ID first
    let client = Client::builder(token, intents).await?;
    
    // Successfully connected to Discord API
    info!("Successfully connected to Discord API");
    
    // Get the bot's user information
    let current_user = client.http.get_current_user().await?;
    info!("Logged in as {} (ID: {})", current_user.name, current_user.id);
    
    // List all guilds the bot is in
    let guilds = client.http.get_guilds(None, None).await?;
    info!("Bot is in {} servers:", guilds.len());
    for guild in &guilds {
        info!("  - {} (ID: {})", guild.name, guild.id);
        
        // List channels in this guild
        if let Ok(channels) = client.http.get_channels(guild.id).await {
            info!("    Channels in this server:");
            for channel in &channels {
                info!("      - {} (ID: {}, Type: {:?})", channel.name, channel.id, channel.kind);
            }
        } else {
            info!("    Could not retrieve channels for this server");
        }
    }
    
    if guilds.is_empty() {
        info!("Bot is not in any servers. Please invite the bot to a server first.");
        info!("You can generate an invite link from the Discord Developer Portal.");
    }
    
    // Find the channel IDs
    info!("Looking for channels to follow...");
    
    let mut channel_ids = Vec::new();
    
    // First check for multiple channel IDs
    if let Some(ids_str) = &config.followed_channel_ids {
        for id_str in ids_str.split(',') {
            let id_str = id_str.trim();
            if let Ok(id) = id_str.parse::<u64>() {
                info!("Adding channel ID: {}", id);
                channel_ids.push(ChannelId::new(id));
            } else {
                error!("Invalid channel ID: {}", id_str);
            }
        }
    }
    
    // Then check for multiple channel names
    if let Some(names_str) = &config.followed_channel_names {
        for name in names_str.split(',') {
            let name = name.trim();
            info!("Searching for channel with name: '{}'", name);
            
            let found_channels = find_channels_by_name(
                &client.http, 
                name, 
                config.followed_server_name.as_deref()
            ).await;
            
            for channel_id in found_channels {
                if !channel_ids.contains(&channel_id) {
                    info!("Adding channel '{}' with ID {}", name, channel_id);
                    channel_ids.push(channel_id);
                }
            }
        }
    }
    
    // Then check for single channel ID (legacy support)
    if let Some(id_str) = &config.followed_channel_id {
        if let Ok(id) = id_str.parse::<u64>() {
            let channel_id = ChannelId::new(id);
            if !channel_ids.contains(&channel_id) {
                info!("Adding single channel ID: {}", id);
                channel_ids.push(channel_id);
            }
        } else {
            error!("Invalid channel ID: {}", id_str);
        }
    }
    
    // Finally check for single channel name (legacy support)
    if let Some(name) = &config.followed_channel_name {
        info!("Searching for single channel with name: '{}'", name);
        
        let found_channels = find_channels_by_name(
            &client.http, 
            name, 
            config.followed_server_name.as_deref()
        ).await;
        
        for channel_id in found_channels {
            if !channel_ids.contains(&channel_id) {
                info!("Adding channel '{}' with ID {}", name, channel_id);
                channel_ids.push(channel_id);
            }
        }
    }
    
    // Check if we found any channels
    if channel_ids.is_empty() {
        error!("❌ No valid channels found to follow!");
        return Err(anyhow::anyhow!("No valid channels found to follow"));
    }
    
    info!("✅ Found {} channels to follow", channel_ids.len());
    
    // Clone values for the Bot struct
    let gemini_api_key_for_bot = gemini_api_key.clone();
    let gemini_api_endpoint_for_bot = gemini_api_endpoint.clone();
    let gemini_prompt_wrapper_for_bot = gemini_prompt_wrapper.clone();
    let gemini_personality_description_for_bot = gemini_personality_description.clone();
    
    // Create a new bot instance with the valid channel IDs
    let bot = Bot::new(
        channel_ids.clone(),
        config.db_host.clone(),
        config.db_name.clone(),
        config.db_user.clone(),
        config.db_password.clone(),
        None, // No Google API key needed anymore
        None, // No Google Search Engine ID needed anymore
        gemini_api_key_for_bot,
        gemini_api_endpoint_for_bot,
        gemini_prompt_wrapper_for_bot,
        Some(gemini_interjection_prompt),
        bot_name.clone(),
        message_db.clone(),
        message_history_limit,
        gateway_bot_ids.clone(),
        google_search_enabled,
        gemini_rate_limit_minute,
        gemini_rate_limit_day,
        gemini_context_messages,
        interjection_mst3k_probability,
        interjection_memory_probability,
        interjection_pondering_probability,
        interjection_ai_probability,
        interjection_fact_probability,
        interjection_news_probability,
        gemini_log_prompts,
        imagine_channels,
        fill_silence_enabled,
        fill_silence_start_hours,
        fill_silence_max_hours,
        gemini_personality_description_for_bot
    );
    
    // Check database connection
    if let Err(e) = bot.check_database_connection().await {
        error!("Error checking database connection: {:?}", e);
    }
    
    // Start the database trimming task
    if let Some(db) = &message_db {
        let db_clone = db.clone();
        let limit = message_history_limit;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(db_trim_interval)).await;
                info!("Running scheduled database trim task");
                match db_utils::trim_message_history(db_clone.clone(), limit).await {
                    Ok(deleted) => {
                        if deleted > 0 {
                            info!("Trimmed database: removed {} old messages", deleted);
                        }
                    },
                    Err(e) => {
                        error!("Error trimming database: {:?}", e);
                    }
                }
            }
        });
        info!("Started database trimming task (interval: {} seconds, limit: {} messages)", db_trim_interval, message_history_limit);
    }
    
    // Create a client with the event handler
    info!("Creating Discord client with event handler...");
    
    // Clone what we need for the spontaneous interjection task
    let fill_silence_manager = bot.fill_silence_manager.clone();
    
    let mut client = Client::builder(token, intents)
        .event_handler(bot)
        .await?;
        
    // Initialize the data structures in the client data
    {
        let mut data = client.data.write().await;
        let recent_speakers = Arc::new(RwLock::new(VecDeque::<(String, String)>::with_capacity(5)));
        let message_history = Arc::new(RwLock::new(VecDeque::with_capacity(message_history_limit)));
        
        // Load existing messages if database is available
        if let Some(db) = &message_db {
            // Create a temporary VecDeque to hold the loaded messages
            let mut temp_history = VecDeque::new();
            let db_clone = db.clone();
            
            if let Err(e) = db_utils::load_message_history(db_clone, &mut temp_history, message_history_limit, None).await {
                error!("Failed to load message history: {:?}", e);
            } else {
                info!("Loaded {} messages from database", temp_history.len());
                
                // For now, we can't directly convert the loaded messages to serenity Message objects
                // In a real implementation, you would need to create Message objects from the stored data
                // or modify the database schema to store all necessary fields
            }
        }
        
        info!("Initializing RecentSpeakersKey in client data");
        data.insert::<RecentSpeakersKey>(recent_speakers);
        info!("Initializing MessageHistoryKey in client data");
        data.insert::<MessageHistoryKey>(message_history);
    }
        
    // Initialize the data structures in the client data
    {
        let mut data = client.data.write().await;
        let recent_speakers = Arc::new(RwLock::new(VecDeque::<(String, String)>::with_capacity(5)));
        let message_history = Arc::new(RwLock::new(VecDeque::with_capacity(message_history_limit)));
        
        // Load existing messages if database is available
        if let Some(db) = &message_db {
            // Create a temporary VecDeque to hold the loaded messages
            let mut temp_history = VecDeque::new();
            let db_clone = db.clone();
            
            if let Err(e) = db_utils::load_message_history(db_clone, &mut temp_history, message_history_limit, None).await {
                error!("Failed to load message history: {:?}", e);
            } else {
                info!("Loaded {} messages from database", temp_history.len());
                
                // For now, we can't directly convert the loaded messages to serenity Message objects
                // In a real implementation, you would need to create Message objects from the stored data
                // or modify the database schema to store all necessary fields
            }
        }
        
        info!("Initializing RecentSpeakersKey in client data");
        data.insert::<RecentSpeakersKey>(recent_speakers);
        info!("Initializing MessageHistoryKey in client data");
        data.insert::<MessageHistoryKey>(message_history);
    }
    
    // Start the client
    info!("✅ Bot initialization complete! Starting bot...");
    info!("Bot name: {}", bot_name);
    info!("Following {} channels", channel_ids.len());
    for channel_id in &channel_ids {
        info!("- Channel ID: {}", channel_id);
    }
    if !gateway_bot_ids.is_empty() {
        info!("Will respond to gateway bots with IDs: {:?}", gateway_bot_ids);
    }
    info!("Google search feature is {}", if google_search_enabled { "enabled" } else { "disabled" });
    
    // Start the spontaneous interjection task if fill silence is enabled
    if fill_silence_enabled {
        info!("Starting spontaneous interjection task for fill silence feature");
        
        // Clone what we need for the task
        let http = client.http.clone();
        let interjection_channels = interjection_channel_ids.clone();
        let bot_id = client.http.get_current_user().await?.id;
        let message_db_clone = message_db.clone();
        let bot_name_clone = bot_name.clone();
        
        // Log interjection channels
        info!("Sending interjections to {} channels", interjection_channels.len());
        for channel_id in &interjection_channels {
            info!("- Interjection channel ID: {}", channel_id);
        }
        
        // Create a new Gemini client for the task if we have an API key
        let task_gemini_client = if let Some(api_key) = &gemini_api_key {
            info!("Creating Gemini client for spontaneous interjection task");
            Some(GeminiClient::new(
                api_key.clone(),
                gemini_api_endpoint.clone(),
                gemini_prompt_wrapper.clone(),
                bot_name.clone(),
                gemini_rate_limit_minute,
                gemini_rate_limit_day,
                gemini_context_messages,
                gemini_log_prompts,
                gemini_personality_description.clone()
            ))
        } else {
            None
        };
        
        // Spawn the task
        tokio::spawn(async move {
            loop {
                // Check each channel for spontaneous interjections
                for channel_id in &interjection_channels {
                    if fill_silence_manager.should_check_spontaneous_interjection(*channel_id, bot_id).await {
                        // Get a random interjection type (skipping type 2 - Message Pondering)
                        let mut interjection_type = rand::thread_rng().gen_range(0..=4);
                        
                        // Adjust the type number to skip over type 2
                        if interjection_type >= 2 {
                            interjection_type += 1;
                        }
                        
                        info!("Making spontaneous interjection in channel {} (type: {})", channel_id, interjection_type);
                        
                        // Send a typing indicator
                        if let Err(e) = channel_id.broadcast_typing(&http).await {
                            error!("Failed to send typing indicator for spontaneous interjection: {:?}", e);
                        }
                        
                        // Wait a bit to simulate typing
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        
                        // Send a message based on the interjection type
                        let message = match interjection_type {
                            0 => {
                                // MST3K Quote interjection - log but don't send anything
                                info!("Spontaneous MST3K quote interjection requested but fallbacks are disabled");
                                String::new()
                            },
                            1 => {
                                // Memory interjection - get a random message from the database and process it
                                if let Some(db) = &message_db_clone {
                                    // Get recent messages for context
                                    let context_messages = match db_utils::get_recent_messages(db.clone(), gemini_context_messages, Some(&channel_id.to_string())).await {
                                        Ok(messages) => messages,
                                        Err(e) => {
                                            error!("Error retrieving recent messages for memory interjection: {:?}", e);
                                            Vec::new()
                                        }
                                    };
                                    
                                    // Format context for the prompt
                                    let context_text = if !context_messages.is_empty() {
                                        // Reverse the messages to get chronological order (oldest first)
                                        let mut chronological_messages = context_messages.clone();
                                        chronological_messages.reverse();
                                        
                                        let formatted_messages: Vec<String> = chronological_messages.iter()
                                            .map(|(_author, display_name, content)| format!("{}: {}", display_name, content))
                                            .collect();
                                        formatted_messages.join("\n")
                                    } else {
                                        "".to_string()
                                    };
                                    // Query the database for a random message with minimum length of 20 characters
                                    let query_result = db.lock().await.call(|conn| {
                                        let query = "SELECT content, author, display_name FROM messages WHERE length(content) >= 20 ORDER BY RANDOM() LIMIT 1";
                                        let mut stmt = conn.prepare(query)?;
                                        
                                        let rows = stmt.query_map([], |row| {
                                            Ok((
                                                row.get::<_, String>(0)?,
                                                row.get::<_, String>(1)?,
                                                row.get::<_, Option<String>>(2)?.unwrap_or_default()
                                            ))
                                        })?;
                                        
                                        let mut result = Vec::new();
                                        for row in rows {
                                            result.push(row?);
                                        }
                                        
                                        Ok::<_, rusqlite::Error>(result)
                                    }).await;
                                    
                                    match query_result {
                                        Ok(messages) => {
                                            if let Some((content, _, _)) = messages.first() {
                                                // If we have a Gemini client, process the message
                                                if let Some(gemini) = &task_gemini_client {
                                                    let memory_prompt = format!(
                                                        "You are {}, a witty Discord bot. You've found this message in your memory: \"{}\". \
                                                        Please contribute to the conversation by saying something related to this memory.\n\n\
                                                        Recent conversation context (use this for relevance only):\n{}\n\n\
                                                        Guidelines:\n\
                                                        1. Your comment should be primarily based on the MEMORY, not the recent context\n\
                                                        2. Use the recent context only to make your comment relevant to the current conversation\n\
                                                        3. Keep it short and natural (1-2 sentences)\n\
                                                        4. Don't quote or reference the memory directly - just say what you want to say\n\
                                                        5. Don't identify yourself or explain what you're doing\n\
                                                        6. If you can't make it work naturally, respond with 'pass'\n\
                                                        7. Correct any obvious typos but preserve the message's character\n\
                                                        8. NEVER reference the movie \"Manos: The Hands of Fate\"\n\
                                                        9. NEVER make jokes about dating, relationships, or sexual topics\n\
                                                        10. ALWAYS use a person's correct pronouns when addressing or referring to them. If someone has specified their pronouns \
                                                        (e.g., in their username like \"name (she/her)\"), ALWAYS use those pronouns. If pronouns aren't specified, take cues from \
                                                        the conversation context or use gender-neutral language (they/them) to avoid misgendering.\n\
                                                        11. If you're unsure if a response is appropriate, respond with ONLY the word \"pass\"\n\
                                                        Remember: Be natural and direct - no meta-commentary.",
                                                        bot_name_clone, content, context_text
                                                    );
                                                    
                                                    match gemini.generate_response_with_context(&memory_prompt, "", &context_messages, None).await {
                                                        Ok(response) => {
                                                            if response.trim().to_lowercase() == "pass" {
                                                                info!("Gemini API chose to pass on memory interjection");
                                                                String::new() // Return empty string to skip the interjection
                                                            } else {
                                                                response
                                                            }
                                                        },
                                                        Err(e) => {
                                                            error!("Error generating memory interjection: {:?}", e);
                                                            String::new() // Return empty string to skip the interjection
                                                        }
                                                    }
                                                } else {
                                                    // No Gemini API, just use the content directly
                                                    content.clone()
                                                }
                                            } else {
                                                info!("No suitable messages found for memory interjection");
                                                String::new()
                                            }
                                        },
                                        Err(e) => {
                                            error!("Error querying database for memory interjection: {:?}", e);
                                            String::new()
                                        }
                                    }
                                } else {
                                    info!("No message database available for memory interjection");
                                    String::new()
                                }
                            },
                            2 => {
                                // Pondering interjection - log but don't send anything
                                info!("Spontaneous pondering interjection requested but fallbacks are disabled");
                                String::new()
                            },
                            3 => {
                                // AI-like interjection using Gemini API
                                if let Some(gemini_client) = &task_gemini_client {
                                    // Get recent messages for context
                                    let context_messages = if let Some(db) = &message_db_clone {
                                        match db_utils::get_recent_messages(db.clone(), gemini_context_messages, Some(&channel_id.to_string())).await {
                                            Ok(messages) => messages,
                                            Err(e) => {
                                                error!("Error retrieving recent messages for AI interjection: {:?}", e);
                                                Vec::new()
                                            }
                                        }
                                    } else {
                                        Vec::new()
                                    };
                                    
                                    // Format context for the prompt
                                    let context_text = if !context_messages.is_empty() {
                                        // Reverse the messages to get chronological order (oldest first)
                                        let mut chronological_messages = context_messages.clone();
                                        chronological_messages.reverse();
                                        
                                        let formatted_messages: Vec<String> = chronological_messages.iter()
                                            .map(|(_author, display_name, content)| format!("{}: {}", display_name, content))
                                            .collect();
                                        formatted_messages.join("\n")
                                    } else {
                                        "".to_string()
                                    };
                                    
                                    // Create the AI interjection prompt
                                    let ai_prompt = format!(
                                        "You are {}, a witty Discord bot who lives on the Satellite of Love. \
                                        Please contribute to the conversation with a brief, natural comment.\n\n\
                                        Recent conversation context:\n{}\n\n\
                                        Guidelines:\n\
                                        1. Keep it short and natural (1-2 sentences)\n\
                                        2. Be relevant to the conversation topic\n\
                                        3. Be witty, friendly, and slightly sarcastic\n\
                                        4. Don't identify yourself or explain what you're doing\n\
                                        5. If you can't make a relevant comment, respond with ONLY the word \"pass\"\n\
                                        6. You may include a direct MST3K reference if relevant (like \"Watch out for snakes!\")\n\
                                        7. Don't use phrases like \"I noticed\" or \"I see you're talking about\"\n\
                                        8. NEVER reference the movie \"Manos: The Hands of Fate\" - this reference is overused\n\
                                        9. NEVER make jokes about dating, relationships, or sexual topics\n\
                                        10. ALWAYS use a person's correct pronouns when addressing or referring to them. If someone has specified their pronouns \
                                        (e.g., in their username like \"name (she/her)\"), ALWAYS use those pronouns. If pronouns aren't specified, take cues from \
                                        the conversation context or use gender-neutral language (they/them) to avoid misgendering.\n\
                                        11. If you're unsure if a response is appropriate, respond with ONLY the word \"pass\"\n\
                                        Remember: Be natural and direct - no meta-commentary.",
                                        bot_name_clone, context_text
                                    );
                                    
                                    // Call Gemini API with the AI prompt
                                    match gemini_client.generate_response_with_context(&ai_prompt, "", &context_messages, None).await {
                                        Ok(response) => {
                                            // Check if the response is "pass" - if so, don't send anything
                                            if response.trim().to_lowercase() == "pass" {
                                                info!("AI interjection evaluation: decided to PASS - no response sent");
                                                String::new() // Return empty string to skip the interjection
                                            } else {
                                                response
                                            }
                                        },
                                        Err(e) => {
                                            error!("Error generating AI interjection: {:?}", e);
                                            String::new() // Return empty string to skip the interjection
                                        }
                                    }
                                } else {
                                    // If Gemini API is not configured
                                    info!("AI Interjection not available (Gemini API not configured) - no response sent");
                                    String::new()
                                }
                            },
                            4 => {
                                // Fact interjection using Gemini API
                                if let Some(gemini_client) = &task_gemini_client {
                                    // Use the dedicated fact interjection module for spontaneous interjections
                                    match fact_interjection::handle_spontaneous_fact_interjection(
                                        &http, 
                                        *channel_id, 
                                        gemini_client, 
                                        &message_db_clone, 
                                        &bot_name_clone, 
                                        gemini_context_messages
                                    ).await {
                                        Ok(_) => {
                                            // The fact was sent directly by the module, so return empty string
                                            // to prevent the spontaneous interjection task from sending another message
                                            String::new()
                                        },
                                        Err(e) => {
                                            error!("Error handling spontaneous fact interjection: {:?}", e);
                                            String::new()
                                        }
                                    }
                                } else {
                                    // If Gemini API is not configured
                                    info!("Fact Interjection not available (Gemini API not configured) - no response sent");
                                    String::new()
                                }
                            },
                            _ => {
                                // Use the AI-generated news interjection
                                if let Some(gemini_client) = &task_gemini_client {
                                    // Get recent messages for context
                                    let context_messages = if let Some(db) = &message_db_clone {
                                        match db_utils::get_recent_messages(db.clone(), gemini_context_messages, Some(&channel_id.to_string())).await {
                                            Ok(messages) => messages,
                                            Err(e) => {
                                                error!("Error retrieving recent messages for news interjection: {:?}", e);
                                                Vec::new()
                                            }
                                        }
                                    } else {
                                        Vec::new()
                                    };
                                    
                                    // Format context for the prompt
                                    let context_text = if !context_messages.is_empty() {
                                        // Reverse the messages to get chronological order (oldest first)
                                        let mut chronological_messages = context_messages.clone();
                                        chronological_messages.reverse();
                                        
                                        let formatted_messages: Vec<String> = chronological_messages.iter()
                                            .map(|(_author, display_name, content)| format!("{}: {}", display_name, content))
                                            .collect();
                                        formatted_messages.join("\n")
                                    } else {
                                        "".to_string()
                                    };
                                    
                                    // Create the news prompt
                                    let news_prompt = String::from(r#"You are {bot_name}, a Discord bot. Share an interesting technology or weird news article link with a brief comment about why it's interesting.

{context}

Guidelines:
1. Create a fictional but plausible news article link about technology or weird news (NO sports)
2. Format as: "Article title: https://example.com/article-path"
3. The URL must be specific and detailed (e.g., https://arstechnica.com/tech-policy/2025/06/new-ai-regulations-impact-open-source/)
4. Never use generic URLs like https://arstechnica.com/ or https://techcrunch.com/
5. Always include year, month, and a descriptive path in the URL
6. Then add a brief comment (1-2 sentences) on why it's interesting or relevant to the conversation
7. If possible, relate it to the conversation, but don't force it
8. Don't use phrases like "Check out this article" or "You might find this interesting"
9. NEVER include tags like "(via search)", "(via Google)", or any other source attribution
10. If you can't think of a relevant article, respond with "pass"

Example good response: "AI Creates Perfect Pizza Recipe Through Taste Simulation: https://techcrunch.com/2025/06/ai-taste-simulation-pizza This shows how AI sensory processing is advancing beyond visual and audio into taste simulation."

Example bad response: "Check out this interesting article about AI and food: https://techcrunch.com/ai-food-article (via search) I thought you might find this interesting given our conversation about technology."

Be creative but realistic with your article title and URL."#)
                                        .replace("{bot_name}", &bot_name_clone)
                                        .replace("{context}", &context_text);
                                    
                                    // Call Gemini API with the news prompt
                                    match gemini_client.generate_response_with_context(&news_prompt, "", &context_messages, None).await {
                                        Ok(response) => {
                                            // Check if the response is "pass" - if so, don't send anything
                                            if response.trim().to_lowercase() == "pass" {
                                                info!("News interjection evaluation: decided to PASS - no response sent");
                                                String::new() // Return empty string to skip the interjection
                                            } else {
                                                // Remove any "(via search)" or similar tags using regex
                                                let via_regex = regex::Regex::new(r"\s*\(via\s+[^)]+\)\s*").unwrap();
                                                let cleaned_response = via_regex.replace_all(&response, "").to_string();
                                                
                                                // Validate the URL
                                                let url_regex = regex::Regex::new(r"https?://[^\s]+").unwrap();
                                                if let Some(url_match) = url_regex.find(&cleaned_response) {
                                                    let url_str = url_match.as_str();
                                                    
                                                    // Try to parse the URL
                                                    if let Ok(url) = url::Url::parse(url_str) {
                                                        // Check if the URL has a proper path (not just "/")
                                                        let path = url.path();
                                                        if path.len() <= 1 {
                                                            // URL doesn't have a proper path
                                                            info!("News interjection URL validation failed: URL has no proper path: {}", url_str);
                                                            String::new()
                                                        } else {
                                                            // Validate that the URL actually exists
                                                            match news_interjection::validate_url_exists(url_str).await {
                                                                Ok((true, Some(final_url))) => {
                                                                    // URL exists, return the cleaned response with the final URL
                                                                    info!("URL validation successful: {} exists", final_url);
                                                                    if url_str != final_url {
                                                                        cleaned_response.replace(url_str, &final_url).trim().to_string()
                                                                    } else {
                                                                        cleaned_response.trim().to_string()
                                                                    }
                                                                },
                                                                Ok(_) => {
                                                                    // URL doesn't exist or isn't HTML
                                                                    info!("News interjection skipped: URL doesn't exist or isn't HTML: {}", url_str);
                                                                    String::new()
                                                                },
                                                                Err(e) => {
                                                                    // Error validating URL
                                                                    error!("Error validating URL {}: {:?}", url_str, e);
                                                                    String::new()
                                                                }
                                                            }
                                                        }
                                                    } else {
                                                        // Invalid URL
                                                        info!("News interjection URL validation failed: Invalid URL: {}", url_str);
                                                        String::new()
                                                    }
                                                } else {
                                                    // No URL found
                                                    info!("News interjection URL validation failed: No URL found in response");
                                                    String::new()
                                                }
                                            }
                                        },
                                        Err(e) => {
                                            error!("Error generating news interjection: {:?}", e);
                                            String::new()
                                        }
                                    }
                                } else {
                                    // If Gemini API is not configured
                                    info!("News Interjection not available (Gemini API not configured) - no response sent");
                                    String::new()
                                }
                            }
                        };
                        
                        // Only send the message if it's not empty
                        if !message.trim().is_empty() {
                            if let Err(e) = channel_id.say(&http, message.clone()).await {
                                error!("Failed to send spontaneous interjection: {:?}", e);
                            } else {
                                info!("Sent spontaneous interjection: {}", message);
                                
                                // Mark the bot as the last speaker in this channel
                                fill_silence_manager.mark_bot_as_last_speaker(*channel_id).await;
                                
                                // Update the last activity time for this channel
                                fill_silence_manager.update_activity(*channel_id, bot_id).await;
                            }
                        } else {
                            info!("Skipping empty spontaneous interjection");
                        }
                    }
                }
                
                // Sleep for a minute before checking again
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        });
    }
    
    info!("Press Ctrl+C to stop the bot");
    client.start().await?;

    Ok(())
}
