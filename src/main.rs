use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use anyhow::{Context as AnyhowContext, Result};
use serenity::all::*;
use serenity::async_trait;
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::model::id::ChannelId;
use serenity::prelude::*;
use tokio::sync::RwLock;
use tracing::{error, info};
use tokio_rusqlite::Connection;
use serde::Deserialize;
use rand::Rng;
use rand::seq::SliceRandom;
use mysql::{Pool, OptsBuilder, prelude::*};
use reqwest;
use serde_json;

// Import database utility functions
mod db_utils;
// Define keys for the client data
struct RecentSpeakersKey;
impl TypeMapKey for RecentSpeakersKey {
    type Value = Arc<RwLock<VecDeque<(String, String)>>>;  // (username, display_name)
}

struct MessageHistoryKey;
impl TypeMapKey for MessageHistoryKey {
    type Value = Arc<RwLock<VecDeque<Message>>>;
}

#[derive(Debug, Deserialize, Clone)]
struct Config {
    discord_token: String,
    followed_channel_name: Option<String>,
    followed_channel_id: Option<String>,
    followed_server_name: Option<String>,
    bot_name: Option<String>,
    message_history_limit: Option<String>,
    db_trim_interval_secs: Option<String>,
    gemini_rate_limit_minute: Option<String>,
    gemini_rate_limit_day: Option<String>,
    gemini_api_key: Option<String>,
    gemini_api_endpoint: Option<String>,
    gemini_prompt_wrapper: Option<String>,
    google_api_key: Option<String>,
    google_search_engine_id: Option<String>,
    db_host: Option<String>,
    db_name: Option<String>,
    db_user: Option<String>,
    db_password: Option<String>,
}

// Create a DatabaseManager struct
struct DatabaseManager {
    pool: Option<Pool>,
}

impl DatabaseManager {
    fn new(host: Option<String>, db: Option<String>, user: Option<String>, password: Option<String>) -> Self {
        info!("Creating DatabaseManager with host={:?}, db={:?}, user={:?}, password={}",
              host, db, user, if password.is_some() { "provided" } else { "not provided" });
        
        let pool = if let (Some(host), Some(db), Some(user), Some(password)) = 
            (&host, &db, &user, &password) {
            info!("All database credentials provided, attempting to connect to MySQL");
            let opts = OptsBuilder::new()
                .ip_or_hostname(Some(host.clone()))
                .db_name(Some(db.clone()))
                .user(Some(user.clone()))
                .pass(Some(password.clone()));
                
            match Pool::new(opts) {
                Ok(pool) => {
                    info!("✅ Successfully created MySQL connection pool");
                    // Test the connection with a simple query
                    match pool.get_conn() {
                        Ok(mut conn) => {
                            match conn.query_first::<String, _>("SELECT 'Connection test'") {
                                Ok(_) => info!("✅ MySQL connection test successful"),
                                Err(e) => error!("❌ MySQL connection test failed: {:?}", e),
                            }
                        },
                        Err(e) => error!("❌ Could not get MySQL connection: {:?}", e),
                    }
                    Some(pool)
                },
                Err(e) => {
                    error!("❌ Failed to create MySQL connection pool: {:?}", e);
                    None
                }
            }
        } else {
            let missing = vec![
                if host.is_none() { "host" } else { "" },
                if db.is_none() { "database" } else { "" },
                if user.is_none() { "user" } else { "" },
                if password.is_none() { "password" } else { "" },
            ].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
            
            error!("❌ MySQL database connection not configured - missing: {}", missing);
            None
        };
        
        Self { pool }
    }
    
    // Add this method to check if the database is configured
    fn is_configured(&self) -> bool {
        self.pool.is_some()
    }
    
    // Add this method to test the connection
    fn test_connection(&self) -> Result<bool> {
        if let Some(pool) = &self.pool {
            match pool.get_conn() {
                Ok(mut conn) => {
                    match conn.query_first::<String, _>("SELECT 'Connection test'") {
                        Ok(_) => {
                            info!("✅ MySQL connection test successful");
                            Ok(true)
                        },
                        Err(e) => {
                            error!("❌ MySQL connection test failed: {:?}", e);
                            Ok(false)
                        }
                    }
                },
                Err(e) => {
                    error!("❌ Could not get MySQL connection: {:?}", e);
                    Ok(false)
                }
            }
        } else {
            error!("❌ Cannot test connection - MySQL pool is None");
            Ok(false)
        }
    }
    
