use anyhow::Result;
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

    // Find the last message from a user by author_id
    pub async fn find_last_message_by_id(
        &self,
        conn: Arc<Mutex<SqliteConnection>>,
        author_id: &str,
    ) -> Result<Option<(String, String, String, u64)>, anyhow::Error> {
        let author_id = author_id.to_string();
        let conn_guard = conn.lock().await;

        let result = conn_guard
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT author, display_name, content, timestamp FROM messages
                 WHERE author_id = ?1 AND content != ''
                 ORDER BY timestamp DESC LIMIT 1",
                )?;

                let rows = stmt.query_map([&author_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1).unwrap_or_else(|_| "".to_string()),
                        row.get::<_, String>(2)?,
                        row.get::<_, u64>(3)?,
                    ))
                })?;

                let result = rows.flatten().next();

                Ok::<_, rusqlite::Error>(result)
            })
            .await?;

        Ok(result)
    }

    // Find the last message from a user by name (nickname, display name, or username)
    pub async fn find_last_message(
        &self,
        conn: Arc<Mutex<SqliteConnection>>,
        name: &str,
    ) -> Result<Option<(String, String, String, u64)>, anyhow::Error> {
        let name_lower = name.to_lowercase();
        let name_pattern = format!("%{name_lower}%");
        let conn_guard = conn.lock().await;

        // Query the database for the most recent message from a user matching the name pattern
        let result = conn_guard
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT author, display_name, content, timestamp FROM messages
                 WHERE (LOWER(author) LIKE ?1 OR LOWER(display_name) LIKE ?1) AND content != ''
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

                let result = rows.flatten().next();

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

        let mut remaining = now - timestamp;

        let years = remaining / (365 * 86400);
        remaining %= 365 * 86400;
        let months = remaining / (30 * 86400);
        remaining %= 30 * 86400;
        let days = remaining / 86400;
        remaining %= 86400;
        let hours = remaining / 3600;
        remaining %= 3600;
        let minutes = remaining / 60;
        let seconds = remaining % 60;

        let mut parts = Vec::new();
        if years > 0 {
            parts.push(format!(
                "{} {}",
                years,
                if years == 1 { "year" } else { "years" }
            ));
        }
        if months > 0 {
            parts.push(format!(
                "{} {}",
                months,
                if months == 1 { "month" } else { "months" }
            ));
        }
        if days > 0 {
            parts.push(format!(
                "{} {}",
                days,
                if days == 1 { "day" } else { "days" }
            ));
        }
        if hours > 0 {
            parts.push(format!(
                "{} {}",
                hours,
                if hours == 1 { "hour" } else { "hours" }
            ));
        }
        if minutes > 0 {
            parts.push(format!(
                "{} {}",
                minutes,
                if minutes == 1 { "minute" } else { "minutes" }
            ));
        }
        if seconds > 0 || parts.is_empty() {
            parts.push(format!(
                "{} {}",
                seconds,
                if seconds == 1 { "second" } else { "seconds" }
            ));
        }

        parts.join(", ")
    }
}

pub async fn handle_lastseen_command(
    http: &serenity::http::Http,
    msg: &Message,
    name: &str,
    user_id: Option<&str>,
    db_conn: &Option<Arc<Mutex<SqliteConnection>>>,
) -> Result<()> {
    if name.is_empty() && user_id.is_none() {
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

        let result = if let Some(uid) = user_id {
            finder.find_last_message_by_id(conn.clone(), uid).await
        } else {
            finder.find_last_message(conn.clone(), name).await
        };

        match result {
            Ok(Some((author, display_name, content, timestamp))) => {
                // Use display name if available, otherwise use author
                let user_name = if !display_name.is_empty() {
                    display_name
                } else {
                    author
                };

                let time_ago = finder.format_time_ago(timestamp);
                let response =
                    format!("{user_name} was last seen {time_ago} ago, saying: \"{content}\"");

                if let Err(e) = msg.channel_id.say(http, response).await {
                    error!("Error sending lastseen response: {:?}", e);
                }
            }
            Ok(None) => {
                if let Err(e) = msg
                    .channel_id
                    .say(http, format!("I haven't seen anyone matching \"{name}\""))
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
