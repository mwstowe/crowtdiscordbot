use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_rusqlite::Connection as SqliteConnection;
use serenity::model::channel::Message;
use serenity::model::id::{MessageId, ChannelId, GuildId, UserId};
use tracing::{info, error};
use std::collections::HashSet;
// Removed unused imports

// Initialize the SQLite database with enhanced schema
pub async fn initialize_database(path: &str) -> Result<Arc<Mutex<SqliteConnection>>, Box<dyn std::error::Error>> {
    // Connect to the database
    let conn = SqliteConnection::open(path).await?;
    
    // First check if the table exists at all
    let table_exists = conn.call(|conn| {
        let result: i64 = conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='messages'",
            [],
            |row| row.get(0)
        )?;
        Ok::<_, rusqlite::Error>(result > 0)
    }).await?;
    
    if !table_exists {
        // Table doesn't exist, create it with the full schema
        conn.call(|conn| {
            conn.execute(
                "CREATE TABLE messages (
                    id INTEGER PRIMARY KEY,
                    message_id TEXT NOT NULL,
                    channel_id TEXT NOT NULL,
                    guild_id TEXT,
                    author_id TEXT NOT NULL,
                    author TEXT NOT NULL,
                    display_name TEXT,
                    content TEXT NOT NULL,
                    timestamp INTEGER NOT NULL,
                    referenced_message_id TEXT
                )",
                [],
            )?;
            Ok::<_, rusqlite::Error>(())
        }).await?;
    } else {
        // Table exists, check if migration is needed
        let needs_migration = conn.call(|conn| {
            let mut stmt = conn.prepare("PRAGMA table_info(messages)")?;
            let columns = stmt.query_map([], |row| {
                let name: String = row.get(1)?;
                Ok(name)
            })?;
            
            let mut has_message_id = false;
            let mut has_channel_id = false;
            let mut has_author_id = false;
            
            for column_result in columns {
                if let Ok(column_name) = column_result {
                    if column_name == "message_id" {
                        has_message_id = true;
                    } else if column_name == "channel_id" {
                        has_channel_id = true;
                    } else if column_name == "author_id" {
                        has_author_id = true;
                    }
                }
            }
            
            Ok::<_, rusqlite::Error>(!has_message_id || !has_channel_id || !has_author_id)
        }).await?;
        
        if needs_migration {
            info!("Migrating messages database to enhanced schema...");
            
            // Create a backup of the old table
            conn.call(|conn| {
                conn.execute("ALTER TABLE messages RENAME TO messages_backup", [])?;
                Ok::<_, rusqlite::Error>(())
            }).await?;
            
            // Create the new table with the enhanced schema
            conn.call(|conn| {
                conn.execute(
                    "CREATE TABLE messages (
                        id INTEGER PRIMARY KEY,
                        message_id TEXT NOT NULL,
                        channel_id TEXT NOT NULL,
                        guild_id TEXT,
                        author_id TEXT NOT NULL,
                        author TEXT NOT NULL,
                        display_name TEXT,
                        content TEXT NOT NULL,
                        timestamp INTEGER NOT NULL,
                        referenced_message_id TEXT
                    )",
                    [],
                )?;
                Ok::<_, rusqlite::Error>(())
            }).await?;
            
            // Migrate data from backup with default values for new columns
            conn.call(|conn| {
                conn.execute(
                    "INSERT INTO messages (id, author, display_name, content, timestamp, message_id, channel_id, author_id)
                     SELECT id, author, display_name, content, timestamp, '0', '0', '0' FROM messages_backup",
                    [],
                )?;
                Ok::<_, rusqlite::Error>(())
            }).await?;
        }
    }

    // Create indexes for faster queries
    let indexes = vec![
        ("idx_message_timestamp", "messages (timestamp)"),
        ("idx_message_author_id", "messages (author, id)")
    ];

    for (name, sql) in indexes {
        let sql = format!("CREATE INDEX IF NOT EXISTS {} ON {}", name, sql);
        conn.call(move |conn| {
            conn.execute(&sql, []).map(|_| ())?;
            Ok::<_, rusqlite::Error>(())
        }).await?;
    }
    
    // Return the connection wrapped in an Arc<Mutex>
    Ok(Arc::new(Mutex::new(conn)))
}

