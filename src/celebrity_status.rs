use anyhow::Result;
use serenity::all::Http;
use serenity::model::channel::Message;
use tracing::{error, info};
use reqwest::Client;
use serde_json::Value;
use chrono::{NaiveDate, Datelike};
use regex::Regex;

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
    
    // Check if this is a person (has birth/death dates in parentheses or "born"/"died" in the extract)
    let is_person = extract.contains(" born ") || extract.contains(" died ") || 
                    Regex::new(r"\([^)]*\d{4}[^)]*\)").ok().map_or(false, |re| re.is_match(extract));
    
    if !is_person {
        info!("Page doesn't appear to be about a person: {}", page_title);
        return Ok(Some(format!("I found information about '{}', but it doesn't appear to be a person.", page_title)));
    }
    
    // Try to extract birth and death dates from parentheses after the name
    let (birth_date, death_date, cleaned_extract) = extract_dates_from_parentheses(extract);
    
    // Get a short description (first sentence or two)
    let description = cleaned_extract
        .split('.')
        .take(2)
        .collect::<Vec<&str>>()
        .join(".")
        .trim()
        .to_string();
    
    // Determine if the person is dead
    let is_dead = death_date.is_some() || extract.contains(" died ");
    
    if is_dead {
        // If we have a death date from parentheses, use it
        if let Some(date) = death_date {
            info!("Using death date from parentheses: {}", date);
            return Ok(Some(format!("**{}**: {}. They died on {}.", page_title, description, date)));
        }
        
        // Otherwise try to extract death date from the text
        let extracted_death_date = extract_date(&cleaned_extract, "died");
        
        match extracted_death_date {
            Some(date) => {
                info!("Using extracted death date: {}", date);
                return Ok(Some(format!("**{}**: {}. They died on {}.", page_title, description, date)));
            },
            None => {
                info!("No death date found for {}", page_title);
                return Ok(Some(format!("**{}**: {}. They have died, but I couldn't determine the exact date.", page_title, description)));
            }
        }
    } else {
        // Person is alive
        // If we have a birth date from parentheses, use it to calculate age
        if let Some(date_str) = birth_date {
            // Try to parse the date in various formats
            let parsed_date = parse_date(&date_str);
            
            if let Some(birth) = parsed_date {
                // Calculate age
                let today = chrono::Local::now().naive_local().date();
                let age = calculate_age(birth, today);
                return Ok(Some(format!("**{}**: {}. They are still alive at {} years old.", page_title, description, age)));
            }
        }
        
        // Try to extract birth date from the text
        let extracted_birth_date = extract_date(&cleaned_extract, "born");
        
        match extracted_birth_date {
            Some(date_str) => {
                // Try to parse the date
                let parsed_date = parse_date(&date_str);
                
                if let Some(birth) = parsed_date {
                    // Calculate age
                    let today = chrono::Local::now().naive_local().date();
                    let age = calculate_age(birth, today);
                    return Ok(Some(format!("**{}**: {}. They are still alive at {} years old.", page_title, description, age)));
                } else {
                    return Ok(Some(format!("**{}**: {}. They are still alive, born on {}.", page_title, description, date_str)));
                }
            },
            None => {
                return Ok(Some(format!("**{}**: {}. They appear to be alive, but I couldn't determine their age.", page_title, description)));
            }
        }
    }
}

fn extract_dates_from_parentheses(text: &str) -> (Option<String>, Option<String>, String) {
    // Look for parentheses near the beginning of the text that contain dates
    let re = Regex::new(r"^([^(]*)\(([^)]+)\)(.*)$").unwrap();
    
    if let Some(captures) = re.captures(text) {
        let before = captures.get(1).map_or("", |m| m.as_str());
        let parentheses_content = captures.get(2).map_or("", |m| m.as_str());
        let after = captures.get(3).map_or("", |m| m.as_str());
        
        info!("Found parentheses content: {}", parentheses_content);
        
        // Extract birth and death dates from parentheses
        let birth_date = extract_year_from_parentheses(parentheses_content, "born");
        let death_date = extract_year_from_parentheses(parentheses_content, "died");
        
        // Create cleaned text without the parentheses
        let cleaned_text = format!("{}{}", before, after);
        
        return (birth_date, death_date, cleaned_text);
    }
    
    // If no parentheses found, return the original text
    (None, None, text.to_string())
}