    async fn query_random_entry(&self, http: &Http, msg: &Message, search_term: Option<String>, show_name: Option<String>, entry_type: &str) -> Result<()> {
        // Check if we have MySQL connection info
        if self.pool.is_none() {
            error!("❌ MySQL pool is None when handling {} command", entry_type);
            msg.channel_id.say(http, "MySQL database is not configured.").await?;
            return Ok(());
        }
        
        info!("MySQL pool exists, attempting to get connection for {} command", entry_type);
        
        // Get a connection from the pool
        let pool = self.pool.as_ref().unwrap();
        let mut conn = match pool.get_conn() {
            Ok(conn) => {
                info!("✅ Successfully got MySQL connection for {} command", entry_type);
                conn
            },
            Err(e) => {
                error!("❌ Failed to get MySQL connection for {} command: {:?}", entry_type, e);
                msg.channel_id.say(http, format!("Failed to connect to the {} database.", entry_type)).await?;
                return Ok(());
            }
        };
        
        // Build the WHERE clause based on search term
        let where_clause = if let Some(terms) = &search_term {
            // Split the search terms and join with % for LIKE query
            let terms: Vec<&str> = terms.split_whitespace().collect();
            if !terms.is_empty() {
                let search_pattern = format!("%{}%", terms.join("%"));
                search_pattern
            } else {
                "%".to_string()
            }
        } else {
            "%".to_string()
        };
        
        // Build the show clause based on show name
        let show_clause = if let Some(show) = &show_name {
            // Split the show name and join with % for LIKE query
            let show_terms: Vec<&str> = show.split_whitespace().collect();
            if !show_terms.is_empty() {
                let show_pattern = format!("%{}%", show_terms.join("%"));
                show_pattern
            } else {
                "%".to_string()
            }
        } else {
            "%".to_string()
        };
        
        // Determine which table and column to use based on entry_type
        match entry_type {
            "quote" => {
                // For quotes, we need to join with masterlist_shows to filter by show name
                info!("Executing quote query with where_clause: {} and show_clause: {}", where_clause, show_clause);
                
                // Count total matching quotes
                let count_query = "SELECT COUNT(*) FROM masterlist_quotes, masterlist_episodes, masterlist_shows \
                                  WHERE masterlist_episodes.show_id = masterlist_shows.show_id \
                                  AND masterlist_quotes.show_id = masterlist_shows.show_id \
                                  AND masterlist_quotes.show_ep = masterlist_episodes.show_ep \
                                  AND quote LIKE ? AND show_title LIKE ?";
                
                let total_entries = match conn.exec_first::<i64, _, _>(
                    count_query,
                    (where_clause.clone(), show_clause.clone())
                ) {
                    Ok(Some(count)) => {
                        info!("Found {} matching quotes with show filter", count);
                        count
                    },
                    Ok(None) => {
                        info!("No matching quotes found with show filter");
                        0
                    },
                    Err(e) => {
                        error!("Failed to count quotes: {:?}", e);
                        msg.channel_id.say(http, "Failed to query the quote database.").await?;
                        return Ok(());
                    }
                };
                
                if total_entries == 0 {
                    let mut message = "No quotes found".to_string();
                    if let Some(terms) = &search_term {
                        message.push_str(&format!(" matching '{}'", terms));
                    }
                    if let Some(show) = &show_name {
                        message.push_str(&format!(" in show '{}'", show));
                    }
                    msg.channel_id.say(http, message).await?;
                    return Ok(());
                }
                
                // Get a random quote
                let random_index = rand::thread_rng().gen_range(0..total_entries);
                info!("Selected random index {} of {} for quotes", random_index, total_entries);
                
                let select_query = "SELECT quote, show_title, masterlist_episodes.show_ep, title \
                                   FROM masterlist_quotes, masterlist_episodes, masterlist_shows \
                                   WHERE masterlist_episodes.show_id = masterlist_shows.show_id \
                                   AND masterlist_quotes.show_id = masterlist_shows.show_id \
                                   AND masterlist_quotes.show_ep = masterlist_episodes.show_ep \
                                   AND quote LIKE ? AND show_title LIKE ? \
                                   LIMIT ?, 1";
                
                let quote_result = conn.exec_first::<(String, String, String, String), _, _>(
                    select_query,
                    (where_clause, show_clause, random_index)
                );
                
                // Format and send the quote
                match quote_result {
                    Ok(Some((quote_text, show_title, episode_num, episode_title))) => {
                        // Clean up HTML entities
                        let clean_quote = html_escape::decode_html_entities(&quote_text);
                        
                        msg.channel_id.say(http, format!("(Quote {} of {}) {} -- {} {}: {}", 
                            random_index + 1, total_entries, clean_quote, show_title, episode_num, episode_title)).await?;
                    },
                    Ok(None) => {
                        error!("Query returned no results despite count being {}", total_entries);
                        msg.channel_id.say(http, "No quotes found.").await?;
                    },
                    Err(e) => {
                        error!("Failed to query quote: {:?}", e);
                        msg.channel_id.say(http, "Failed to retrieve a quote from the database.").await?;
                    }
                }
            },
            "slogan" => {
                // For slogans, we use the simple query as before
                info!("Executing slogan query with where_clause: {}", where_clause);
                
                // Count total matching slogans
                let total_entries = match conn.exec_first::<i64, _, _>(
                    "SELECT COUNT(*) FROM nuke_quotes WHERE pn_quote LIKE ?",
                    (where_clause.clone(),)
                ) {
                    Ok(Some(count)) => {
                        info!("Found {} matching slogans", count);
                        count
                    },
                    Ok(None) => {
                        info!("No matching slogans found");
                        0
                    },
                    Err(e) => {
                        error!("Failed to count slogans: {:?}", e);
                        msg.channel_id.say(http, "Failed to query the slogan database.").await?;
                        return Ok(());
                    }
                };
                
                if total_entries == 0 {
                    if let Some(terms) = &search_term {
                        msg.channel_id.say(http, format!("No slogans match '{}'", terms)).await?;
                    } else {
                        msg.channel_id.say(http, "No slogans found.").await?;
                    }
                    return Ok(());
                }
            },
            _ => {
                error!("Unknown entry type: {}", entry_type);
                msg.channel_id.say(http, "Unknown database query type.").await?;
                return Ok(());
            }
        }
        
        Ok(())
    }
}