// Save a message to the SQLite database with enhanced fields
pub async fn save_message(
    conn: Arc<Mutex<SqliteConnection>>,
    author: &str,
    display_name: &str,
    content: &str,
    message: Option<&Message>, // Optional Message object for enhanced fields
    operation_id: Option<String> // Optional operation ID for tracking
) -> Result<(), Box<dyn std::error::Error>> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    
    let author = author.to_string();
    // Use the display_name::clean_display_name function for consistency
    let clean_display_name = crate::display_name::clean_display_name(display_name);
    let content = content.to_string();
    let op_id = operation_id.unwrap_or_else(|| "unknown".to_string());
    
    let conn_guard = conn.lock().await;
    
    // If we have a Message object, save all fields
    if let Some(msg) = message {
        // Clone the values we need from the Message
        let message_id = msg.id.to_string();
        let channel_id = msg.channel_id.to_string();
        let guild_id = msg.guild_id.map(|id| id.to_string()).unwrap_or_default();
        let author_id = msg.author.id.to_string();
        let referenced_message_id = msg.referenced_message.as_ref().map(|m| m.id.to_string()).unwrap_or_default();
        
        info!("DB_CHECK: Checking if message ID {} exists (operation: {})", message_id, op_id);
        
        // Check if this message already exists in the database
        let exists = conn_guard.call({
            let message_id = message_id.clone();
            move |conn| {
                let result: Result<i64, _> = conn.query_row(
                    "SELECT 1 FROM messages WHERE message_id = ?",
                    [&message_id],
                    |_| Ok(1)
                );
                Ok::<_, rusqlite::Error>(result.is_ok())
            }
        }).await?;
        
        if exists {
            // Message already exists, update it instead of inserting a new record
            info!("DB_UPDATE: Message {} already exists, updating content (operation: {})", message_id, op_id);
            conn_guard.call(move |conn| {
                conn.execute(
                    "UPDATE messages SET content = ? WHERE message_id = ?",
                    [&content, &message_id],
                )?;
                Ok::<_, rusqlite::Error>(())
            }).await?;
        } else {
            // Message doesn't exist, insert it
            info!("DB_INSERT: Inserting new message ID {} (operation: {})", message_id, op_id);
            conn_guard.call(move |conn| {
                conn.execute(
                    "INSERT INTO messages (
                        message_id, channel_id, guild_id, author_id, author, display_name, content, timestamp, referenced_message_id
                    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    [
                        &message_id,
                        &channel_id,
                        &guild_id,
                        &author_id,
                        &author,
                        &clean_display_name,
                        &content,
                        &timestamp.to_string(),
                        &referenced_message_id,
                    ],
                )?;
                Ok::<_, rusqlite::Error>(())
            }).await?;
        }
    } else {
        // Fallback to basic fields if no Message object is provided
        info!("DB_INSERT: Inserting message without ID (operation: {})", op_id);
        conn_guard.call(move |conn| {
            conn.execute(
                "INSERT INTO messages (
                    message_id, channel_id, author_id, author, display_name, content, timestamp
                ) VALUES (?, ?, ?, ?, ?, ?, ?)",
                [
                    "0", // Default message_id
                    "0", // Default channel_id
                    "0", // Default author_id
                    &author,
                    &clean_display_name,
                    &content,
                    &timestamp.to_string(),
                ],
            )?;
            Ok::<_, rusqlite::Error>(())
        }).await?;
    }
    
    Ok(())
}