fn extract_year_from_parentheses(text: &str, date_type: &str) -> Option<String> {
    // Common patterns in Wikipedia parentheses
    // Examples: "born January 20, 1930", "20 January 1930 – 15 April 2023"
    
    info!("Extracting {} date from parentheses: {}", date_type, text);
    
    if date_type == "born" {
        // Look for birth date
        // Pattern: "born January 20, 1930" or just a date at the beginning
        let born_re = Regex::new(r"(?:born|b\.)\s+([A-Za-z]+\s+\d{1,2},?\s+\d{4})").unwrap();
        if let Some(captures) = born_re.captures(text) {
            let date = captures.get(1).map(|m| m.as_str().to_string());
            info!("Found birth date with 'born' pattern: {:?}", date);
            return date;
        }
        
        // If there's a dash, the birth date is likely before it
        if text.contains('–') || text.contains('-') {
            let parts: Vec<&str> = text.split(|c| c == '–' || c == '-').collect();
            if !parts.is_empty() {
                let potential_date = parts[0].trim();
                // Check if it looks like a date (contains a year)
                if Regex::new(r"\d{4}").unwrap().is_match(potential_date) {
                    info!("Found birth date before dash: {}", potential_date);
                    return Some(potential_date.to_string());
                }
            }
        }
    } else if date_type == "died" {
        // Look for death date
        // Pattern: "died April 15, 2023" or date after a dash
        let died_re = Regex::new(r"(?:died|d\.)\s+([A-Za-z]+\s+\d{1,2},?\s+\d{4})").unwrap();
        if let Some(captures) = died_re.captures(text) {
            let date = captures.get(1).map(|m| m.as_str().to_string());
            info!("Found death date with 'died' pattern: {:?}", date);
            return date;
        }
        
        // If there's a dash, the death date is likely after it
        if text.contains('–') || text.contains('-') {
            let parts: Vec<&str> = text.split(|c| c == '–' || c == '-').collect();
            if parts.len() > 1 {
                let potential_date = parts[1].trim();
                // Check if it looks like a date (contains a year)
                if Regex::new(r"\d{4}").unwrap().is_match(potential_date) {
                    info!("Found death date after dash: {}", potential_date);
                    return Some(potential_date.to_string());
                }
            }
        }
    }
    
    info!("No {} date found in parentheses", date_type);
    None
}

fn extract_date(text: &str, keyword: &str) -> Option<String> {
    info!("Extracting {} date from text: {}", keyword, text);
    
    // Common date patterns in Wikipedia
    let patterns = [
        format!(r"{} on (\d+ [A-Za-z]+ \d{{4}})", keyword),
        format!(r"{} in ([A-Za-z]+ \d{{4}})", keyword),
        format!(r"{} (\d+ [A-Za-z]+ \d{{4}})", keyword),
        format!(r"{} at .* on (\d+ [A-Za-z]+ \d{{4}})", keyword),
        format!(r"{} .* on (\d+ [A-Za-z]+ \d{{4}})", keyword),
        // Add more patterns as needed
    ];
    
    for pattern in &patterns {
        info!("Trying pattern: {}", pattern);
        if let Some(captures) = regex::Regex::new(pattern).ok()?.captures(text) {
            if let Some(date_match) = captures.get(1) {
                let date = date_match.as_str().to_string();
                info!("Found date with pattern {}: {}", pattern, date);
                return Some(date);
            }
        }
    }
    
    // If we couldn't find a date with the specific patterns, try a more general approach
    // Look for dates near the keyword
    let keyword_pos = match text.find(keyword) {
        Some(pos) => pos,
        None => return None,
    };
    
    // Look for a date pattern within 100 characters after the keyword
    let search_end = (keyword_pos + 100).min(text.len());
    let search_text = &text[keyword_pos..search_end];
    
    info!("Searching for date in: {}", search_text);
    
    // General date patterns
    let general_patterns = [
        r"(\d{1,2} [A-Za-z]+ \d{4})",  // 20 April 2023
        r"([A-Za-z]+ \d{1,2}, \d{4})",  // April 20, 2023
        r"(\d{4}-\d{2}-\d{2})",         // 2023-04-20
    ];
    
    for pattern in &general_patterns {
        info!("Trying general pattern: {}", pattern);
        if let Some(captures) = regex::Regex::new(pattern).ok()?.captures(search_text) {
            if let Some(date_match) = captures.get(1) {
                let date = date_match.as_str().to_string();
                info!("Found date with general pattern {}: {}", pattern, date);
                return Some(date);
            }
        }
    }
    
    info!("No date found for keyword: {}", keyword);
    None
}

fn parse_date(date_str: &str) -> Option<NaiveDate> {
    // Try various date formats
    let formats = [
        "%d %B %Y",       // 20 April 2023
        "%B %d, %Y",      // April 20, 2023
        "%Y-%m-%d",       // 2023-04-20
        "%B %Y",          // April 2023
        "%d %b %Y",       // 20 Apr 2023
        "%b %d, %Y",      // Apr 20, 2023
    ];
    
    for format in &formats {
        if let Ok(date) = NaiveDate::parse_from_str(date_str, format) {
            return Some(date);
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
