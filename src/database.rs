use anyhow::Result;
use mysql::{prelude::*, OptsBuilder, Pool};
use rand::RngExt;
use serenity::all::Http;
use serenity::model::channel::Message;
use tracing::{error, info};

#[derive(Clone)]
pub struct DatabaseManager {
    pub pool: Option<Pool>,
}

/// Result of a database query for a random entry (quote or slogan).
enum QueryResult {
    /// No pool configured
    NotConfigured,
    /// Failed to get a connection from the pool
    ConnectionFailed(String),
    /// Query executed but found no matching entries
    NoResults {
        search_term: Option<String>,
        show_name: Option<String>,
    },
    /// A quote was found
    Quote {
        quote_text: String,
        show_title: String,
        episode_num: String,
        episode_title: String,
        quote_index: i64,
        total: i64,
    },
    /// A slogan was found
    Slogan {
        slogan_text: String,
        slogan_index: i64,
        total: i64,
    },
    /// A database query error occurred
    QueryError(String),
}

impl DatabaseManager {
    pub fn is_configured(&self) -> bool {
        self.pool.is_some()
    }

    pub fn new(
        host: Option<String>,
        db: Option<String>,
        user: Option<String>,
        password: Option<String>,
    ) -> Self {
        info!(
            "Creating DatabaseManager with host={:?}, db={:?}, user={:?}, password={}",
            host,
            db,
            user,
            if password.is_some() {
                "provided"
            } else {
                "not provided"
            }
        );

        let pool = if let (Some(host), Some(db), Some(user), Some(password)) =
            (&host, &db, &user, &password)
        {
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
                        }
                        Err(e) => error!("❌ Could not get MySQL connection: {:?}", e),
                    }
                    Some(pool)
                }
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
            ]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", ");

            error!(
                "❌ MySQL database connection not configured - missing: {}",
                missing
            );
            None
        };

        Self { pool }
    }

    // Add this method to test the connection
    pub fn test_connection(&self) -> Result<bool> {
        if let Some(pool) = &self.pool {
            match pool.get_conn() {
                Ok(mut conn) => match conn.query_first::<String, _>("SELECT 'Connection test'") {
                    Ok(_) => {
                        info!("✅ MySQL connection test successful");
                        Ok(true)
                    }
                    Err(e) => {
                        error!("❌ MySQL connection test failed: {:?}", e);
                        Ok(false)
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

    /// Execute the blocking MySQL query on a dedicated thread, then send the
    /// result back to the async context for Discord message delivery.
    pub async fn query_random_entry(
        &self,
        http: &Http,
        msg: &Message,
        search_term: Option<String>,
        show_name: Option<String>,
        entry_type: &str,
    ) -> Result<()> {
        let entry_type_owned = entry_type.to_string();
        let pool = self.pool.clone();
        let search_term_clone = search_term.clone();
        let show_name_clone = show_name.clone();

        // Run all blocking MySQL operations on a dedicated thread
        let result = tokio::task::spawn_blocking(move || {
            Self::query_random_entry_blocking(
                pool,
                search_term_clone,
                show_name_clone,
                &entry_type_owned,
            )
        })
        .await
        .unwrap_or(QueryResult::QueryError(
            "Database task panicked".to_string(),
        ));

        // Handle the result back in async context (send Discord messages)
        match result {
            QueryResult::NotConfigured => {
                error!("❌ MySQL pool is None when handling {} command", entry_type);
                msg.channel_id
                    .say(http, "MySQL database is not configured.")
                    .await?;
            }
            QueryResult::ConnectionFailed(e) => {
                error!(
                    "❌ Failed to get MySQL connection for {} command: {}",
                    entry_type, e
                );
                msg.channel_id
                    .say(
                        http,
                        format!("Failed to connect to the {entry_type} database."),
                    )
                    .await?;
            }
            QueryResult::NoResults {
                search_term,
                show_name,
            } => {
                let mut message = match entry_type {
                    "quote" => "No quotes found".to_string(),
                    "slogan" => "No slogans found".to_string(),
                    _ => "No results found".to_string(),
                };
                if let Some(terms) = &search_term {
                    message.push_str(&format!(" matching '{terms}'"));
                }
                if let Some(show) = &show_name {
                    message.push_str(&format!(" in show '{show}'"));
                }
                msg.channel_id.say(http, message).await?;
            }
            QueryResult::Quote {
                quote_text,
                show_title,
                episode_num,
                episode_title,
                quote_index,
                total,
            } => {
                let clean_quote = html_escape::decode_html_entities(&quote_text);
                let quote_num = quote_index + 1;
                msg.channel_id
                    .say(
                        http,
                        format!(
                            "(Quote {quote_num} of {total}) {clean_quote} -- {show_title} {episode_num}: {episode_title}"
                        ),
                    )
                    .await?;
            }
            QueryResult::Slogan {
                slogan_text,
                slogan_index,
                total,
            } => {
                let clean_slogan = html_escape::decode_html_entities(&slogan_text);
                let slogan_num = slogan_index + 1;
                msg.channel_id
                    .say(
                        http,
                        format!("(Slogan {slogan_num} of {total}) {clean_slogan}"),
                    )
                    .await?;
            }
            QueryResult::QueryError(e) => {
                error!("❌ Database query error for {} command: {}", entry_type, e);
                msg.channel_id
                    .say(
                        http,
                        format!("Failed to retrieve a {entry_type} from the database."),
                    )
                    .await?;
            }
        }

        Ok(())
    }

    /// Blocking MySQL query logic. Runs on a dedicated thread via spawn_blocking.
    fn query_random_entry_blocking(
        pool: Option<Pool>,
        search_term: Option<String>,
        show_name: Option<String>,
        entry_type: &str,
    ) -> QueryResult {
        let pool = match &pool {
            Some(p) => p,
            None => return QueryResult::NotConfigured,
        };

        info!(
            "MySQL pool exists, attempting to get connection for {} command",
            entry_type
        );

        let mut conn = match pool.get_conn() {
            Ok(conn) => {
                info!(
                    "✅ Successfully got MySQL connection for {} command",
                    entry_type
                );
                conn
            }
            Err(e) => {
                return QueryResult::ConnectionFailed(format!("{:?}", e));
            }
        };

        // Build the WHERE clause based on search term
        let where_clause = if let Some(terms) = &search_term {
            let terms: Vec<&str> = terms.split_whitespace().collect();
            if !terms.is_empty() {
                let joined_terms = terms.join("%");
                format!("%{joined_terms}%")
            } else {
                "%".to_string()
            }
        } else {
            "%".to_string()
        };

        // Build the show clause based on show name
        let show_clause = if let Some(show) = &show_name {
            let show_terms: Vec<&str> = show.split_whitespace().collect();
            if !show_terms.is_empty() {
                let joined_show_terms = show_terms.join("%");
                format!("%{joined_show_terms}%")
            } else {
                "%".to_string()
            }
        } else {
            "%".to_string()
        };

        match entry_type {
            "quote" => {
                Self::query_quote(&mut conn, where_clause, show_clause, search_term, show_name)
            }
            "slogan" => Self::query_slogan(&mut conn, where_clause, search_term, show_name),
            _ => {
                error!("Unknown entry type: {}", entry_type);
                QueryResult::QueryError(format!("Unknown database query type: {}", entry_type))
            }
        }
    }

    fn query_quote(
        conn: &mut mysql::PooledConn,
        where_clause: String,
        show_clause: String,
        search_term: Option<String>,
        show_name: Option<String>,
    ) -> QueryResult {
        info!(
            "Executing quote query with where_clause: {} and show_clause: {}",
            where_clause, show_clause
        );

        // Count total matching quotes
        let count_query =
            "SELECT COUNT(*) FROM masterlist_quotes, masterlist_episodes, masterlist_shows \
                          WHERE masterlist_episodes.show_id = masterlist_shows.show_id \
                          AND masterlist_quotes.show_id = masterlist_shows.show_id \
                          AND masterlist_quotes.show_ep = masterlist_episodes.show_ep \
                          AND quote LIKE ? AND show_title LIKE ?";

        let total_entries = match conn
            .exec_first::<i64, _, _>(count_query, (where_clause.clone(), show_clause.clone()))
        {
            Ok(Some(count)) => {
                info!("Found {} matching quotes with show filter", count);
                count
            }
            Ok(None) => {
                info!("No matching quotes found with show filter");
                0
            }
            Err(e) => {
                error!("Failed to count quotes: {:?}", e);
                return QueryResult::QueryError(format!("Failed to count quotes: {:?}", e));
            }
        };

        if total_entries == 0 {
            return QueryResult::NoResults {
                search_term,
                show_name,
            };
        }

        // Get a random quote
        let random_index = rand::rng().random_range(0..total_entries);
        info!(
            "Selected random index {} of {} for quotes",
            random_index, total_entries
        );

        let select_query = "SELECT quote, show_title, masterlist_episodes.show_ep, title \
                           FROM masterlist_quotes, masterlist_episodes, masterlist_shows \
                           WHERE masterlist_episodes.show_id = masterlist_shows.show_id \
                           AND masterlist_quotes.show_id = masterlist_shows.show_id \
                           AND masterlist_quotes.show_ep = masterlist_episodes.show_ep \
                           AND quote LIKE ? AND show_title LIKE ? \
                           LIMIT ?, 1";

        match conn.exec_first::<(String, String, String, String), _, _>(
            select_query,
            (where_clause, show_clause, random_index),
        ) {
            Ok(Some((quote_text, show_title, episode_num, episode_title))) => QueryResult::Quote {
                quote_text,
                show_title,
                episode_num,
                episode_title,
                quote_index: random_index,
                total: total_entries,
            },
            Ok(None) => {
                error!(
                    "Query returned no results despite count being {}",
                    total_entries
                );
                QueryResult::NoResults {
                    search_term,
                    show_name,
                }
            }
            Err(e) => {
                error!("Failed to query quote: {:?}", e);
                QueryResult::QueryError(format!("Failed to query quote: {:?}", e))
            }
        }
    }

    fn query_slogan(
        conn: &mut mysql::PooledConn,
        where_clause: String,
        search_term: Option<String>,
        show_name: Option<String>,
    ) -> QueryResult {
        info!("Executing slogan query with where_clause: {}", where_clause);

        // Count total matching slogans
        let total_entries = match conn.exec_first::<i64, _, _>(
            "SELECT COUNT(*) FROM nuke_quotes WHERE pn_quote LIKE ?",
            (where_clause.clone(),),
        ) {
            Ok(Some(count)) => {
                info!("Found {} matching slogans", count);
                count
            }
            Ok(None) => {
                info!("No matching slogans found");
                0
            }
            Err(e) => {
                error!("Failed to count slogans: {:?}", e);
                return QueryResult::QueryError(format!("Failed to count slogans: {:?}", e));
            }
        };

        if total_entries == 0 {
            return QueryResult::NoResults {
                search_term,
                show_name,
            };
        }

        // Get a random slogan
        let random_index = rand::rng().random_range(0..total_entries);
        info!(
            "Selected random index {} of {} for slogans",
            random_index, total_entries
        );

        let select_query = "SELECT pn_quote FROM nuke_quotes WHERE pn_quote LIKE ? LIMIT ?, 1";

        match conn.exec_first::<String, _, _>(select_query, (where_clause, random_index)) {
            Ok(Some(slogan_text)) => QueryResult::Slogan {
                slogan_text,
                slogan_index: random_index,
                total: total_entries,
            },
            Ok(None) => {
                error!(
                    "Query returned no results despite count being {}",
                    total_entries
                );
                QueryResult::NoResults {
                    search_term,
                    show_name,
                }
            }
            Err(e) => {
                error!("Failed to query slogan: {:?}", e);
                QueryResult::QueryError(format!("Failed to query slogan: {:?}", e))
            }
        }
    }
}
