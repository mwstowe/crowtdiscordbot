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

// Function to determine gender and return appropriate pronouns
fn determine_gender(text: &str) -> (&'static str, &'static str, &'static str) {
    // Default to they/them/their
    let mut subject = "they";
    let mut object = "them";
    let mut possessive = "their";
    
    // Look for gendered pronouns in the text
    let text_lower = text.to_lowercase();
    
    // Check for male indicators
    if text_lower.contains(" he ") || text_lower.contains(" his ") || text_lower.contains(" him ") || 
       text_lower.contains(" himself ") || text_lower.contains(" mr. ") || text_lower.contains(" mr ") ||
       text_lower.contains(" actor ") || text_lower.contains(" father ") || text_lower.contains(" son ") ||
       text_lower.contains(" brother ") || text_lower.contains(" husband ") || text_lower.contains(" boyfriend ") {
        subject = "he";
        object = "him";
        possessive = "his";
        info!("Gender detection: Male pronouns detected");
    }
    // Check for female indicators
    else if text_lower.contains(" she ") || text_lower.contains(" her ") || text_lower.contains(" hers ") || 
            text_lower.contains(" herself ") || text_lower.contains(" mrs. ") || text_lower.contains(" mrs ") ||
            text_lower.contains(" ms. ") || text_lower.contains(" ms ") || text_lower.contains(" miss ") ||
            text_lower.contains(" actress ") || text_lower.contains(" mother ") || text_lower.contains(" daughter ") ||
            text_lower.contains(" sister ") || text_lower.contains(" wife ") || text_lower.contains(" girlfriend ") {
        subject = "she";
        object = "her";
        possessive = "her";
        info!("Gender detection: Female pronouns detected");
    } else {
        info!("Gender detection: No clear gender indicators, using they/them");
    }
    
    (subject, object, possessive)
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
    let raw_extract = match pages.get(page_id).and_then(|p| p.get("extract")).and_then(|e| e.as_str()) {
        Some(e) => e,
        None => {
            info!("No extract found for page: {}", page_title);
            return Ok(None);
        }
    };
    
    info!("Raw extract: {}", raw_extract);
    
    // Check if this is a person (has birth/death dates in parentheses or "born"/"died" in the extract)
    let is_person = raw_extract.contains(" born ") || raw_extract.contains(" died ") || 
                    Regex::new(r"\([^)]*\d{4}[^)]*\)").ok().map_or(false, |re| re.is_match(raw_extract));
    
    if !is_person {
        info!("Page doesn't appear to be about a person: {}", page_title);
        return Ok(Some(format!("I found information about '{}', but it doesn't appear to be a person.", page_title)));
    }
    
    // Determine gender for proper pronoun usage
    let (subject_pronoun, object_pronoun, possessive_pronoun) = determine_gender(raw_extract);
    info!("Using pronouns: {}/{}/{}", subject_pronoun, object_pronoun, possessive_pronoun);
    
    // Try to extract birth and death dates from parentheses after the name
    let (birth_date, death_date, cleaned_extract) = extract_dates_from_parentheses(raw_extract);
    
    info!("FINAL EXTRACTION RESULTS - Birth date: {:?}, Death date: {:?}", birth_date, death_date);
    info!("Cleaned extract: {}", cleaned_extract);
    
    // Get a short description (first sentence or two)
    let description = cleaned_extract
        .split('.')
        .take(2)
        .collect::<Vec<&str>>()
        .join(".")
        .trim()
        .to_string();
    
    // Build the response
    let mut response = format!("**{}**: {}", page_title, description);
    
    // Determine if the person is dead and add appropriate information
    let contains_was = raw_extract.contains(" was ");
    let contains_is = raw_extract.contains(" is ");
    
    // Debug the death detection logic
    info!("Death detection - death_date: {:?}, contains_was: {}, contains_is: {}", 
          death_date, contains_was, contains_is);
    
    // A person is considered dead if:
    // 1. We have a death date from parentheses, OR
    // 2. The text uses past tense ("was") and not present tense ("is") and has parentheses (likely birth-death dates)
    let is_dead = death_date.is_some() || 
                 (contains_was && !contains_is && raw_extract.contains("(") && raw_extract.contains(")"));
    
    info!("Is dead determination: {}", is_dead);
    
    if is_dead {
        // Try to extract cause of death
        let cause_of_death = extract_cause_of_death(raw_extract);
        
        // First check if we have a death date from parentheses
        if let Some(date) = death_date {
            info!("Using death date from parentheses: {}", date);
            let mut death_info = format!(". {} died on {}.", 
                subject_pronoun.to_string().to_uppercase().chars().next().unwrap().to_string() + &subject_pronoun[1..], 
                date);
            
            // Add cause of death if available
            if let Some(cause) = cause_of_death {
                death_info.push_str(&format!(" Cause of death: {}.", cause));
            }
            
            response.push_str(&death_info);
            return Ok(Some(response));
        }
        
        // If not, try to extract death date from the text
        if let Some(date) = extract_date(&cleaned_extract, "died") {
            info!("Using extracted death date: {}", date);
            let mut death_info = format!(". {} died on {}.", 
                subject_pronoun.to_string().to_uppercase().chars().next().unwrap().to_string() + &subject_pronoun[1..], 
                date);
            
            // Add cause of death if available
            if let Some(cause) = cause_of_death {
                death_info.push_str(&format!(" Cause of death: {}.", cause));
            }
            
            response.push_str(&death_info);
            return Ok(Some(response));
        }
        
        // If we still don't have a death date
        info!("No death date found for {}", page_title);
        let mut death_info = format!(". {} has died, but I couldn't determine the exact date.", 
            subject_pronoun.to_string().to_uppercase().chars().next().unwrap().to_string() + &subject_pronoun[1..]);
        
        // Add cause of death if available
        if let Some(cause) = cause_of_death {
            death_info.push_str(&format!(" Cause of death: {}.", cause));
        }
        
        response.push_str(&death_info);
        return Ok(Some(response));
    } else {
        // Person is alive - try to calculate their age
        
        // First try with birth date from parentheses
        if let Some(date_str) = birth_date {
            if let Some(birth) = parse_date(&date_str) {
                let today = chrono::Local::now().naive_local().date();
                let age = calculate_age(birth, today);
                info!("Calculated age {} from birth date {}", age, date_str);
                response.push_str(&format!(". {} is still alive at {} years old.", subject_pronoun.to_string().to_uppercase().chars().next().unwrap().to_string() + &subject_pronoun[1..], age));
                return Ok(Some(response));
            }
        }
        
        // If that fails, try with birth date from text
        if let Some(date_str) = extract_date(&cleaned_extract, "born") {
            if let Some(birth) = parse_date(&date_str) {
                let today = chrono::Local::now().naive_local().date();
                let age = calculate_age(birth, today);
                info!("Calculated age {} from extracted birth date {}", age, date_str);
                response.push_str(&format!(". {} is still alive at {} years old.", subject_pronoun.to_string().to_uppercase().chars().next().unwrap().to_string() + &subject_pronoun[1..], age));
                return Ok(Some(response));
            } else {
                // We have a birth date string but couldn't parse it
                response.push_str(&format!(". {} is still alive, born on {}.", subject_pronoun.to_string().to_uppercase().chars().next().unwrap().to_string() + &subject_pronoun[1..], date_str));
                return Ok(Some(response));
            }
        }
        
        // If we couldn't determine age
        response.push_str(&format!(". {} appears to be alive, but I couldn't determine {}_age.", subject_pronoun.to_string().to_uppercase().chars().next().unwrap().to_string() + &subject_pronoun[1..], possessive_pronoun));
        return Ok(Some(response));
    }
}