// Trim the database to keep only the most recent messages
pub async fn trim_database(
    conn: Arc<Mutex<SqliteConnection>>,
    limit: usize
) -> Result<usize, Box<dyn std::error::Error>> {
    let conn_guard = conn.lock().await;
    
    // First, count how many messages we have
    let count = conn_guard.call(move |conn| {
        let mut stmt = conn.prepare("SELECT COUNT(*) FROM messages")?;
        let count: i64 = stmt.query_row([], |row| row.get(0))?;
        Ok::<_, rusqlite::Error>(count)
    }).await?;
    
    // If we have more messages than the limit, delete the oldest ones
    if count as usize > limit {
        let to_delete = count as usize - limit;
        
        conn_guard.call(move |conn| {
            conn.execute(
                "DELETE FROM messages WHERE id IN (
                    SELECT id FROM messages ORDER BY timestamp ASC LIMIT ?
                )",
                [to_delete],
            )?;
            Ok::<_, rusqlite::Error>(())
        }).await?;
        
        return Ok(to_delete);
    }
    
    Ok(0)
}

// Get recent messages from the database in chronological order
pub async fn get_recent_messages(
    conn: Arc<Mutex<SqliteConnection>>,
    limit: usize,
    channel_id: Option<&str>
) -> Result<Vec<(String, String, String)>, Box<dyn std::error::Error>> {
    let conn_guard = conn.lock().await;
    
    // Add debug logging
    info!("Getting recent messages. Limit: {}, Channel ID: {:?}", limit, channel_id);
    
    // Debug: Check which database file we're using
    let db_path = conn_guard.call(move |conn| {
        let path = conn.query_row("PRAGMA database_list", [], |row| {
            row.get::<_, String>(2)
        }).unwrap_or_else(|_| "Unknown".to_string());
        Ok::<_, rusqlite::Error>(path)
    }).await?;
    
    info!("Using database file: {}", db_path);
    
    // Debug: List all tables in the database
    let tables = conn_guard.call(move |conn| {
        let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table'")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut result = Vec::new();
        for row_result in rows {
            if let Ok(name) = row_result {
                result.push(name);
            }
        }
        Ok::<_, rusqlite::Error>(result)
    }).await?;
    
    info!("Tables in database: {:?}", tables);
    
    // Debug: List all distinct channel IDs in the database
    let channels = conn_guard.call(move |conn| {
        let mut stmt = conn.prepare("SELECT DISTINCT channel_id FROM messages")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut result = Vec::new();
        for row_result in rows {
            if let Ok(id) = row_result {
                result.push(id);
            }
        }
        Ok::<_, rusqlite::Error>(result)
    }).await?;
    
    info!("Channel IDs in database: {:?}", channels);
    
    // If channel_id is provided, filter by it
    let raw_messages: Vec<(String, String, String, String)> = if let Some(channel) = channel_id {
        let channel_str = channel.to_string();
        
        // First get the total count of messages for this channel
        let count = conn_guard.call({
            let channel_str = channel_str.clone();
            move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT COUNT(*) FROM messages WHERE channel_id = ?"
                )?;
                let count: i64 = stmt.query_row([&channel_str], |row| row.get(0))?;
                Ok::<_, rusqlite::Error>(count)
            }
        }).await?;
        
        info!("Found {} messages in channel {}", count, channel_str);
        
        // Debug: Check if there are any messages with non-zero message_id
        let valid_count = conn_guard.call({
            let channel_str = channel_str.clone();
            move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT COUNT(*) FROM messages WHERE channel_id = ? AND message_id != '0'"
                )?;
                let count: i64 = stmt.query_row([&channel_str], |row| row.get(0))?;
                Ok::<_, rusqlite::Error>(count)
            }
        }).await?;
        
        info!("Found {} messages with valid message_id in channel {}", valid_count, channel_str);
        
        // Calculate the offset to get only the most recent messages
        let offset = if count > limit as i64 { count - limit as i64 } else { 0 };
        
        info!("Using offset {} to get the most recent {} messages", offset, limit);
        
        // Get the most recent messages in chronological order
        // TEMPORARILY REMOVED message_id != '0' filter for debugging
        let result = conn_guard.call({
            let channel_str = channel_str.clone();
            move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT message_id, author, display_name, content FROM messages 
                     WHERE channel_id = ? 
                     ORDER BY timestamp ASC
                     LIMIT ? OFFSET ?"
                )?;
                
                let rows = stmt.query_map([&channel_str, &limit.to_string(), &offset.to_string()], |row| {
                    let msg_id = row.get::<_, String>(0)?;
                    let author = row.get::<_, String>(1)?;
                    let display_name = row.get::<_, String>(2).unwrap_or_else(|_| "".to_string());
                    let content = row.get::<_, String>(3)?;
                    
                    info!("Retrieved message: ID={}, Author={}, Content={}", msg_id, author, content);
                    
                    Ok((
                        msg_id,
                        author,
                        display_name,
                        content,
                    ))
                })?;
                
                let mut result = Vec::new();
                for row_result in rows {
                    if let Ok(row) = row_result {
                        result.push(row);
                    }
                }
                
                Ok::<_, rusqlite::Error>(result)
            }
        }).await?;
        
        info!("Retrieved {} messages for context", result.len());
        result
    } else {
        // If no channel_id is provided, get messages from all channels
        
        // First get the total count of messages
        let count = conn_guard.call(move |conn| {
            let mut stmt = conn.prepare("SELECT COUNT(*) FROM messages")?;
            let count: i64 = stmt.query_row([], |row| row.get(0))?;
            Ok::<_, rusqlite::Error>(count)
        }).await?;
        
        info!("Found {} total messages across all channels", count);
        
        // Debug: Check if there are any messages with non-zero message_id
        let valid_count = conn_guard.call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT COUNT(*) FROM messages WHERE message_id != '0'"
            )?;
            let count: i64 = stmt.query_row([], |row| row.get(0))?;
            Ok::<_, rusqlite::Error>(count)
        }).await?;
        
        info!("Found {} messages with valid message_id across all channels", valid_count);
        
        // Calculate the offset to get only the most recent messages
        let offset = if count > limit as i64 { count - limit as i64 } else { 0 };
        
        info!("Using offset {} to get the most recent {} messages", offset, limit);
        
        // Get the most recent messages in chronological order
        // TEMPORARILY REMOVED message_id != '0' filter for debugging
        let result = conn_guard.call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT message_id, author, display_name, content FROM messages 
                 ORDER BY timestamp ASC
                 LIMIT ? OFFSET ?"
            )?;
            
            let rows = stmt.query_map([&limit.to_string(), &offset.to_string()], |row| {
                let msg_id = row.get::<_, String>(0)?;
                let author = row.get::<_, String>(1)?;
                let display_name = row.get::<_, String>(2).unwrap_or_else(|_| "".to_string());
                let content = row.get::<_, String>(3)?;
                
                info!("Retrieved message: ID={}, Author={}, Content={}", msg_id, author, content);
                
                Ok((
                    msg_id,
                    author,
                    display_name,
                    content,
                ))
            })?;
            
            let mut result = Vec::new();
            for row_result in rows {
                if let Ok(row) = row_result {
                    result.push(row);
                }
            }
            
            Ok::<_, rusqlite::Error>(result)
        }).await?;
        
        info!("Retrieved {} messages for context", result.len());
        result
    };
    
    // Deduplicate messages based on content
    info!("Before deduplication: {} messages", raw_messages.len());
    
    // Use a HashSet to track seen content
    let mut seen_content = HashSet::new();
    let mut deduplicated_messages = Vec::new();
    
    for (msg_id, author, display_name, content) in raw_messages {
        if seen_content.insert(content.clone()) {
            // This is a new message content, add it to the result
            deduplicated_messages.push((author, display_name, content));
        }
    }
    
    info!("After deduplication: {} messages", deduplicated_messages.len());
    
    Ok(deduplicated_messages)
}

