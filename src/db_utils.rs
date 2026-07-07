use serenity::model::channel::Message;
use serenity::model::id::{ChannelId, GuildId, MessageId, UserId};
use std::collections::HashSet;
use std::sync::Arc;
use tokio_rusqlite::Connection as SqliteConnection;
use tracing::{error, info};
// Removed unused imports

// Initialize the SQLite database with enhanced schema
pub async fn initialize_database(
    path: &str,
) -> Result<Arc<SqliteConnection>, Box<dyn std::error::Error>> {
    // Connect to the database
    let conn = SqliteConnection::open(path).await?;

    // Enable WAL mode and set busy timeout for better concurrent access
    conn.call(|conn| {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;",
        )?;
        Ok::<_, rusqlite::Error>(())
    })
    .await?;

    // First check if the table exists at all
    let table_exists = conn
        .call(|conn| {
            let result: i64 = conn.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='messages'",
                [],
                |row| row.get(0),
            )?;
            Ok::<_, rusqlite::Error>(result > 0)
        })
        .await?;

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
        })
        .await?;
    } else {
        // Table exists, check if migration is needed
        let needs_migration = conn
            .call(|conn| {
                let mut stmt = conn.prepare("PRAGMA table_info(messages)")?;
                let columns = stmt.query_map([], |row| {
                    let name: String = row.get(1)?;
                    Ok(name)
                })?;

                let mut has_message_id = false;
                let mut has_channel_id = false;
                let mut has_author_id = false;

                for column_name in columns.flatten() {
                    if column_name == "message_id" {
                        has_message_id = true;
                    } else if column_name == "channel_id" {
                        has_channel_id = true;
                    } else if column_name == "author_id" {
                        has_author_id = true;
                    }
                }

                Ok::<_, rusqlite::Error>(!has_message_id || !has_channel_id || !has_author_id)
            })
            .await?;

        if needs_migration {
            info!("Migrating messages database to enhanced schema...");

            conn.call(|conn| {
                let tx = conn.transaction()?;
                tx.execute("ALTER TABLE messages RENAME TO messages_backup", [])?;
                tx.execute(
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
                tx.execute(
                    "INSERT INTO messages (id, author, display_name, content, timestamp, message_id, channel_id, author_id)
                     SELECT id, author, display_name, content, timestamp, '0', '0', '0' FROM messages_backup",
                    [],
                )?;
                tx.commit()?;
                Ok::<_, rusqlite::Error>(())
            }).await?;
        }
    }

    // Create indexes for faster queries
    let indexes = vec![
        ("idx_message_timestamp", "messages (timestamp)"),
        ("idx_message_author_id", "messages (author, id)"),
    ];

    for (name, sql) in indexes {
        let sql = format!("CREATE INDEX IF NOT EXISTS {name} ON {sql}");
        conn.call(move |conn| {
            conn.execute(&sql, []).map(|_| ())?;
            Ok::<_, rusqlite::Error>(())
        })
        .await?;
    }

    // Return the connection wrapped in an Arc
    Ok(Arc::new(conn))
}