struct Bot {
    followed_channel: ChannelId,
    db_manager: DatabaseManager,
    google_api_key: Option<String>,
    google_search_engine_id: Option<String>,
    gemini_api_key: Option<String>,
    gemini_api_endpoint: Option<String>,
    bot_name: String,
    message_db: Option<Arc<tokio::sync::Mutex<Connection>>>,
    message_history_limit: usize,
    gemini_prompt_wrapper: String,
    commands: HashMap<String, String>,
    keyword_triggers: Vec<(Vec<String>, String)>,
}

impl Bot {
    fn new(
        followed_channel: ChannelId,
        mysql_host: Option<String>,
        mysql_db: Option<String>,
        mysql_user: Option<String>,
        mysql_password: Option<String>,
        google_api_key: Option<String>,
        google_search_engine_id: Option<String>,
        gemini_api_key: Option<String>,
        gemini_api_endpoint: Option<String>,
        bot_name: String,
        message_db: Option<Arc<tokio::sync::Mutex<Connection>>>,
        message_history_limit: usize,
    ) -> Self {
        // Define the commands the bot will respond to
        let mut commands = HashMap::new();
        commands.insert("hello".to_string(), "world!".to_string());
        commands.insert("help".to_string(), "Available commands:\n!hello - Say hello\n!help - Show this help message\n!fightcrime - Generate a crime fighting duo\n!quote [search_term] - Get a random quote\n!quote -show [show_name] - Get a random quote from a specific show\n!quote -dud [username] - Get a random message from a user\n!slogan [search_term] - Get a random advertising slogan".to_string());
        
        // Define keyword triggers
        let mut keyword_triggers = Vec::new();
        keyword_triggers.push((vec!["magic".to_string(), "voice".to_string()], 
                              "I heard someone talking about magic voice!".to_string()));
        keyword_triggers.push((vec!["discord".to_string(), "bot".to_string()], 
                              format!("Yes, I'm a Discord bot! My name is {}!", bot_name)));
        keyword_triggers.push((vec!["who".to_string(), "fights".to_string(), "crime".to_string()], 
                              "CRIME_FIGHTING_DUO".to_string()));
        keyword_triggers.push((vec!["lisa".to_string(), "needs".to_string(), "braces".to_string()], 
                              "DENTAL PLAN!".to_string()));
        
        // Create database manager
        let db_manager = DatabaseManager::new(mysql_host.clone(), mysql_db.clone(), mysql_user.clone(), mysql_password.clone());
        info!("Database manager created, is configured: {}", db_manager.is_configured());
        
        Self {
            followed_channel,
            db_manager,
            google_api_key,
            google_search_engine_id,
            gemini_api_key,
            gemini_api_endpoint,
            bot_name,
            message_db,
            message_history_limit,
            gemini_prompt_wrapper: "You are {bot_name}, a helpful and friendly Discord bot. Respond to {user}: {message}".to_string(),
            commands,
            keyword_triggers,
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
        
        // Generate random descriptions
        let mut rng = rand::thread_rng();
        
        let descriptions1 = [
            "a superhumanly strong",
            "a brilliant but troubled",
            "a time-traveling",
            "a genetically enhanced",
            "a cybernetically augmented",
            "a telepathic",
            "a shape-shifting",
            "a dimension-hopping",
            "a technologically advanced",
            "a magically empowered",
        ];
        
        let occupations1 = [
            "former detective",
            "ex-spy",
            "disgraced scientist",
            "retired superhero",
            "rogue AI researcher",
            "reformed villain",
            "exiled royal",
            "amnesiac assassin",
            "interdimensional refugee",
            "time-displaced warrior",
        ];
        
        let traits1 = [
            "with a mysterious past",
            "with a score to settle",
            "with nothing left to lose",
            "with a secret identity",
            "with supernatural abilities",
            "with advanced martial arts training",
            "with a tragic backstory",
            "with a vendetta against crime",
            "with a photographic memory",
            "with unfinished business",
        ];
        
        let descriptions2 = [
            "a sarcastic",
            "a no-nonsense",
            "a radical",
            "a by-the-book",
            "a rebellious",
            "a tech-savvy",
            "a streetwise",
            "a wealthy",
            "a mysterious",
            "an eccentric",
        ];
        
        let occupations2 = [
            "hacker",
            "martial artist",
            "forensic scientist",
            "archaeologist",
            "journalist",
            "medical examiner",
            "weapons expert",
            "psychologist",
            "conspiracy theorist",
            "paranormal investigator",
        ];
        
        let traits2 = [
            "with a secret technique",
            "with a passion for justice",
            "with unconventional methods",
            "with a troubled past",
            "with powerful connections",
            "with a unique perspective",
            "with specialized equipment",
            "with a hidden agenda",
            "with incredible luck",
            "with unwavering determination",
        ];
        
        // Select random descriptions
        let desc1 = descriptions1.choose(&mut rng).unwrap();
        let occ1 = occupations1.choose(&mut rng).unwrap();
        let trait1 = traits1.choose(&mut rng).unwrap();
        
        let desc2 = descriptions2.choose(&mut rng).unwrap();
        let occ2 = occupations2.choose(&mut rng).unwrap();
        let trait2 = traits2.choose(&mut rng).unwrap();
        
        // Format the crime fighting duo description
        let duo_description = format!(
            "{} is {} {} {}. {} is {} {} {}. They fight crime!",
            speaker1, desc1, occ1, trait1,
            speaker2, desc2, occ2, trait2
        );
        
        Ok(duo_description)
    }
    
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
                    let mut stmt = conn.prepare(
                        "SELECT author, content FROM messages WHERE author = ? ORDER BY RANDOM() LIMIT 1"
                    )?;
                    
                    let rows = stmt.query_map([&user_clone], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })?;
                    
