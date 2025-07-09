use tracing::{error, info};
use mysql::prelude::Queryable;
use rand::seq::SliceRandom;
use regex::Regex;

/// Process an MST3K quote from the database
/// 
/// This function:
/// 1. Retrieves a random MST3K quote from the database
/// 2. Processes the quote to extract character dialogue
/// 3. If multiple characters are speaking, selects one line randomly
/// 4. Removes speaker attribution
/// 5. Returns the formatted quote or None if any step fails
pub async fn process_mst3k_quote(
    pool: &mysql::Pool,
) -> Option<String> {
    // Query for a random MST3K quote
    match get_random_mst3k_quote(pool) {
        Some(quote) => {
            // Process the quote to extract dialogue
            format_mst3k_quote(&quote)
        },
        None => {
            // Silently fail if no quote is found
            error!("No MST3K quote found in database");
            None
        }
    }
}

/// Get a random MST3K quote from the database
fn get_random_mst3k_quote(pool: &mysql::Pool) -> Option<String> {
    // Get a connection from the pool
    match pool.get_conn() {
        Ok(mut conn) => {
            // Query for a random MST3K quote
            let query = "SELECT q.quote FROM masterlist_quotes q \
                       JOIN masterlist_episodes e ON q.show_id = e.show_id AND q.show_ep = e.show_ep \
                       JOIN masterlist_shows s ON e.show_id = s.show_id \
                       WHERE s.show_title LIKE '%Mystery Science Theater%' \
                       AND q.quote NOT LIKE '%Watch out for snakes%' \
                       ORDER BY RAND() LIMIT 1";
            
            match conn.query_first::<String, _>(query) {
                Ok(Some(quote)) => {
                    info!("Retrieved random MST3K quote: {}", quote);
                    Some(quote)
                },
                Ok(None) => {
                    error!("No MST3K quotes found in the database");
                    None
                },
                Err(e) => {
                    error!("Error querying for MST3K quote: {:?}", e);
                    None
                }
            }
        },
        Err(e) => {
            error!("Error connecting to MySQL database for MST3K quote: {:?}", e);
            None
        }
    }
}

/// Format an MST3K quote by extracting dialogue and removing speaker attribution
fn format_mst3k_quote(quote: &str) -> Option<String> {
    // Clean up HTML entities
    let clean_quote = html_escape::decode_html_entities(quote).to_string();
    
    // Try to extract character name and quote (format: "Character: Quote")
    if let Some(formatted) = extract_character_quote(&clean_quote) {
        return Some(formatted);
    }
    
    // Try to extract speaker-line pairs (format: "<Speaker> Line <Speaker> Line")
    if let Some(formatted) = extract_speaker_lines(&clean_quote) {
        return Some(formatted);
    }
    
    // If we couldn't extract anything, use the whole quote as is
    if !clean_quote.is_empty() {
        Some(clean_quote)
    } else {
        None
    }
}

/// Extract a character quote from the format "Character: Quote"
fn extract_character_quote(quote: &str) -> Option<String> {
    if let Some(colon_pos) = quote.find(':') {
        if colon_pos > 0 && colon_pos < quote.len() - 1 {
            let character_quote = quote[colon_pos+1..].trim();
            
            // Only use if we have a non-empty quote
            if !character_quote.is_empty() {
                return Some(character_quote.to_string());
            }
        }
    }
    None
}

/// Extract speaker lines from the format "<Speaker> Line <Speaker> Line"
fn extract_speaker_lines(quote: &str) -> Option<String> {
    let re = match Regex::new(r"<([^>]+)>\s*([^<]+)") {
        Ok(re) => re,
        Err(e) => {
            error!("Failed to compile regex for MST3K quote parsing: {:?}", e);
            return None;
        }
    };
    
    // Find all speaker-line pairs
    let mut lines = Vec::new();
    for cap in re.captures_iter(quote) {
        if let (Some(_speaker), Some(line_match)) = (cap.get(1), cap.get(2)) {
            let line = line_match.as_str().trim();
            if !line.is_empty() {
                lines.push(line.to_string());
            }
        }
    }
    
    // If we found any lines, pick one randomly
    if !lines.is_empty() {
        lines.choose(&mut rand::thread_rng()).cloned()
    } else {
        None
    }
}