// Save a message to the SQLite database with enhanced fields
pub async fn save_message(
    conn: Arc<SqliteConnection>,
    author: &str,
    display_name: &str,
    content: &str,
    message: Option<&Message>, // Optional Message object for enhanced fields
    _operation_id: Option<String>, // Optional operation ID for tracking (no longer used)
) -> Result<(), Box<dyn std::error::Error>> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();

    let author = author.to_string();
    // Use the display_name::clean_display_name function for consistency
    let clean_display_name = crate::display_name::clean_display_name(display_name);
    let content = content.to_string();

    // If we have a Message object, save all fields
    if let Some(msg) = message {
        // Clone the values we need from the Message
        let message_id = msg.id.to_string();
        let channel_id = msg.channel_id.to_string();
        let guild_id = msg.guild_id.map(|id| id.to_string()).unwrap_or_default();
        let author_id = msg.author.id.to_string();
        let referenced_message_id = msg
            .referenced_message
            .as_ref()
            .map(|m| m.id.to_string())
            .unwrap_or_default();

        // Check if this message already exists in the database
        let exists = conn
            .call({
                let message_id = message_id.clone();
                move |conn| {
                    let result: Result<i64, _> = conn.query_row(
                        "SELECT 1 FROM messages WHERE message_id = ?",
                        [&message_id],
                        |_| Ok(1),
                    );
                    Ok::<_, rusqlite::Error>(result.is_ok())
                }
            })
            .await?;

        if exists {
            // Message already exists, update it instead of inserting a new record
            conn.call(move |conn| {
                conn.execute(
                    "UPDATE messages SET content = ? WHERE message_id = ?",
                    [&content, &message_id],
                )?;
                Ok::<_, rusqlite::Error>(())
            })
            .await?;
        } else {
            // Message doesn't exist, insert it
            conn.call(move |conn| {
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
        conn.call(move |conn| {
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
        })
        .await?;
    }

    Ok(())
}

// Trim the database to keep only the most recent messages
pub async fn trim_database(
    conn: Arc<SqliteConnection>,
    limit: usize,
) -> Result<usize, Box<dyn std::error::Error>> {
    // First, count how many messages we have
    let count = conn
        .call(move |conn| {
            let mut stmt = conn.prepare("SELECT COUNT(*) FROM messages")?;
            let count: i64 = stmt.query_row([], |row| row.get(0))?;
            Ok::<_, rusqlite::Error>(count)
        })
        .await?;

    // If we have more messages than the limit, delete the oldest ones
    if count as usize > limit {
        let to_delete = count as usize - limit;

        conn.call(move |conn| {
            conn.execute(
                "DELETE FROM messages WHERE id IN (
                    SELECT id FROM messages ORDER BY timestamp ASC LIMIT ?
                )",
                [to_delete],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await?;

        return Ok(to_delete);
    }

    Ok(0)
}

// Get recent messages from the database in chronological order
// Get recent messages from the database with reply context
pub async fn get_recent_messages_with_reply_context(
    conn: Arc<SqliteConnection>,
    limit: usize,
    channel_id: Option<&str>,
) -> Result<Vec<(String, String, Option<String>, String, Option<String>)>, Box<dyn std::error::Error>>
{
    // If channel_id is provided, filter by it
    let raw_messages: Vec<(String, String, String, String, Option<String>)> = if let Some(channel) =
        channel_id
    {
        let channel_str = channel.to_string();

        // Get the most recent messages with their referenced message content
        let result = conn
            .call({
                let channel_str = channel_str.clone();
                move |conn| {
                    let mut stmt = conn.prepare(
                        "SELECT m.message_id, m.channel_id, m.guild_id, m.author_id, m.author,
                                m.display_name, m.content, m.timestamp, m.referenced_message_id,
                                ref.author as ref_author, ref.display_name as ref_display_name, ref.content as ref_content
                         FROM messages m
                         LEFT JOIN messages ref ON m.referenced_message_id = ref.message_id
                         WHERE m.channel_id = ?
                         ORDER BY m.timestamp DESC LIMIT ?"
                    )?;

                    let rows = stmt.query_map([&channel_str, &limit.to_string()], |row| {
                        let _ref_author: Option<String> = row.get(9)?;
                        let ref_display_name: Option<String> = row.get(10)?;
                        let ref_content: Option<String> = row.get(11)?;

                        let reply_context = if let (Some(ref_display), Some(ref_cont)) = (ref_display_name, ref_content) {
                            Some(format!("{}: {}", ref_display, ref_cont))
                        } else {
                            None
                        };

                        Ok((
                            row.get::<_, String>(4)?, // author
                            row.get::<_, String>(5)?, // display_name
                            row.get::<_, String>(6)?, // content
                            row.get::<_, i64>(7)?.to_string(), // timestamp
                            reply_context, // reply context
                        ))
                    })?;

                    let result: Vec<_> = rows.collect::<Result<Vec<_>, _>>()?;
                    Ok::<_, rusqlite::Error>(result)
                }
            })
            .await?;

        result
    } else {
        // If no channel_id is provided, get messages from all channels
        conn.call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT m.message_id, m.channel_id, m.guild_id, m.author_id, m.author,
                        m.display_name, m.content, m.timestamp, m.referenced_message_id,
                        ref.author as ref_author, ref.display_name as ref_display_name, ref.content as ref_content
                 FROM messages m
                 LEFT JOIN messages ref ON m.referenced_message_id = ref.message_id
                 ORDER BY m.timestamp DESC LIMIT ?"
            )?;

            let rows = stmt.query_map([&limit.to_string()], |row| {
                let _ref_author: Option<String> = row.get(9)?;
                let ref_display_name: Option<String> = row.get(10)?;
                let ref_content: Option<String> = row.get(11)?;

                let reply_context = if let (Some(ref_display), Some(ref_cont)) = (ref_display_name, ref_content) {
                    Some(format!("{}: {}", ref_display, ref_cont))
                } else {
                    None
                };

                Ok((
                    row.get::<_, String>(4)?, // author
                    row.get::<_, String>(5)?, // display_name
                    row.get::<_, String>(6)?, // content
                    row.get::<_, i64>(7)?.to_string(), // timestamp
                    reply_context, // reply context
                ))
            })?;

            let result: Vec<_> = rows.collect::<Result<Vec<_>, _>>()?;
            Ok::<_, rusqlite::Error>(result)
        }).await?
    };

    // Convert to the expected format: (author, display_name, pronouns, content, reply_context)
    #[allow(clippy::type_complexity)]
    let messages: Vec<(String, String, Option<String>, String, Option<String>)> = raw_messages
        .into_iter()
        .map(
            |(author, display_name, content, _timestamp, reply_context)| {
                // Extract pronouns from display name if present
                let pronouns = crate::utils::extract_pronouns(&display_name);
                let clean_display_name = crate::display_name::clean_display_name(&display_name);

                (author, clean_display_name, pronouns, content, reply_context)
            },
        )
        .collect();

    Ok(messages)
}

// Get recent messages from the database in chronological order with pronouns
#[allow(dead_code)]
pub async fn get_recent_messages_with_pronouns(
    conn: Arc<SqliteConnection>,
    limit: usize,
    channel_id: Option<&str>,
) -> Result<Vec<(String, String, Option<String>, String)>, Box<dyn std::error::Error>> {
    let raw_messages: Vec<(String, String, String, String)> = if let Some(channel) = channel_id {
        let channel_str = channel.to_string();

        conn.call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT message_id, author, display_name, content FROM messages
                     WHERE channel_id = ?
                     ORDER BY timestamp DESC
                     LIMIT ?",
            )?;

            let rows = stmt.query_map([&channel_str, &limit.to_string()], |row| {
                let msg_id = row.get::<_, String>(0)?;
                let author = row.get::<_, String>(1)?;
                let display_name = row.get::<_, String>(2).unwrap_or_default();
                let content = row.get::<_, String>(3)?;
                Ok((msg_id, author, display_name, content))
            })?;

            let result: Vec<_> = rows.flatten().collect();
            Ok::<_, rusqlite::Error>(result)
        })
        .await?
    } else {
        conn.call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT message_id, author, display_name, content FROM messages
                     ORDER BY timestamp DESC
                     LIMIT ?",
            )?;

            let rows = stmt.query_map([&limit.to_string()], |row| {
                let msg_id = row.get::<_, String>(0)?;
                let author = row.get::<_, String>(1)?;
                let display_name = row.get::<_, String>(2).unwrap_or_default();
                let content = row.get::<_, String>(3)?;
                Ok((msg_id, author, display_name, content))
            })?;

            let result: Vec<_> = rows.flatten().collect();
            Ok::<_, rusqlite::Error>(result)
        })
        .await?
    };

    info!("Retrieved {} messages for context", raw_messages.len());

    // Deduplicate messages based on content
    let mut seen_content = HashSet::new();
    let mut deduplicated_messages = Vec::new();

    for (_msg_id, author, display_name, content) in raw_messages {
        if seen_content.insert(content.clone()) {
            let pronouns = crate::utils::extract_pronouns(&display_name);
            deduplicated_messages.push((author, display_name, pronouns, content));
        }
    }

    Ok(deduplicated_messages)
}