                    let mut result = Vec::new();
                    for row in rows {
                        result.push(row?);
                    }
                    
                    Ok::<_, rusqlite::Error>(result)
                }).await?
            } else {
                info!("Quote -dud request for random user");
                
                // Query the database for a random message from any user
                db_clone.lock().await.call(move |conn| {
                    let mut stmt = conn.prepare(
                        "SELECT author, content FROM messages ORDER BY RANDOM() LIMIT 1"
                    )?;
                    
                    let rows = stmt.query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })?;
                    
                    let mut result = Vec::new();
                    for row in rows {
                        result.push(row?);
                    }
                    
                    Ok::<_, rusqlite::Error>(result)
                }).await?
            };
            
            // If we found a message, send it
            if let Some((author, content)) = messages.first() {
                msg.channel_id.say(http, format!("<{}> {}", author, content)).await?;
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
    
    // Process a message
    async fn process_message(&self, ctx: &Context, msg: &Message) -> Result<()> {
        // Store the message in the database if available
        if let Some(db) = &self.message_db {
            let author = msg.author.name.clone();
            let content = msg.content.clone();
            let db_clone = db.clone();
            
            if let Err(e) = db_utils::save_message(db_clone, &author, &content).await {
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
        if msg.content.to_lowercase().starts_with("google ") && msg.content.len() > 7 {
            let query = &msg.content[7..];
            
            if let (Some(_api_key), Some(_search_engine_id)) = (&self.google_api_key, &self.google_search_engine_id) {
                if let Err(e) = msg.channel_id.say(&ctx.http, format!("Searching for: {}", query)).await {
                    error!("Error sending search confirmation: {:?}", e);
                }
                
                // TODO: Implement Google search functionality
            } else {
                if let Err(e) = msg.channel_id.say(&ctx.http, "Google search is not configured.").await {
                    error!("Error sending search error: {:?}", e);
                }
            }
            return Ok(());
        }
        
        // Check if message starts with the bot's name
        let content_lower = msg.content.to_lowercase();
        let bot_name_lower = self.bot_name.to_lowercase();
        
        if content_lower.starts_with(&bot_name_lower) {
            // Extract the message content without the bot's name
            let content = msg.content[self.bot_name.len()..].trim().to_string();
            
            if !content.is_empty() {
                if let Some(_api_key) = &self.gemini_api_key {
                    // Send a "thinking" message
                    let mut thinking_msg = match msg.channel_id.say(&ctx.http, "*thinking...*").await {
                        Ok(msg) => msg,
                        Err(e) => {
                            error!("Error sending thinking message: {:?}", e);
                            return Ok(());
                        }
                    };
                    
                    // Call the Gemini API with user's display name
                    let user_name = &msg.author.name;
                    match self.call_gemini_api_with_user(&content, user_name).await {
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
                    // Fallback if Gemini API is not configured
                    if let Err(e) = msg.channel_id.say(&ctx.http, format!("Hello {}, you called my name! I'm {}! (Gemini API is not configured)", msg.author.name, self.bot_name)).await {
                        error!("Error sending name response: {:?}", e);
                    }
                }
                return Ok(());
            }
        }
        
        // Check for keyword triggers
        let content_lower = msg.content.to_lowercase();
        
        for (keywords, response) in &self.keyword_triggers {
            if keywords.iter().all(|keyword| content_lower.contains(&keyword.to_lowercase())) {
                // Special handling for "who fights crime"
                if response == "CRIME_FIGHTING_DUO" {
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
                } else {
                    if let Err(e) = msg.channel_id.say(&ctx.http, response).await {
                        error!("Error sending keyword response: {:?}", e);
                    }
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
                if let Some(_api_key) = &self.gemini_api_key {
                    // Send a "thinking" message
                    let mut thinking_msg = match msg.channel_id.say(&ctx.http, "*thinking...*").await {
                        Ok(msg) => msg,
                        Err(e) => {
                            error!("Error sending thinking message: {:?}", e);
                            return Ok(());
                        }
                    };
                    
                    // Call the Gemini API with user's display name
                    let user_name = &msg.author.name;
                    match self.call_gemini_api_with_user(&content, user_name).await {
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
                    // Fallback if Gemini API is not configured
                    if let Err(e) = msg.channel_id.say(&ctx.http, format!("Hello {}, you mentioned me! I'm {}! (Gemini API is not configured)", msg.author.name, self.bot_name)).await {
                        error!("Error sending mention response: {:?}", e);
                    }
                }
            }
        }
        
        Ok(())
    }
}

impl Bot {
    async fn call_gemini_api(&self, prompt: &str) -> Result<String> {
        // For backward compatibility, call the version with user name
        self.call_gemini_api_with_user(prompt, "User").await
    }
    
    async fn call_gemini_api_with_user(&self, prompt: &str, user_name: &str) -> Result<String> {
        // Check if we have an API key
        let api_key = match &self.gemini_api_key {
            Some(key) => key,
            None => {
                return Err(anyhow::anyhow!("Gemini API key not configured"));
            }
        };
        
        // Determine which endpoint to use
        let endpoint = self.gemini_api_endpoint.as_deref().unwrap_or("https://generativelanguage.googleapis.com/v1beta/models/gemini-1.5-flash:generateContent");
        
        // Format the prompt using the wrapper, including the user's name
        let formatted_prompt = self.gemini_prompt_wrapper
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
        let response = client.post(format!("{}?key={}", endpoint, api_key))
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

#[async_trait]
impl EventHandler for Bot {
    async fn message(&self, ctx: Context, msg: Message) {
        // Ignore messages from bots (including ourselves)
        if msg.author.bot {
            return;
        }
        
        // Only process messages in the followed channel
        if msg.channel_id != self.followed_channel {
            return;
        }
        
        // Process the message
        if let Err(e) = self.process_message(&ctx, &msg).await {
            error!("Error processing message: {:?}", e);
        }
    }

    async fn ready(&self, _: Context, ready: Ready) {
        info!("✅ {} ({}) is connected and following channel {}!", self.bot_name, ready.user.name, self.followed_channel);
        info!("Bot is ready to respond to messages in the channel");
        
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

// Helper function to find a channel by name
async fn find_channel_by_name(http: &Http, name: &str, server_name: Option<&str>) -> Option<ChannelId> {
    // Get all the guilds (servers) the bot is in
    let guilds = http.get_guilds(None, None).await.ok()?;
    
    info!("Searching for channel '{}' across {} servers", name, guilds.len());
    
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
                    return Some(channel.id);
                }
            }
            
            info!("No matching channel found in this server");
        } else {
            info!("Could not retrieve channels for this server");
        }
    }
    
    info!("❌ Channel '{}' not found in any server", name);
    None
}

fn load_config() -> Result<Config> {
    let config_path = Path::new("CrowConfig.toml");
    
    if config_path.exists() {
        let config_content = fs::read_to_string(config_path)
            .context("Failed to read CrowConfig.toml")?;
        
        let config: Config = toml::from_str(&config_content)
            .context("Failed to parse CrowConfig.toml")?;
        
        return Ok(config);
    }
    
    Err(anyhow::anyhow!("Configuration file CrowConfig.toml not found"))
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    // Load configuration
    let config = load_config()?;
    
    // Get the discord token
    let token = &config.discord_token;
    
    // Get the bot name
    let bot_name = config.bot_name.unwrap_or_else(|| "Crow".to_string());
    
    // Get the message history limit
    let message_history_limit = config.message_history_limit
        .and_then(|limit| limit.parse::<usize>().ok())
        .unwrap_or(10000);
    
    info!("Message history limit set to {}", message_history_limit);
    
    // Get database trim interval (default: 1 hour)
    let db_trim_interval = config.db_trim_interval_secs
        .and_then(|interval| interval.parse::<u64>().ok())
        .unwrap_or(3600); // Default: 1 hour
    
    info!("Database trim interval set to {} seconds", db_trim_interval);
    
    // Get Gemini API rate limits
    let gemini_rate_limit_minute = config.gemini_rate_limit_minute
        .and_then(|limit| limit.parse::<u32>().ok())
        .unwrap_or(15); // Default: 15 calls per minute
    
    let gemini_rate_limit_day = config.gemini_rate_limit_day
        .and_then(|limit| limit.parse::<u32>().ok())
        .unwrap_or(1500); // Default: 1500 calls per day
    
    info!("Gemini API rate limits set to {} calls per minute and {} calls per day", 
          gemini_rate_limit_minute, gemini_rate_limit_day);
    
    // Get Gemini API key
    let gemini_api_key = config.gemini_api_key;
    if gemini_api_key.is_none() {
        error!("Gemini API key not found in config");
    } else {
        info!("Gemini API key loaded");
    }
    
    // Get custom prompt wrapper if available
    let gemini_prompt_wrapper = config.gemini_prompt_wrapper;
    
    // Get custom Gemini API endpoint if available
    let gemini_api_endpoint = config.gemini_api_endpoint;
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
    
    // Find the channel ID
    info!("Looking for channel by {}...", 
          if config.followed_channel_name.is_some() { 
              format!("name '{}'", config.followed_channel_name.as_ref().unwrap()) 
          } else { 
              format!("ID '{}'", config.followed_channel_id.as_ref().unwrap_or(&"none".to_string())) 
          });
          
    let channel_id = if let Some(name) = &config.followed_channel_name {
        // Try to find by name
        info!("Searching for channel with name: '{}'", name);
        if let Some(server_name) = &config.followed_server_name {
            info!("Limiting search to server: '{}'", server_name);
        }
        
        match find_channel_by_name(&client.http, name, config.followed_server_name.as_deref()).await {
            Some(id) => {
                info!("✅ Found channel '{}' with ID {}", name, id);
                id
            },
            None => {
                info!("❌ Could not find channel with name '{}'", name);
                // Fall back to ID if provided
                if let Some(id_str) = &config.followed_channel_id {
                    if let Ok(id) = id_str.parse::<u64>() {
                        info!("Using fallback channel ID: {}", id);
                        ChannelId::new(id)
                    } else {
                        error!("❌ Could not find channel '{}' and FOLLOWED_CHANNEL_ID '{}' is not valid", name, id_str);
                        return Err(anyhow::anyhow!("Could not find channel '{}' and FOLLOWED_CHANNEL_ID is not valid", name));
                    }
                } else {
                    error!("❌ Could not find channel '{}' and no FOLLOWED_CHANNEL_ID provided", name);
                    return Err(anyhow::anyhow!("Could not find channel '{}' and no FOLLOWED_CHANNEL_ID provided", name));
                }
            }
        }
    } else if let Some(id_str) = &config.followed_channel_id {
        // Use ID directly
        let id = id_str.parse::<u64>()
            .context("'FOLLOWED_CHANNEL_ID' is not a valid u64")?;
        info!("Using provided channel ID: {}", id);
        ChannelId::new(id)
    } else {
        error!("❌ Neither FOLLOWED_CHANNEL_NAME nor FOLLOWED_CHANNEL_ID was provided");
        return Err(anyhow::anyhow!("Neither FOLLOWED_CHANNEL_NAME nor FOLLOWED_CHANNEL_ID was provided"));
    };
    
    // Create a new bot instance with the valid channel ID
    let mut bot = Bot::new(
        channel_id,
        config.db_host.clone(),
        config.db_name.clone(),
        config.db_user.clone(),
        config.db_password.clone(),
        config.google_api_key,
        config.google_search_engine_id,
        gemini_api_key,
        gemini_api_endpoint,
        bot_name.clone(),
        message_db.clone(),
        message_history_limit
    );
    
    // Check database connection
    if let Err(e) = bot.check_database_connection().await {
        error!("Error checking database connection: {:?}", e);
    }
    
    // Set custom prompt wrapper if available
    if let Some(prompt_wrapper) = gemini_prompt_wrapper {
        bot.gemini_prompt_wrapper = prompt_wrapper;
        info!("Using custom Gemini prompt wrapper");
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
    info!("Following channel ID: {}", channel_id);
    info!("Press Ctrl+C to stop the bot");
    client.start().await?;

    Ok(())
}

// Remove the placeholder database functions since we're now using db_utils