// Trim the message history to keep only the most recent messages
pub async fn trim_message_history(
    conn: Arc<tokio::sync::Mutex<SqliteConnection>>,
    limit: usize
) -> Result<usize, Box<dyn std::error::Error>> {
    // This is just an alias for trim_database for backward compatibility
    trim_database(conn, limit).await
}

// Load message history from the database
pub async fn load_message_history(
    conn: Arc<tokio::sync::Mutex<SqliteConnection>>,
    history: &mut std::collections::VecDeque<serenity::model::channel::Message>,
    limit: usize,
    channel_id: Option<&str>
) -> Result<(), Box<dyn std::error::Error>> {
    let conn_guard = conn.lock().await;
    
    // Get messages from the database, filtered by channel_id if provided
    let db_messages = if let Some(channel) = channel_id {
        let channel = channel.to_string();
        conn_guard.call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT message_id, channel_id, guild_id, author_id, author, content, timestamp, referenced_message_id 
                 FROM messages 
                 WHERE channel_id = ?
                 ORDER BY timestamp DESC LIMIT ?"
            )?;
            
            let rows = stmt.query_map([&channel, &limit.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?, // message_id
                    row.get::<_, String>(1)?, // channel_id
                    row.get::<_, Option<String>>(2)?, // guild_id
                    row.get::<_, String>(3)?, // author_id
                    row.get::<_, String>(4)?, // author
                    row.get::<_, String>(5)?, // content
                    row.get::<_, i64>(6)?, // timestamp
                    row.get::<_, Option<String>>(7)?, // referenced_message_id
                ))
            })?;
            
            let mut result = Vec::new();
            for row_result in rows {
                if let Ok(row) = row_result {
                    result.push(row);
                }
            }
            
            Ok::<_, rusqlite::Error>(result)
        }).await?
    } else {
        // If no channel_id is provided, get messages from all channels
        conn_guard.call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT message_id, channel_id, guild_id, author_id, author, content, timestamp, referenced_message_id 
                 FROM messages ORDER BY timestamp DESC LIMIT ?"
            )?;
            
            let rows = stmt.query_map([limit], |row| {
                Ok((
                    row.get::<_, String>(0)?, // message_id
                    row.get::<_, String>(1)?, // channel_id
                    row.get::<_, Option<String>>(2)?, // guild_id
                    row.get::<_, String>(3)?, // author_id
                    row.get::<_, String>(4)?, // author
                    row.get::<_, String>(5)?, // content
                    row.get::<_, i64>(6)?, // timestamp
                    row.get::<_, Option<String>>(7)?, // referenced_message_id
                ))
            })?;
            
            let mut result = Vec::new();
            for row_result in rows {
                if let Ok(row) = row_result {
                    result.push(row);
                }
            }
            
            Ok::<_, rusqlite::Error>(result)
        }).await?
    };
    
    // Try to convert database records to Message objects
    for (msg_id_str, channel_id_str, guild_id_opt, author_id_str, author_name, content, _timestamp, _ref_msg_id) in db_messages {
        // Parse IDs - use default values if parsing fails
        let msg_id = msg_id_str.parse::<u64>().unwrap_or(0);
        let channel_id = channel_id_str.parse::<u64>().unwrap_or(0);
        let author_id = author_id_str.parse::<u64>().unwrap_or(0);
        
        // Skip records with invalid IDs (likely from old schema)
        if msg_id == 0 || channel_id == 0 || author_id == 0 {
            continue;
        }
        
        // Create a minimal Message object with the available data
        let mut msg = Message::default();
        msg.id = MessageId::new(msg_id);
        msg.channel_id = ChannelId::new(channel_id);
        if let Some(guild_id_str) = guild_id_opt {
            if let Ok(guild_id) = guild_id_str.parse::<u64>() {
                msg.guild_id = Some(GuildId::new(guild_id));
            }
        }
        msg.author.id = UserId::new(author_id);
        msg.author.name = author_name;
        msg.content = content;
        
        // Add to history
        history.push_back(msg);
    }
    
    Ok(())
}