// Trim the message history to keep only the most recent messages
pub async fn trim_message_history(
    conn: Arc<SqliteConnection>,
    limit: usize,
) -> Result<usize, Box<dyn std::error::Error>> {
    // This is just an alias for trim_database for backward compatibility
    trim_database(conn, limit).await
}

// Load message history from the database
pub async fn load_message_history(
    conn: Arc<SqliteConnection>,
    history: &mut std::collections::VecDeque<serenity::model::channel::Message>,
    limit: usize,
    channel_id: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Get messages from the database, filtered by channel_id if provided
    let db_messages = if let Some(channel) = channel_id {
        let channel = channel.to_string();
        conn.call(move |conn| {
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

            let result: Vec<_> = rows.flatten().collect();

            Ok::<_, rusqlite::Error>(result)
        }).await?
    } else {
        // If no channel_id is provided, get messages from all channels
        conn.call(move |conn| {
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

            let result: Vec<_> = rows.flatten().collect();

            Ok::<_, rusqlite::Error>(result)
        }).await?
    };

    // Try to convert database records to Message objects
    for (
        msg_id_str,
        channel_id_str,
        guild_id_opt,
        author_id_str,
        author_name,
        content,
        _timestamp,
        _ref_msg_id,
    ) in db_messages
    {
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
    conn: Arc<SqliteConnection>,
    message_id: String,
    new_content: String,
) -> Result<(), Box<dyn std::error::Error>> {
    conn.call(move |conn| {
        // Update only the content field, keeping all other fields the same
        conn.execute(
            "UPDATE messages SET content = ? WHERE message_id = ?",
            [&new_content, &message_id],
        )?;

        Ok::<_, rusqlite::Error>(())
    })
    .await?;

    Ok(())
}

// Add a function to clean up duplicate messages and add a unique index
pub async fn clean_up_duplicates(
    conn: Arc<SqliteConnection>,
) -> Result<usize, Box<dyn std::error::Error>> {
    info!("Starting database cleanup to remove duplicate messages...");

    // First, identify duplicate message_ids
    let duplicate_count = conn.call(move |conn| {
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

            Ok::<_, rusqlite::Error>(deleted)
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
// Get the last message for each channel from the messages table
pub async fn get_last_messages_by_channel(
    conn: Arc<SqliteConnection>,
) -> Result<
    std::collections::HashMap<ChannelId, (serenity::model::Timestamp, MessageId)>,
    Box<dyn std::error::Error + Send + Sync>,
> {
    let result = conn
        .call(|conn| {
            // This query gets the most recent message for each channel
            let query = "
            SELECT channel_id, MAX(timestamp) as max_timestamp, message_id
            FROM messages
            GROUP BY channel_id
        ";

            let mut stmt = conn.prepare(query)?;

            let rows = stmt.query_map([], |row| {
                let channel_id_str: String = row.get(0)?;
                let timestamp: i64 = row.get(1)?;
                let message_id_str: String = row.get(2)?;

                // Parse the channel_id and message_id
                let channel_id = channel_id_str.parse::<u64>().unwrap_or_default();
                let message_id = message_id_str.parse::<u64>().unwrap_or_default();

                Ok((
                    ChannelId::new(channel_id),
                    (
                        serenity::model::Timestamp::from_unix_timestamp(timestamp)
                            .unwrap_or_default(),
                        MessageId::new(message_id),
                    ),
                ))
            })?;

            let mut result = std::collections::HashMap::new();
            for row in rows {
                let (channel_id, timestamp_and_message_id) = row?;
                result.insert(channel_id, timestamp_and_message_id);
            }

            Ok::<_, rusqlite::Error>(result)
        })
        .await?;

    Ok(result)
}

/// Initialize the posted_news table for tracking shared article URLs
pub async fn initialize_posted_news_table(
    conn: &Arc<SqliteConnection>,
) -> Result<(), Box<dyn std::error::Error>> {
    conn.call(|conn| {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS posted_news (
                url TEXT PRIMARY KEY,
                posted_at INTEGER NOT NULL
            )",
            [],
        )?;
        Ok::<_, rusqlite::Error>(())
    })
    .await?;
    Ok(())
}

/// Load all posted news URLs from the database (for restoring state on startup)
pub async fn load_posted_news_urls(
    conn: &Arc<SqliteConnection>,
) -> Result<HashSet<String>, Box<dyn std::error::Error>> {
    let urls = conn
        .call(|conn| {
            let mut stmt = conn.prepare("SELECT url FROM posted_news")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            let mut result = HashSet::new();
            for url in rows.flatten() {
                result.insert(url);
            }
            Ok::<_, rusqlite::Error>(result)
        })
        .await?;
    Ok(urls)
}

/// Record a posted news URL in the database
pub async fn save_posted_news_url(
    conn: &Arc<SqliteConnection>,
    url: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let url = url.to_string();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    conn.call(move |conn| {
        conn.execute(
            "INSERT OR IGNORE INTO posted_news (url, posted_at) VALUES (?, ?)",
            rusqlite::params![url, timestamp],
        )?;
        Ok::<_, rusqlite::Error>(())
    })
    .await?;
    Ok(())
}

/// Clean up old posted news entries (older than given number of days)
pub async fn cleanup_old_posted_news(
    conn: &Arc<SqliteConnection>,
    days: u64,
) -> Result<usize, Box<dyn std::error::Error>> {
    let cutoff = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
        - (days as i64 * 86400);

    let deleted = conn
        .call(move |conn| {
            let count = conn.execute(
                "DELETE FROM posted_news WHERE posted_at < ?",
                rusqlite::params![cutoff],
            )?;
            Ok::<_, rusqlite::Error>(count)
        })
        .await?;
    Ok(deleted)
}