pub fn extract_dates_from_parentheses(text: &str) -> (Option<String>, Option<String>, String) {
    // Special case for the exact format we're seeing
    if let Some(start_idx) = text.find('(') {
        if let Some(end_idx) = text[start_idx..].find(')') {
            let end_idx = start_idx + end_idx;
            let parentheses_content = &text[start_idx+1..end_idx];
            
            info!("Direct extraction - parentheses content: {}", parentheses_content);
            
            // Check for birth-death format with en dash or hyphen
            if parentheses_content.contains('–') || parentheses_content.contains('-') {
                let separator = if parentheses_content.contains('–') { '–' } else { '-' };
                let parts: Vec<&str> = parentheses_content.split(separator).collect();
                
                if parts.len() == 2 {
                    let birth_part = parts[0].trim();
                    let death_part = parts[1].trim();
                    
                    // Check if both parts look like dates (contain years)
                    let year_regex = Regex::new(r"\d{4}").unwrap();
                    if year_regex.is_match(birth_part) && year_regex.is_match(death_part) {
                        // Create cleaned text without the parentheses
                        // Remove any double spaces that might be created when removing parentheses
                        let mut cleaned_text = format!("{}{}", &text[0..start_idx], &text[end_idx+1..]);
                        cleaned_text = cleaned_text.replace("  ", " ");
                        
                        info!("DIRECT EXTRACTION SUCCESS - Birth: {}, Death: {}", birth_part, death_part);
                        info!("Cleaned text: {}", cleaned_text);
                        
                        return (Some(birth_part.to_string()), Some(death_part.to_string()), cleaned_text);
                    }
                }
            }
        }
    }
    
    // If the direct approach didn't work, fall back to the regex approach
    let re = Regex::new(r"^(.*?)\(([^)]+)\)(.*)$").unwrap();
    
    if let Some(captures) = re.captures(text) {
        let before = captures.get(1).map_or("", |m| m.as_str());
        let parentheses_content = captures.get(2).map_or("", |m| m.as_str());
        let after = captures.get(3).map_or("", |m| m.as_str());
        
        info!("Regex extraction - parentheses content: {}", parentheses_content);
        
        // Create cleaned text without the parentheses
        // Remove any double spaces that might be created when removing parentheses
        let mut cleaned_text = format!("{}{}", before, after);
        cleaned_text = cleaned_text.replace("  ", " ");
        
        // Direct check for birth-death date format
        if parentheses_content.contains('–') || parentheses_content.contains('-') {
            let separator = if parentheses_content.contains('–') { '–' } else { '-' };
            let parts: Vec<&str> = parentheses_content.split(separator).collect();
            
            if parts.len() == 2 {
                let birth_part = parts[0].trim();
                let death_part = parts[1].trim();
                
                // Check if both parts look like dates (contain years)
                let year_regex = Regex::new(r"\d{4}").unwrap();
                if year_regex.is_match(birth_part) && year_regex.is_match(death_part) {
                    info!("REGEX EXTRACTION SUCCESS - Birth: {}, Death: {}", birth_part, death_part);
                    return (Some(birth_part.to_string()), Some(death_part.to_string()), cleaned_text);
                }
            }
        }
        
        // If direct extraction didn't work, try the more complex patterns
        let birth_date = extract_year_from_parentheses(parentheses_content, "born");
        let death_date = extract_year_from_parentheses(parentheses_content, "died");
        
        info!("Pattern-based extraction - Birth date: {:?}, Death date: {:?}", birth_date, death_date);
        
        return (birth_date, death_date, cleaned_text);
    }
    
    // If no parentheses found, return the original text
    info!("No parentheses found in text");
    (None, None, text.to_string())
}

