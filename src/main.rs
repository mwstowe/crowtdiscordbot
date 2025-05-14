use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use anyhow::Result;
use serenity::all::*;
use serenity::async_trait;
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::model::id::ChannelId;
use serenity::prelude::*;
use serenity::builder::CreateMessage;
use serenity::model::channel::MessageReference;
use serenity::model::event::MessageUpdateEvent;
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
mod display_name;
mod buzz;
mod lastseen;

// Use our modules
use config::{load_config, parse_config};
use database::DatabaseManager;
use google_search::GoogleSearchClient;
use gemini_api::GeminiClient;
use crime_fighting::CrimeFightingGenerator;
use frinkiac::{FrinkiacClient, handle_frinkiac_command};
use morbotron::{MorbotronClient, handle_morbotron_command};
use response_timing::apply_realistic_delay;
use masterofallscience::{MasterOfAllScienceClient, handle_masterofallscience_command};
use display_name::get_best_display_name;
use buzz::handle_buzz_command;
use lastseen::handle_lastseen_command;

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
    thinking_message: Option<String>,
    commands: HashMap<String, String>,
    keyword_triggers: Vec<(Vec<String>, String)>,
    crime_generator: CrimeFightingGenerator,
    gateway_bot_ids: Vec<u64>,
    google_search_enabled: bool,
}

