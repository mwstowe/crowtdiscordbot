use anyhow::Result;
use humantime::format_duration;
use serenity::all::Message;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tokio_rusqlite::Connection as SqliteConnection;
use tracing::error;

pub struct LastSeenFinder;

impl LastSeenFinder {
    pub fn new() -> Self {
        Self {}
    }

    // Find the last message from a user by name (nickname, display name, or username)
    pub async fn find_last_message(
        &self,
        conn: Arc<Mutex<SqliteConnection>>,
        name: &str,
    ) -> Result<Option<(String, String, String, u64)>, anyhow::Error> {
        let name_pattern = format!("%{}%", name.to_lowercase());
        let conn_guard = conn.lock().await;

        // Query the database for the most recent message from a user matching the name pattern
        let result = conn_guard
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT author, display_name, content, timestamp FROM messages
                 WHERE LOWER(author) LIKE ?1 OR LOWER(display_name) LIKE ?1
                 ORDER BY timestamp DESC LIMIT 1",
                )?;

                let rows = stmt.query_map([&name_pattern], |row| {
                    Ok((
                        row.get::<_, String>(0)?,                                   // author
                        row.get::<_, String>(1).unwrap_or_else(|_| "".to_string()), // display_name
                        row.get::<_, String>(2)?,                                   // content
                        row.get::<_, u64>(3)?,                                      // timestamp
                    ))
                })?;

                let mut result = None;
                for row_result in rows {
                    if let Ok(row) = row_result {
                        result = Some(row);
                        break;
                    }
                }

                Ok::<_, rusqlite::Error>(result)
            })
            .await?;

        Ok(result)
    }

    // Format the time difference between now and the timestamp
    pub fn format_time_ago(&self, timestamp: u64) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        if now < timestamp {
            return "in the future (clock mismatch?)".to_string();
        }

        let duration = std::time::Duration::from_secs(now - timestamp);
        format!("{}", format_duration(duration))
    }
}

pub async fn handle_lastseen_command(
    http: &serenity::http::Http,
    msg: &Message,
    name: &str,
    db_conn: &Option<Arc<Mutex<SqliteConnection>>>,
) -> Result<()> {
    if name.is_empty() {
        if let Err(e) = msg.channel_id.say(http, "Usage: !lastseen [name]").await {
            error!("Error sending usage message: {:?}", e);
        }
        return Ok(());
    }

    // Get the bot's current user information
    let current_user = match http.get_current_user().await {
        Ok(user) => user,
        Err(e) => {
            error!("Error getting current user: {:?}", e);
            if let Err(e) = msg
                .channel_id
                .say(http, "Error retrieving bot information")
                .await
            {
                error!("Error sending error message: {:?}", e);
            }
            return Ok(());
        }
    };

    // Check if the user is asking about the bot itself
    let bot_name = current_user.name.to_lowercase();
    let name_lower = name.to_lowercase();

    if name_lower.contains(&bot_name) || bot_name.contains(&name_lower) {
        if let Err(e) = msg.channel_id.say(http, "I'm right here!").await {
            error!("Error sending bot presence message: {:?}", e);
        }
        return Ok(());
    }

    // Check if the user is asking about themselves
    let author_name = msg.author.name.to_lowercase();
    let author_display_name = msg
        .author_nick(http)
        .await
        .unwrap_or_else(|| msg.author.global_name.clone().unwrap_or_default())
        .to_lowercase();

    if name_lower.contains(&author_name)
        || author_name.contains(&name_lower)
        || (!author_display_name.is_empty()
            && (name_lower.contains(&author_display_name)
                || author_display_name.contains(&name_lower)))
    {
        if let Err(e) = msg.channel_id.say(http, "You're right here!").await {
            error!("Error sending self-reference message: {:?}", e);
        }
        return Ok(());
    }

    // Check if we have a database connection
    if let Some(conn) = db_conn {
        let finder = LastSeenFinder::new();

        match finder.find_last_message(conn.clone(), name).await {
            Ok(Some((author, display_name, content, timestamp))) => {
                // Use display name if available, otherwise use author
                let user_name = if !display_name.is_empty() {
                    display_name
                } else {
                    author
                };

                let time_ago = finder.format_time_ago(timestamp);
                let response = format!(
                    "{} was last seen {} ago, saying: \"{}\"",
                    user_name, time_ago, content
                );

                if let Err(e) = msg.channel_id.say(http, response).await {
                    error!("Error sending lastseen response: {:?}", e);
                }
            }
            Ok(None) => {
                if let Err(e) = msg
                    .channel_id
                    .say(http, format!("I haven't seen anyone matching \"{}\"", name))
                    .await
                {
                    error!("Error sending no match message: {:?}", e);
                }
            }
            Err(e) => {
                error!("Error finding last message: {:?}", e);
                if let Err(e) = msg
                    .channel_id
                    .say(http, "Error searching message history")
                    .await
                {
                    error!("Error sending error message: {:?}", e);
                }
            }
        }
    } else if let Err(e) = msg
        .channel_id
        .say(http, "Message history database is not available")
        .await
    {
        error!("Error sending database unavailable message: {:?}", e);
    }

    Ok(())
}
