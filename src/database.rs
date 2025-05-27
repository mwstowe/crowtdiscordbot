use anyhow::Result;
use mysql::{Pool, OptsBuilder, prelude::*};
use serenity::model::channel::Message;
use serenity::all::Http;
use tracing::{error, info};
use rand::Rng;

#[derive(Clone)]
pub struct DatabaseManager {
    pub pool: Option<Pool>,
}

impl DatabaseManager {
    pub fn new(host: Option<String>, db: Option<String>, user: Option<String>, password: Option<String>) -> Self {
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
    pub fn is_configured(&self) -> bool {
        self.pool.is_some()
    }
    
    // Add this method to test the connection
    pub fn test_connection(&self) -> Result<bool> {
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
    
    pub async fn query_random_entry(&self, http: &Http, msg: &Message, search_term: Option<String>, show_name: Option<String>, entry_type: &str) -> Result<()> {
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
                
                // Get a random slogan
                let random_index = rand::thread_rng().gen_range(0..total_entries);
                info!("Selected random index {} of {} for slogans", random_index, total_entries);
                
                let select_query = "SELECT pn_quote FROM nuke_quotes WHERE pn_quote LIKE ? LIMIT ?, 1";
                
                let slogan_result = conn.exec_first::<String, _, _>(
                    select_query,
                    (where_clause, random_index)
                );
                
                // Format and send the slogan
                match slogan_result {
                    Ok(Some(slogan_text)) => {
                        // Clean up HTML entities
                        let clean_slogan = html_escape::decode_html_entities(&slogan_text);
                        
                        msg.channel_id.say(http, format!("(Slogan {} of {}) {}", 
                            random_index + 1, total_entries, clean_slogan)).await?;
                    },
                    Ok(None) => {
                        error!("Query returned no results despite count being {}", total_entries);
                        msg.channel_id.say(http, "No slogans found.").await?;
                    },
                    Err(e) => {
                        error!("Failed to query slogan: {:?}", e);
                        msg.channel_id.say(http, "Failed to retrieve a slogan from the database.").await?;
                    }
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