// Update an existing message in the database when it's edited
#[allow(dead_code)]
pub async fn update_message(
    conn: Arc<Mutex<SqliteConnection>>,
    message_id: String,
    new_content: String
) -> Result<(), Box<dyn std::error::Error>> {
    let conn_guard = conn.lock().await;
    
    conn_guard.call(move |conn| {
        // Update only the content field, keeping all other fields the same
        conn.execute(
            "UPDATE messages SET content = ? WHERE message_id = ?",
            [&new_content, &message_id],
        )?;
        
        Ok(())
    }).await?;
    
    Ok(())
}

// Add a function to clean up duplicate messages and add a unique index
pub async fn clean_up_duplicates(conn: Arc<Mutex<SqliteConnection>>) -> Result<usize, Box<dyn std::error::Error>> {
    let conn_guard = conn.lock().await;
    
    info!("Starting database cleanup to remove duplicate messages...");
    
    // First, identify duplicate message_ids
    let duplicate_count = conn_guard.call(move |conn| {
        // Count how many duplicates we have
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) - COUNT(DISTINCT message_id) FROM messages WHERE message_id != '0'",
            [],
            |row| row.get(0)
        )?;
        
        info!("Found {} duplicate message IDs", count);
        
        if count > 0 {
            // Create a temporary table with unique messages
            conn.execute("CREATE TEMPORARY TABLE temp_messages AS 
                          SELECT MIN(id) as min_id, message_id
                          FROM messages 
                          WHERE message_id != '0'
                          GROUP BY message_id", [])?;
            
            // Delete all duplicates (keeping only the first occurrence of each message_id)
            let deleted = conn.execute(
                "DELETE FROM messages 
                 WHERE message_id != '0' AND id NOT IN (SELECT min_id FROM temp_messages)",
                []
            )?;
            
            // Drop the temporary table
            conn.execute("DROP TABLE temp_messages", [])?;
            
            info!("Deleted {} duplicate messages", deleted);
            
            // Now try to create a unique index
            match conn.execute(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_unique_message_id ON messages(message_id) 
                 WHERE message_id != '0'",
                []
            ) {
                Ok(_) => info!("Successfully created unique index on message_id"),
                Err(e) => {
                    error!("Failed to create unique index: {}", e);
                    // If we still have duplicates, we need to handle them differently
                    if let rusqlite::Error::SqliteFailure(_, Some(msg)) = &e {
                        if msg.contains("UNIQUE constraint failed") {
                            info!("Still have duplicates, using more aggressive cleanup...");
                            
                            // More aggressive approach: keep only the latest message for each message_id
                            conn.execute(
                                "DELETE FROM messages WHERE id NOT IN (
                                    SELECT MAX(id) FROM messages 
                                    WHERE message_id != '0'
                                    GROUP BY message_id
                                )",
                                []
                            )?;
                            
                            // Try creating the index again
                            conn.execute(
                                "CREATE UNIQUE INDEX IF NOT EXISTS idx_unique_message_id ON messages(message_id) 
                                 WHERE message_id != '0'",
                                []
                            )?;
                            
                            info!("Successfully created unique index after aggressive cleanup");
                        }
                    }
                }
            }
            
            Ok::<_, rusqlite::Error>(deleted as usize)
        } else {
            // No duplicates, just create the index
            conn.execute(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_unique_message_id ON messages(message_id) 
                 WHERE message_id != '0'",
                []
            )?;
            
            info!("No duplicates found. Successfully created unique index on message_id");
            Ok(0)
        }
    }).await?;
    
    Ok(duplicate_count)
}