impl Bot {
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
        bot_name: String,
        message_db: Option<Arc<tokio::sync::Mutex<Connection>>>,
        message_history_limit: usize,
        thinking_message: Option<String>,
        gateway_bot_ids: Vec<u64>,
        google_search_enabled: bool,
        gemini_rate_limit_minute: u32,
        gemini_rate_limit_day: u32,
    ) -> Self {
        // Define the commands the bot will respond to
        let mut commands = HashMap::new();
        commands.insert("hello".to_string(), "world!".to_string());
        
        // Generate a comprehensive help message with all commands
        let help_message = "Available commands:\n!help - Show this help message\n!hello - Say hello\n!buzz - Generate a corporate buzzword phrase\n!fightcrime - Generate a crime fighting duo\n!lastseen [name] - Find when a user was last active\n!quote [search_term] - Get a random quote\n!quote -show [show_name] - Get a random quote from a specific show\n!quote -dud [username] - Get a random message from a user\n!slogan [search_term] - Get a random advertising slogan\n!frinkiac [search_term] - Get a Simpsons screenshot from Frinkiac (or random if no term provided)\n!morbotron [search_term] - Get a Futurama screenshot from Morbotron (or random if no term provided)\n!masterofallscience [search_term] - Get a Rick and Morty screenshot from Master of All Science (or random if no term provided)";
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
                    gemini_rate_limit_day
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
            thinking_message,
            commands,
            keyword_triggers,
            crime_generator,
            gateway_bot_ids,
            google_search_enabled,
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
            if let Some((_, display_name, content)) = messages.first() {
                // Use the display_name::clean_display_name function for consistency
                let clean_display_name = display_name::clean_display_name(display_name);
                
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
        // Random interjection (2% chance - 1 in 50)
        if rand::thread_rng().gen_bool(0.02) {
            info!("Triggered random interjection (1 in 50 chance)");
            
            // Choose which type of interjection to make (MST3K quote, channel memory, or message pondering)
            let interjection_type = rand::thread_rng().gen_range(0..3);
            
            match interjection_type {
                0 => {
                    // MST3K Quote
                    info!("Random interjection: MST3K Quote");
                    let mst3k_quotes = [
                        "Watch out for snakes!",
                        "It's the amazing Rando!",
                        "Normal view... Normal view... NORMAL VIEW!",
                        "Hi-keeba!",
                        "I'm different!",
                        "Rowsdower!",
                        "Mitchell!",
                        "Deep hurting...",
                        "Trumpy, you can do magic things!",
                        "Torgo's theme intensifies",
                    ];
                    
                    let quote = mst3k_quotes.choose(&mut rand::thread_rng()).unwrap_or(&"I'm different!").to_string();
                    if let Err(e) = msg.channel_id.say(&ctx.http, quote).await {
                        error!("Error sending random MST3K quote: {:?}", e);
                    }
                },
                1 => {
                    // Channel Memory (quote something someone previously said)
                    info!("Random interjection: Channel Memory");
                    if let Some(db) = &self.message_db {
                        let db_clone = Arc::clone(db);
                        
                        // Query the database for a random message
                        let result = db_clone.lock().await.call(|conn| {
                            let query = "SELECT content FROM messages ORDER BY RANDOM() LIMIT 1";
                            let mut stmt = conn.prepare(query)?;
                            
                            let rows = stmt.query_map([], |row| {
                                Ok(row.get::<_, String>(0)?)
                            })?;
                            
                            let mut result = Vec::new();
                            for row in rows {
                                result.push(row?);
                            }
                            
                            Ok::<_, rusqlite::Error>(result)
                        }).await;
                        
                        match result {
                            Ok(messages) => {
                                if let Some(content) = messages.first() {
                                    if let Err(e) = msg.channel_id.say(&ctx.http, content).await {
                                        error!("Error sending random channel memory: {:?}", e);
                                    }
                                }
                            },
                            Err(e) => {
                                error!("Error querying database for random message: {:?}", e);
                            }
                        }
                    }
                },
                2 => {
                    // Message Pondering (respond to the last message with a thoughtful comment)
                    info!("Random interjection: Message Pondering");
                    let ponderings = [
                        "Hmm, that's an interesting point.",
                        "I was just thinking about that!",
                        "That reminds me of something...",
                        "I'm not sure I agree with that.",
                        "Fascinating perspective.",
                        "I've been pondering that very question.",
                        "That's what I've been saying all along!",
                        "I never thought of it that way before.",
                        "You know, that's actually quite profound.",
                        "Wait, what?",
                    ];
                    
                    let pondering = ponderings.choose(&mut rand::thread_rng()).unwrap_or(&"Hmm, interesting.").to_string();
                    if let Err(e) = msg.channel_id.say(&ctx.http, pondering).await {
                        error!("Error sending random pondering: {:?}", e);
                    }
                },
                _ => {} // Should never happen
            }
        }
        
        // Store the message in the database if available
        if let Some(db) = &self.message_db {
            let author = msg.author.name.clone();
            let display_name = get_best_display_name(ctx, msg).await;
            let content = msg.content.clone();
            let db_clone = db.clone();
            
            if let Err(e) = db_utils::save_message(db_clone, &author, &display_name, &content, Some(msg)).await {
                error!("Error storing message: {:?}", e);
            }
        }
        
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
                } else if command == "help" {
                    // Help command - use the help message from our commands HashMap
                    if let Some(help_text) = self.commands.get("help") {
                        if let Err(e) = msg.channel_id.say(&ctx.http, help_text).await {
                            error!("Error sending help message: {:?}", e);
                        }
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
                    let search_term = if parts.len() > 1 {
                        Some(parts[1..].join(" "))
                    } else {
                        None
                    };
                    
                    // Handle the frinkiac command
                    if let Err(e) = handle_frinkiac_command(&ctx.http, &msg, search_term, &self.frinkiac_client).await {
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
                    if let Err(e) = handle_morbotron_command(&ctx.http, &msg, search_term, &self.morbotron_client).await {
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
                    if let Err(e) = handle_masterofallscience_command(&ctx.http, &msg, search_term, &self.masterofallscience_client).await {
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
                    // Get the display name
                    let display_name = get_best_display_name(ctx, msg).await;
                    let clean_display_name = gemini_client.strip_pronouns(&display_name);
                    
                    // Check if we should use a thinking message
                    let should_use_thinking = match &self.thinking_message {
                        Some(message) if message.is_empty() || message == "[none]" => false,
                        Some(_) => true,
                        None => true // Default to using thinking message if not specified
                    };
                    
                    if should_use_thinking {
                        // Send a "thinking" message
                        let thinking_text = self.thinking_message.as_ref().unwrap_or(&"*thinking...*".to_string()).clone();
                        let mut thinking_msg = match msg.channel_id.say(&ctx.http, thinking_text).await {
                            Ok(msg) => msg,
                            Err(e) => {
                                error!("Error sending thinking message: {:?}", e);
                                return Ok(());
                            }
                        };
                        
                        // Get recent messages for context
                        let context_messages = if let Some(db) = &self.message_db {
                            // Get the last 5 messages from the database
                            match db_utils::get_recent_messages(db.clone(), 5).await {
                                Ok(messages) => messages,
                                Err(e) => {
                                    error!("Error retrieving recent messages: {:?}", e);
                                    Vec::new()
                                }
                            }
                        } else {
                            Vec::new()
                        };
                        
                        // Start typing indicator before making API call
                        if let Err(e) = msg.channel_id.broadcast_typing(&ctx.http).await {
                            error!("Failed to send typing indicator: {:?}", e);
                        }
                        
                        // Call the Gemini API with context
                        match gemini_client.generate_response_with_context(&content, &clean_display_name, &context_messages).await {
                            Ok(response) => {
                                // Apply realistic typing delay based on response length
                                apply_realistic_delay(&response, ctx, msg.channel_id).await;
                                
                                // Edit the thinking message with the actual response
                                if let Err(e) = thinking_msg.edit(&ctx.http, EditMessage::new().content(response.clone())).await {
                                    error!("Error editing thinking message: {:?}", e);
                                    // Try sending a new message if editing fails
                                    if let Err(e) = msg.channel_id.say(&ctx.http, "Sorry, I couldn't edit my message. Here's my response:").await {
                                        error!("Error sending fallback message: {:?}", e);
                                    }
                                    // Apply realistic typing delay based on response length
                                    apply_realistic_delay(&response, ctx, msg.channel_id).await;

                                    if let Err(e) = msg.channel_id.say(&ctx.http, response).await {
                                        error!("Error sending Gemini response: {:?}", e);
                                    }
                                }
                            },
                            Err(e) => {
                                error!("Error calling Gemini API: {:?}", e);
                                if let Err(e) = thinking_msg.edit(&ctx.http, EditMessage::new().content(format!("Sorry, I encountered an error: {}", e))).await {
                                    error!("Error editing thinking message: {:?}", e);
                                }
                            }
                        }
                    } else {
                        // Direct response without thinking message
                        let context_messages = if let Some(db) = &self.message_db {
                            match db_utils::get_recent_messages(db.clone(), 5).await {
                                Ok(messages) => messages,
                                Err(e) => {
                                    error!("Error retrieving recent messages: {:?}", e);
                                    Vec::new()
                                }
                            }
                        } else {
                            Vec::new()
                        };
                        
                        // Start typing indicator before making API call
                        if let Err(e) = msg.channel_id.broadcast_typing(&ctx.http).await {
                            error!("Failed to send typing indicator: {:?}", e);
                        }
                        
                        match gemini_client.generate_response_with_context(&content, &clean_display_name, &context_messages).await {
                            Ok(response) => {
                                // Apply realistic typing delay based on response length
                                apply_realistic_delay(&response, ctx, msg.channel_id).await;

                                // Create a message reference for replying
                                let message_reference = MessageReference::from(msg);
                                let mut create_message = CreateMessage::new().content(response.clone()).reference_message(message_reference);
                                
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
                                if let Err(e) = msg.channel_id.say(&ctx.http, format!("Sorry, I encountered an error: {}", e)).await {
                                    error!("Error sending error message: {:?}", e);
                                }
                            }
                        }
                    }
                } else {
                    // Fallback if Gemini API is not configured
                    let display_name = get_best_display_name(ctx, msg).await;
                    if let Err(e) = msg.channel_id.say(&ctx.http, format!("Hello {}, you called my name! I'm {}! (Gemini API is not configured)", display_name, self.bot_name)).await {
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
                    // Get the display name
                    let display_name = get_best_display_name(ctx, msg).await;
                    let clean_display_name = gemini_client.strip_pronouns(&display_name);
                    
                    // Check if we should use a thinking message
                    let should_use_thinking = match &self.thinking_message {
                        Some(message) if message.is_empty() || message == "[none]" => false,
                        Some(_) => true,
                        None => true // Default to using thinking message if not specified
                    };
                    
                    if should_use_thinking {
                        // Send a "thinking" message
                        let thinking_text = self.thinking_message.as_ref().unwrap_or(&"*thinking...*".to_string()).clone();
                        let mut thinking_msg = match msg.channel_id.say(&ctx.http, thinking_text).await {
                            Ok(msg) => msg,
                            Err(e) => {
                                error!("Error sending thinking message: {:?}", e);
                                return Ok(());
                            }
                        };
                        
                        // Call the Gemini API with user's display name
                        let context_messages = if let Some(db) = &self.message_db {
                            match db_utils::get_recent_messages(db.clone(), 5).await {
                                Ok(messages) => messages,
                                Err(e) => {
                                    error!("Error retrieving recent messages: {:?}", e);
                                    Vec::new()
                                }
                            }
                        } else {
                            Vec::new()
                        };
                        
                        match gemini_client.generate_response_with_context(&content, &clean_display_name, &context_messages).await {
                            Ok(response) => {
                                // Clone the response for editing
                                let response_clone = response.clone();
                                
                                // Apply realistic typing delay based on response length
                                apply_realistic_delay(&response_clone, ctx, msg.channel_id).await;
                                
                                // Edit the thinking message with the actual response
                                if let Err(e) = thinking_msg.edit(&ctx.http, EditMessage::new().content(response_clone)).await {
                                error!("Error editing thinking message: {:?}", e);
                                // Try sending a new message if editing fails
                                if let Err(e) = msg.channel_id.say(&ctx.http, "Sorry, I couldn't edit my message. Here's my response:").await {
                                    error!("Error sending fallback message: {:?}", e);
                                }
                                if let Err(e) = msg.channel_id.say(&ctx.http, response).await {
                                    error!("Error sending Gemini response: {:?}", e);
                                }
                            }
                        },
                        Err(e) => {
                            error!("Error calling Gemini API: {:?}", e);
                            if let Err(e) = thinking_msg.edit(&ctx.http, EditMessage::new().content(format!("Sorry, I encountered an error: {}", e))).await {
                                error!("Error editing thinking message: {:?}", e);
                            }
                        }
                    }
                    } else {
                        // Direct response without thinking message
                        let context_messages = if let Some(db) = &self.message_db {
                            match db_utils::get_recent_messages(db.clone(), 5).await {
                                Ok(messages) => messages,
                                Err(e) => {
                                    error!("Error retrieving recent messages: {:?}", e);
                                    Vec::new()
                                }
                            }
                        } else {
                            Vec::new()
                        };
                        
                        // Start typing indicator before making API call
                        if let Err(e) = msg.channel_id.broadcast_typing(&ctx.http).await {
                            error!("Failed to send typing indicator: {:?}", e);
                        }
                        
                        match gemini_client.generate_response_with_context(&content, &clean_display_name, &context_messages).await {
                            Ok(response) => {
                                // Apply realistic typing delay based on response length
                                apply_realistic_delay(&response, ctx, msg.channel_id).await;
                                
                                // Create a message reference for replying
                                let message_reference = MessageReference::from(msg);
                                let mut create_message = CreateMessage::new().content(response.clone()).reference_message(message_reference);
                                
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
                                if let Err(e) = msg.channel_id.say(&ctx.http, format!("Sorry, I encountered an error: {}", e)).await {
                                    error!("Error sending error message: {:?}", e);
                                }
                            }
                        }
                    }
                } else {
                    // Fallback if Gemini API is not configured
                    let display_name = get_best_display_name(ctx, msg).await;
                    if let Err(e) = msg.channel_id.say(&ctx.http, format!("Hello {}, you mentioned me! I'm {}! (Gemini API is not configured)", display_name, self.bot_name)).await {
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
        // Check if the message is from a bot
        if msg.author.bot {
            // Add detailed logging for bot messages
            let bot_id = msg.author.id.get();
            info!("📝 Received message from bot ID: {} ({})", bot_id, msg.author.name);
            info!("📝 Gateway bot IDs configured: {:?}", self.gateway_bot_ids);
            info!("📝 Is this bot in our gateway list? {}", self.gateway_bot_ids.contains(&bot_id));
            info!("📝 Message content: {}", msg.content);
            
            if !self.gateway_bot_ids.contains(&bot_id) {
                // Not in our gateway bot list, ignore the message
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
        
        // Process the message
        if let Err(e) = self.process_message(&ctx, &msg).await {
            error!("Error processing message: {:?}", e);
        }
    }

    // Handle message updates (edits)
    async fn message_update(&self, _ctx: Context, _old_if_available: Option<Message>, _new: Option<Message>, event: MessageUpdateEvent) {
        // Only process messages in the followed channels
        if !self.followed_channels.contains(&event.channel_id) {
            return;
        }
        
        // We need both the message ID and the new content to update the database
        if let Some(new_content) = event.content {
            let message_id = event.id.to_string();
            
            // Update the message in the database
            if let Some(db) = &self.message_db {
                match db_utils::update_message(db.clone(), message_id, new_content).await {
                    Ok(_) => {
                        info!("Updated message {} in database", event.id);
                    },
                    Err(e) => {
                        error!("Error updating message in database: {:?}", e);
                    }
                }
            }
        }
    }

    async fn ready(&self, _: Context, ready: Ready) {
        info!("✅ {} ({}) is connected and following {} channels!", 
              self.bot_name, ready.user.name, self.followed_channels.len());
        
        // Log each followed channel
        for channel_id in &self.followed_channels {
            info!("Following channel: {}", channel_id);
        }
        
        info!("Bot is ready to respond to messages in the configured channels");
        
        // Log available commands
        let command_list = self.commands.keys()
            .map(|k| format!("!{}", k))
            .collect::<Vec<_>>()
            .join(", ");
        info!("Available commands: {}", command_list);
        
        // Log keyword triggers
        info!("Keyword triggers:");
        for (keywords, _) in &self.keyword_triggers {
            info!("  - {}", keywords.join(" + "));
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
                if channel.name.to_lowercase() == name.to_lowercase() {
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
    let (bot_name, message_history_limit, db_trim_interval, gemini_rate_limit_minute, gemini_rate_limit_day, gateway_bot_ids, google_search_enabled) = 
        parse_config(&config);
    
    // Get Gemini API key
    let gemini_api_key = config.gemini_api_key.clone();
    if gemini_api_key.is_none() {
        error!("Gemini API key not found in config");
    } else {
        info!("Gemini API key loaded");
    }
    
    // Get custom prompt wrapper if available
    let gemini_prompt_wrapper = config.gemini_prompt_wrapper.clone();
    
    // Set thinking message to None to disable it
    let thinking_message = None;
    info!("Thinking message disabled, using typing indicator instead");
    
    // Get custom Gemini API endpoint if available
    let gemini_api_endpoint = config.gemini_api_endpoint.clone();
    if let Some(endpoint) = &gemini_api_endpoint {
        info!("Using custom Gemini API endpoint: {}", endpoint);
    }

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
    
    // Create a new bot instance with the valid channel IDs
    let bot = Bot::new(
        channel_ids.clone(),
        config.db_host.clone(),
        config.db_name.clone(),
        config.db_user.clone(),
        config.db_password.clone(),
        None, // No Google API key needed anymore
        None, // No Google Search Engine ID needed anymore
        gemini_api_key,
        gemini_api_endpoint,
        gemini_prompt_wrapper,
        bot_name.clone(),
        message_db.clone(),
        message_history_limit,
        thinking_message,
        gateway_bot_ids.clone(),
        google_search_enabled,
        gemini_rate_limit_minute,
        gemini_rate_limit_day
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
            
            if let Err(e) = db_utils::load_message_history(db_clone, &mut temp_history, message_history_limit).await {
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
            
            if let Err(e) = db_utils::load_message_history(db_clone, &mut temp_history, message_history_limit).await {
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
    info!("Press Ctrl+C to stop the bot");
    client.start().await?;

    Ok(())
}
