use std::collections::VecDeque;
use tokio_rusqlite::Connection as SqliteConnection;
use tracing::info;
use std::sync::Arc;
use tokio::sync::Mutex;

// Initialize SQLite database for message history
pub async fn initialize_database(db_path: &str) -> Result<SqliteConnection, Box<dyn std::error::Error>> {
    let conn = SqliteConnection::open(db_path).await?;
    
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
    
    Ok(conn)
}

// Load message history from SQLite database
pub async fn load_message_history(
    conn: Arc<Mutex<SqliteConnection>>,
    history: &mut VecDeque<(String, String)>,
    limit: usize
) -> Result<(), Box<dyn std::error::Error>> {
    let conn_guard = conn.lock().await;
    let messages = conn_guard.call(move |conn| {
        let mut stmt = conn.prepare(
            &format!("SELECT author, content FROM messages ORDER BY timestamp DESC LIMIT {}", limit)
        )?;
        
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        
        Ok::<_, rusqlite::Error>(result)
    }).await?;
    
    for (author, content) in messages {
        history.push_back((author, content));
    }
    
    info!("Loaded {} messages from database", history.len());
    Ok(())
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
        // First, ensure the table has a display_name column
        let _ = conn.execute(
            "ALTER TABLE messages ADD COLUMN display_name TEXT",
            [],
        );
        
        conn.execute(
            "INSERT INTO messages (author, display_name, content, timestamp) VALUES (?1, ?2, ?3, ?4)",
            [&author, &clean_display_name, &content, &timestamp.to_string()],
        )?;
        Ok::<_, rusqlite::Error>(())
    }).await?;
    
    Ok(())
}

// Trim the message history database to the specified limit
pub async fn trim_message_history(
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
