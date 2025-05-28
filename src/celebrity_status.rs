use anyhow::Result;
use serenity::all::Http;
use serenity::model::channel::Message;
use tracing::{error, info};
use reqwest::Client;
use serde_json::Value;
use chrono::{NaiveDate, Datelike};

pub async fn handle_aliveordead_command(http: &Http, msg: &Message, celebrity_name: &str) -> Result<()> {
    info!("Handling !aliveordead command for celebrity: {}", celebrity_name);
    
    // Show typing indicator while processing
    if let Err(e) = msg.channel_id.broadcast_typing(http).await {
        error!("Failed to send typing indicator: {:?}", e);
    }
    
    // Search for the celebrity using the Wikipedia API
    match search_celebrity(celebrity_name).await {
        Ok(Some(result)) => {
            // Send the result
            if let Err(e) = msg.channel_id.say(http, result).await {
                error!("Error sending celebrity status: {:?}", e);
                msg.reply(http, "Sorry, I couldn't send the celebrity information.").await?;
            }
        },
        Ok(None) => {
            msg.reply(http, format!("Sorry, I couldn't find information about '{}'.", celebrity_name)).await?;
        },
        Err(e) => {
            error!("Error searching for celebrity: {:?}", e);
            msg.reply(http, "Sorry, I encountered an error while searching for that celebrity.").await?;
        }
    }
    
    Ok(())
}

async fn search_celebrity(name: &str) -> Result<Option<String>> {
    let client = Client::new();
    
    // First, search for the page
    let search_url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&list=search&srsearch={}&format=json&srlimit=1",
        urlencoding::encode(name)
    );
    
    info!("Searching Wikipedia for: {}", name);
    let search_response = client.get(&search_url).send().await?;
    let search_json: Value = search_response.json().await?;
    
    // Extract the page title from search results
    let page_title = match search_json
        .get("query")
        .and_then(|q| q.get("search"))
        .and_then(|s| s.get(0))
        .and_then(|r| r.get("title"))
        .and_then(|t| t.as_str()) {
            Some(title) => title,
            None => {
                info!("No search results found for: {}", name);
                return Ok(None);
            }
        };
    
    info!("Found Wikipedia page: {}", page_title);
    
    // Now get the page content
    let page_url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&prop=extracts|pageprops&exintro&explaintext&redirects=1&titles={}&format=json",
        urlencoding::encode(page_title)
    );
    
    let page_response = client.get(&page_url).send().await?;
    let page_json: Value = page_response.json().await?;
    
    // Extract the page ID
    let pages = match page_json.get("query").and_then(|q| q.get("pages")) {
        Some(p) => p,
        None => {
            info!("No page data found for: {}", page_title);
            return Ok(None);
        }
    };
    
    // Get the first page (there should only be one)
    let page_id = match pages.as_object().and_then(|o| o.keys().next()) {
        Some(id) => id,
        None => {
            info!("No page ID found for: {}", page_title);
            return Ok(None);
        }
    };
    
    // Extract the extract (page content)
    let extract = match pages.get(page_id).and_then(|p| p.get("extract")).and_then(|e| e.as_str()) {
        Some(e) => e,
        None => {
            info!("No extract found for page: {}", page_title);
            return Ok(None);
        }
    };
    
    // Check if this is a person (has "born" or "died" in the extract)
    if !extract.contains(" born ") && !extract.contains(" died ") {
        info!("Page doesn't appear to be about a person: {}", page_title);
        return Ok(Some(format!("I found information about '{}', but it doesn't appear to be a person.", page_title)));
    }
    
    // Get a short description (first sentence or two)
    let description = extract
        .split('.')
        .take(2)
        .collect::<Vec<&str>>()
        .join(".");
    
    // Check for death information
    let is_dead = extract.contains(" died ");
    
    if is_dead {
        // Try to extract death date
        let death_date = extract_date(extract, "died");
        
        match death_date {
            Some(date) => {
                return Ok(Some(format!("**{}**: {}. They died on {}.", page_title, description, date)));
            },
            None => {
                return Ok(Some(format!("**{}**: {}. They have died, but I couldn't determine the exact date.", page_title, description)));
            }
        }
    } else {
        // Try to extract birth date to calculate age
        let birth_date = extract_date(extract, "born");
        
        match birth_date {
            Some(date) => {
                // Try to parse the date
                if let Ok(parsed_date) = NaiveDate::parse_from_str(&date, "%d %B %Y") {
                    // Calculate age
                    let today = chrono::Local::now().naive_local().date();
                    let age = calculate_age(parsed_date, today);
                    return Ok(Some(format!("**{}**: {}. They are still alive at {} years old.", page_title, description, age)));
                } else {
                    return Ok(Some(format!("**{}**: {}. They are still alive, born on {}.", page_title, description, date)));
                }
            },
            None => {
                return Ok(Some(format!("**{}**: {}. They appear to be alive, but I couldn't determine their age.", page_title, description)));
            }
        }
    }
}

fn extract_date(text: &str, keyword: &str) -> Option<String> {
    // Common date patterns in Wikipedia
    let patterns = [
        format!(r"{} on (\d+ [A-Za-z]+ \d{{4}})", keyword),
        format!(r"{} in ([A-Za-z]+ \d{{4}})", keyword),
        format!(r"{} (\d+ [A-Za-z]+ \d{{4}})", keyword),
    ];
    
    for pattern in patterns {
        if let Some(captures) = regex::Regex::new(&pattern).ok()?.captures(text) {
            if let Some(date_match) = captures.get(1) {
                return Some(date_match.as_str().to_string());
            }
        }
    }
    
    None
}

fn calculate_age(birth_date: NaiveDate, today: NaiveDate) -> u32 {
    let mut age = today.year() - birth_date.year();
    
    // Adjust age if birthday hasn't occurred yet this year
    if today.month() < birth_date.month() || 
       (today.month() == birth_date.month() && today.day() < birth_date.day()) {
        age -= 1;
    }
    
    age as u32
}