pub fn extract_year_from_parentheses(text: &str, date_type: &str) -> Option<String> {
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
        
        // Special case for future dates - if the year is greater than current year
        let current_year = chrono::Local::now().year();
        let future_year_re = Regex::new(&format!(r"(\w+\s+\d{{1,2}},?\s+({}-\d{{4}}))", current_year)).unwrap();
        if let Some(captures) = future_year_re.captures(text) {
            if let Some(date_match) = captures.get(1) {
                let date = date_match.as_str().to_string();
                info!("Found future death date: {}", date);
                return Some(date);
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

// Function to extract cause of death from text
fn extract_cause_of_death(text: &str) -> Option<String> {
    info!("Attempting to extract cause of death from text");
    
    // Common patterns for cause of death
    let patterns = [
        r"died (?:of|from|due to|after|following) ([^\.]+)",
        r"death (?:was caused by|was due to|from|by) ([^\.]+)",
        r"died .{0,30}? (?:of|from|due to|after|following) ([^\.]+)",
        r"passed away (?:from|due to|after|following) ([^\.]+)",
        r"succumbed to ([^\.]+)",
        r"lost (?:his|her|their) (?:battle|fight|struggle) with ([^\.]+)",
        r"died .{0,50}? complications (?:of|from) ([^\.]+)",
        r"cause of death was ([^\.]+)",
    ];
    
    let text_lower = text.to_lowercase();
    
    for pattern in &patterns {
        if let Ok(re) = Regex::new(pattern) {
            if let Some(captures) = re.captures(&text_lower) {
                if let Some(cause_match) = captures.get(1) {
                    let mut cause = cause_match.as_str().trim().to_string();
                    
                    // Clean up the cause of death
                    // Remove trailing periods, commas, etc.
                    while cause.ends_with('.') || cause.ends_with(',') || cause.ends_with(';') || cause.ends_with(':') {
                        cause.pop();
                    }
                    
                    // Capitalize first letter
                    if !cause.is_empty() {
                        let first_char = cause.chars().next().unwrap().to_uppercase().collect::<String>();
                        if cause.len() > 1 {
                            cause = first_char + &cause[1..];
                        } else {
                            cause = first_char;
                        }
                    }
                    
                    info!("Found cause of death: {}", cause);
                    return Some(cause);
                }
            }
        }
    }
    
    // If no match found with the patterns, try to find sentences containing death-related terms
    let death_terms = ["died", "death", "passed away", "deceased", "fatal", "killed"];
    
    // Split the text into sentences
    let sentences: Vec<&str> = text.split(|c| c == '.' || c == '!' || c == '?')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    
    for sentence in sentences {
        let sentence_lower = sentence.to_lowercase();
        for term in &death_terms {
            if sentence_lower.contains(term) {
                // Look for cause indicators
                let cause_indicators = ["from", "due to", "of", "after", "by", "with"];
                for indicator in &cause_indicators {
                    if sentence_lower.contains(indicator) {
                        if let Some(pos) = sentence_lower.find(indicator) {
                            let cause = sentence[pos + indicator.len()..].trim();
                            if !cause.is_empty() {
                                info!("Found potential cause of death in sentence: {}", cause);
                                return Some(cause.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    
    info!("No cause of death found");
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
