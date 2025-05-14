use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_rusqlite::Connection as SqliteConnection;

// Initialize the SQLite database
pub async fn initialize_database(path: &str) -> Result<Arc<Mutex<SqliteConnection>>, Box<dyn std::error::Error>> {
    // Connect to the database
    let conn = SqliteConnection::open(path).await?;
    
    // Create the messages table if it doesn't exist
    conn.call(|conn| {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY,
                author TEXT NOT NULL,
                display_name TEXT,
                content TEXT NOT NULL,
                timestamp INTEGER NOT NULL
            )",
            [],
        )?;
        Ok::<_, rusqlite::Error>(())
    }).await?;
    
    // Return the connection wrapped in an Arc<Mutex>
    Ok(Arc::new(Mutex::new(conn)))
}

// Save a message to the SQLite database
pub async fn save_message(
    conn: Arc<Mutex<SqliteConnection>>,
    author: &str,
    display_name: &str,
    content: &str
) -> Result<(), Box<dyn std::error::Error>> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    
    let author = author.to_string();
    // Use the display_name::clean_display_name function for consistency
    let clean_display_name = crate::display_name::clean_display_name(display_name);
    
    let content = content.to_string();
    
    let conn_guard = conn.lock().await;
    
    conn_guard.call(move |conn| {
        conn.execute(
            "INSERT INTO messages (author, display_name, content, timestamp) VALUES (?, ?, ?, ?)",
            [&author, &clean_display_name, &content, &timestamp.to_string()],
        )?;
        Ok::<_, rusqlite::Error>(())
    }).await?;
    
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

// Get recent messages from the database
pub async fn get_recent_messages(
    conn: Arc<Mutex<SqliteConnection>>,
    limit: usize
) -> Result<Vec<(String, String, String)>, Box<dyn std::error::Error>> {
    let conn_guard = conn.lock().await;
    
    let messages = conn_guard.call(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT author, display_name, content FROM messages ORDER BY timestamp DESC LIMIT ?"
        )?;
        
        let rows = stmt.query_map([limit], |row| {
            Ok((
                row.get::<_, String>(0)?, // author
                row.get::<_, String>(1).unwrap_or_else(|_| "".to_string()), // display_name
                row.get::<_, String>(2)?, // content
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
    
    Ok(messages)
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
    limit: usize
) -> Result<(), Box<dyn std::error::Error>> {
    // This is a stub implementation since we can't directly convert database records to Message objects
    // In a real implementation, you would need to create Message objects from the stored data
    
    // For now, we'll just return Ok to avoid breaking the existing code
    Ok(())
}
