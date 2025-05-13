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
use tokio::sync::RwLock;
use tracing::{error, info};
use tokio_rusqlite::Connection;
use rand::seq::SliceRandom;
use rand::Rng;

// Import modules
mod db_utils;
mod config;
mod database;
mod google_search;
mod gemini_api;
mod crime_fighting;
mod rate_limiter;

// Use our modules
use config::{load_config, parse_config};
use database::DatabaseManager;
use google_search::GoogleSearchClient;
use gemini_api::GeminiClient;
use crime_fighting::CrimeFightingGenerator;

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
        commands.insert("help".to_string(), "Available commands:\n!hello - Say hello\n!help - Show this help message\n!fightcrime - Generate a crime fighting duo\n!quote [search_term] - Get a random quote\n!quote -show [show_name] - Get a random quote from a specific show\n!quote -dud [username] - Get a random message from a user\n!slogan [search_term] - Get a random advertising slogan".to_string());
        
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
        
        Self {
            followed_channels,
            db_manager,
            google_client,
            gemini_client,
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
    async fn generate_crime_fighting_duo(&self, ctx: &Context) -> Result<String> {
        // Try to get the list of recent speakers, but use defaults if anything fails
        let data = ctx.data.read().await;
        
        // Default names to use if we can't get real speakers
        let default_speaker1 = "Anonymous Coward".to_string();
        let default_speaker2 = "Redacted".to_string();
        
        // Try to get real speaker names, but fall back to defaults at any error
        let (speaker1, speaker2) = match data.get::<RecentSpeakersKey>() {
            Some(recent_speakers_lock) => {
                match recent_speakers_lock.try_read() {
                    Ok(recent_speakers) => {
                        if recent_speakers.len() >= 2 {
                            // Select two random speakers
                            let mut rng = rand::thread_rng();
                            let speaker_indices: Vec<usize> = (0..recent_speakers.len()).collect();
                            let selected_indices = speaker_indices.choose_multiple(&mut rng, 2).collect::<Vec<&usize>>();
                            
                            // Use display names
                            (recent_speakers[*selected_indices[0]].1.clone(), 
                             recent_speakers[*selected_indices[1]].1.clone())
                        } else {
                            info!("Not enough recent speakers, using default names for crime fighting duo");
                            (default_speaker1, default_speaker2)
                        }
                    },
                    Err(_) => {
                        error!("Could not read recent speakers lock, using default names");
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
                // Clean up display name - remove <> brackets and [irc] tag
                let mut clean_display_name = display_name.clone();
                clean_display_name = clean_display_name.replace("<", "").replace(">", "");
                clean_display_name = clean_display_name.replace("[irc]", "").trim().to_string();
                
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
        
        // Check for the bot's name as a standalone word followed by punctuation
        // This requires word boundary checking to avoid matching substrings
        let name_with_word_boundary = format!(r"\b{}\b", regex::escape(bot_name));
        if let Ok(re) = regex::Regex::new(&name_with_word_boundary) {
            if re.is_match(&content_lower) {
                // The bot name appears as a complete word
                // Now check if it's followed by appropriate punctuation
                let name_followed_by_punctuation = [
                    format!(r"\b{}\s", bot_name),      // Name followed by space
                    format!(r"\b{}\?", bot_name),      // Name followed by question mark
                    format!(r"\b{}!", bot_name),       // Name followed by exclamation
                    format!(r"\b{},", bot_name),       // Name followed by comma
                    format!(r"\b{}:", bot_name),       // Name followed by colon
                ];
                
                for pattern in &name_followed_by_punctuation {
                    if let Ok(re) = regex::Regex::new(pattern) {
                        if re.is_match(&content_lower) {
                            info!("Bot addressed: name followed by punctuation '{}'", pattern);
                            return true;
                        }
                    }
                }
            }
        }
        
        // Check for common phrases that directly address the bot
        let direct_address_patterns = [
            format!(r"\b{} can you\b", bot_name),
            format!(r"\b{} could you\b", bot_name),
            format!(r"\b{} will you\b", bot_name),
            format!(r"\b{} would you\b", bot_name),
            format!(r"\b{} please\b", bot_name),
            format!(r"\bask {}\b", bot_name),
            format!(r"\btell {}\b", bot_name),
        ];
        
        for pattern in &direct_address_patterns {
            if let Ok(re) = regex::Regex::new(&pattern) {
                if re.is_match(&content_lower) {
                    info!("Bot addressed: direct address pattern '{}'", pattern);
                    return true;
                }
            }
        }
        
        // Negative cases - when the bot name is mentioned but not addressed
        // These patterns indicate the bot is being talked about, not to
        let negative_patterns = [
            format!(r"\b{} is\b", bot_name),           // Talking about the bot
            format!(r"\bthe {}\b", bot_name),          // Referring to the bot
            format!(r"\bthat {}\b", bot_name),         // Referring to the bot
            format!(r"\bthis {}\b", bot_name),         // Referring to the bot
            format!(r"\ba {}\b", bot_name),            // Referring to a crow
            format!(r"\ban {}\b", bot_name),           // Referring to a crow
            format!(r"\bmy {}\b", bot_name),           // Referring to someone's crow
            format!(r"\bhis {}\b", bot_name),          // Referring to someone's crow
            format!(r"\bher {}\b", bot_name),          // Referring to someone's crow
            format!(r"\btheir {}\b", bot_name),        // Referring to someone's crow
            format!(r"\babout {}\b", bot_name),        // Talking about crows
            format!(r"\bstupid {}\b", bot_name),       // Negative reference
            format!(r"\bdumb {}\b", bot_name),         // Negative reference
            format!(r"\blike a {}\b", bot_name),       // Comparison
            format!(r"\bas a {}\b", bot_name),         // Comparison
        ];
        
        // First check if the bot name appears as a complete word
        if let Ok(re) = regex::Regex::new(&name_with_word_boundary) {
            if re.is_match(&content_lower) {
                // Check negative patterns
                for pattern in &negative_patterns {
                    if let Ok(re) = regex::Regex::new(pattern) {
                        if re.is_match(&content_lower) {
                            info!("Bot mentioned but not addressed: matched negative pattern '{}'", pattern);
                            return false;
                        }
                    }
                }
                
                // If the bot name appears as a standalone word at the beginning of the message,
                // it's likely being addressed
                if content_lower.trim().starts_with(bot_name) {
                    info!("Bot addressed: name at beginning of trimmed message");
                    return true;
                }
                
                // If the bot name is a standalone word and not matched by negative patterns,
                // we need more context to determine if it's being addressed
                // For now, we'll be conservative and NOT assume it's being addressed
                info!("Bot name found as standalone word, but not clearly addressed");
                return false;
            }
        }
        
        // If we get here, the bot name might be part of another word or not present at all
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
            let display_name = msg.author.global_name.clone().unwrap_or_else(|| msg.author.name.clone());
            let content = msg.content.clone();
            let db_clone = db.clone();
            
            if let Err(e) = db_utils::save_message(db_clone, &author, &display_name, &content).await {
                error!("Error storing message: {:?}", e);
            }
        }
        
        // Update recent speakers list
        {
            let data = ctx.data.read().await;
            if let Some(recent_speakers) = data.get::<RecentSpeakersKey>() {
                let mut speakers = recent_speakers.write().await;
                let username = msg.author.name.clone();
                // Use the username as display name since User doesn't have a display_name method
                let display_name = msg.author.global_name.clone().unwrap_or_else(|| msg.author.name.clone());
                
                // Check if user is already in the list
                if !speakers.iter().any(|(name, _)| name == &username) {
                    if speakers.len() >= 5 {
                        speakers.pop_front();
                    }
                    speakers.push_back((username, display_name));
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
                
                if command == "slogan" {
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
                    match self.generate_crime_fighting_duo(&ctx).await {
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
                        // Format and send the result
                        let response = format!("**{}**\n{}\n{}", result.title, result.url, result.snippet);
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
                        error!("Error performing Google search: {:?}", e);
                        if let Err(e) = msg.channel_id.say(&ctx.http, "Error performing search.").await {
                            error!("Error sending error message: {:?}", e);
                        }
                    }
                }
            } else {
                if let Err(e) = msg.channel_id.say(&ctx.http, "Google search is not configured.").await {
                    error!("Error sending search error: {:?}", e);
                }
            }
            return Ok(());
        }
        
        // Check if the bot is being addressed using our new function
        if self.is_bot_addressed(&msg.content) {
            // Use the full message content including the bot's name
            let content = msg.content.trim().to_string();
            
            if !content.is_empty() {
                if let Some(gemini_client) = &self.gemini_client {
                    // Get the display name
                    let display_name = msg.author.global_name.clone().unwrap_or_else(|| msg.author.name.clone());
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
                        
                        // Call the Gemini API
                        match gemini_client.generate_response(&content, &clean_display_name).await {
                            Ok(response) => {
                                // Edit the thinking message with the actual response
                                if let Err(e) = thinking_msg.edit(&ctx.http, EditMessage::new().content(response.clone())).await {
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
                        match gemini_client.generate_response(&content, &clean_display_name).await {
                            Ok(response) => {
                                if let Err(e) = msg.channel_id.say(&ctx.http, response).await {
                                    error!("Error sending Gemini response: {:?}", e);
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
                    let display_name = msg.author.global_name.clone().unwrap_or_else(|| msg.author.name.clone());
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
            match self.generate_crime_fighting_duo(&ctx).await {
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
                    let display_name = msg.author.global_name.clone().unwrap_or_else(|| msg.author.name.clone());
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
                        match gemini_client.generate_response(&content, &clean_display_name).await {
                            Ok(response) => {
                                // Edit the thinking message with the actual response
                                let response_clone = response.clone();
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
                        match gemini_client.generate_response(&content, &clean_display_name).await {
                            Ok(response) => {
                                if let Err(e) = msg.channel_id.say(&ctx.http, response).await {
                                    error!("Error sending Gemini response: {:?}", e);
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
                    let display_name = msg.author.global_name.clone().unwrap_or_else(|| msg.author.name.clone());
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
            .map(|k| k.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        info!("Available commands: !{}", command_list);
        
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
    
    // Get thinking message if available
    let thinking_message = config.thinking_message.clone();
    if let Some(message) = &thinking_message {
        info!("Using custom thinking message: {}", message);
    }
    
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
            Some(Arc::new(tokio::sync::Mutex::new(conn)))
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
        
        data.insert::<RecentSpeakersKey>(recent_speakers);
        data.insert::<MessageHistoryKey>(message_history);
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
