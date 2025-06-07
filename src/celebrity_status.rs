use anyhow::Result;
use serenity::all::Http;
use serenity::model::channel::Message;
use tracing::{error, info};
use reqwest::Client;
use serde_json::Value;
use chrono::{NaiveDate, Datelike};
use regex::Regex;

pub async fn handle_aliveordead_command(http: &Http, msg: &Message, celebrity_name: &str) -> Result<()> {
    info!("Handling !alive command for celebrity: {}", celebrity_name);
    
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

// Function to check if the text is about a fictional character
fn is_fictional_character(text: &str, title: &str) -> bool {
    let text_lower = text.to_lowercase();
    let title_lower = title.to_lowercase();
    
    // Special case for Gary Gygax and other known real people who might be incorrectly flagged
    let known_real_people = [
        "gary gygax", "ernest gary gygax", "dave arneson", "j. r. r. tolkien", "tolkien",
        "stan lee", "jack kirby", "george lucas", "gene roddenberry", "isaac asimov",
        "stephen king", "george r. r. martin", "jrr tolkien", "j.r.r. tolkien"
    ];
    
    for person in &known_real_people {
        if title_lower.contains(person) {
            info!("Known real person detected: '{}', not a fictional character", person);
            return false;
        }
    }
    
    // Check for explicit "real person" indicators
    let real_person_indicators = [
        "was born", "was a", "is a", "american author", "british author", 
        "writer", "author", "creator", "inventor", "founder", "developer",
        "designer", "producer", "director", "businessman", "businesswoman",
        "politician", "president", "prime minister", "ceo", "executive",
        "scientist", "researcher", "professor", "teacher", "educator",
        "artist", "musician", "composer", "singer", "actor", "actress",
        "athlete", "player", "coach", "manager", "born in", "died in",
        "graduated from", "attended", "studied at", "worked at", "worked for"
    ];
    
    for indicator in &real_person_indicators {
        if text_lower.contains(indicator) {
            // If we find a real person indicator, check if there's also a fictional indicator
            // Only return false if we don't find any fictional indicators
            break;
        }
    }
    
    // Common indicators of fictional characters
    let fictional_indicators = [
        "fictional character", "fictional protagonist", "fictional antagonist",
        "fictional superhero", "fictional supervillain", "fictional detective",
        "main character", "title character", "protagonist of", "antagonist of",
        "character in the", "character from the", "appears in"
    ];
    
    // Check for these indicators
    for indicator in &fictional_indicators {
        if text_lower.contains(indicator) {
            info!("Fictional character indicator found: '{}'", indicator);
            return true;
        }
    }
    
    // "Created by" is ambiguous - it could refer to a fictional character or a creation by a real person
    // Only count it as fictional if it doesn't look like a real person description
    if text_lower.contains("created by") && !text_lower.contains(" born ") && !text_lower.contains(" died ") {
        // Additional check: if it contains biographical information, it's likely a real person
        if !text_lower.contains("graduated") && !text_lower.contains("education") && 
           !text_lower.contains("married") && !text_lower.contains("career") {
            info!("Fictional character indicator 'created by' found without biographical info");
            return true;
        }
    }
    
    // Check if the title contains common fictional character indicators
    let fictional_title_indicators = [
        "(character)", "(fictional character)", "(comics)", "(Marvel Comics)",
        "(DC Comics)", "(Disney)", "(film series)", "(film character)",
    ];
    
    for indicator in &fictional_title_indicators {
        if title_lower.contains(indicator) {
            info!("Fictional character indicator in title: '{}'", indicator);
            return true;
        }
    }
    
    false
}

// Function to find the actor associated with a fictional character
async fn find_actor_for_character(text: &str, character_name: &str, client: &Client) -> Result<Option<String>> {
    let text_lower = text.to_lowercase();
    
    // Look for common patterns that mention actors
    let actor_patterns = [
        r"portrayed by ([^\.]+)",
        r"played by ([^\.]+)",
        r"voiced by ([^\.]+)",
        r"role of [^\.]+? (?:is|was) ([^\.]+)",
        r"actor ([^\.]+) portrays",
        r"actress ([^\.]+) portrays",
        r"actor ([^\.]+) plays",
        r"actress ([^\.]+) plays",
    ];
    
    for pattern in &actor_patterns {
        if let Ok(re) = Regex::new(pattern) {
            if let Some(captures) = re.captures(&text_lower) {
                if let Some(actor_match) = captures.get(1) {
                    let actor_name = actor_match.as_str().trim();
                    
                    // Clean up the actor name (remove "in the film" etc.)
                    let actor_name = actor_name.split(" in ").next().unwrap_or(actor_name).trim();
                    let actor_name = actor_name.split(" on ").next().unwrap_or(actor_name).trim();
                    let actor_name = actor_name.split(" for ").next().unwrap_or(actor_name).trim();
                    
                    info!("Found potential actor: {}", actor_name);
                    
                    // Get information about this actor
                    if let Ok(Some(actor_info)) = search_actor(actor_name, client).await {
                        return Ok(Some(format!("The character is most famously portrayed by {}.", actor_info)));
                    }
                }
            }
        }
    }
    
    // If we couldn't find an actor in the text, try a direct search
    let search_query = format!("{} actor", character_name);
    info!("Trying direct search for actor: {}", search_query);
    
    // Search for the actor
    let search_url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&list=search&srsearch={}&format=json&srlimit=1",
        urlencoding::encode(&search_query)
    );
    
    let search_response = client.get(&search_url).send().await?;
    let search_json: Value = search_response.json().await?;
    
    // Extract the page title from search results
    if let Some(title) = search_json
        .get("query")
        .and_then(|q| q.get("search"))
        .and_then(|s| s.get(0))
        .and_then(|r| r.get("title"))
        .and_then(|t| t.as_str()) 
    {
        // Check if this looks like an actor's name (not the character again)
        if !title.to_lowercase().contains(&character_name.to_lowercase()) {
            info!("Found potential actor via search: {}", title);
            
            // Get information about this actor
            if let Ok(Some(actor_info)) = search_actor(title, client).await {
                return Ok(Some(format!("The character is most famously portrayed by {}.", actor_info)));
            }
        }
    }
    
    Ok(None)
}

// Function to search for information about an actor
async fn search_actor(name: &str, client: &Client) -> Result<Option<String>> {
    // Search for the actor's page
    let search_url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&list=search&srsearch={}&format=json&srlimit=1",
        urlencoding::encode(name)
    );
    
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
                info!("No search results found for actor: {}", name);
                return Ok(None);
            }
        };
    
    // Now get the page content
    let page_url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&prop=extracts&exintro&explaintext&redirects=1&titles={}&format=json",
        urlencoding::encode(page_title)
    );
    
    let page_response = client.get(&page_url).send().await?;
    let page_json: Value = page_response.json().await?;
    
    // Extract the page ID
    let pages = match page_json.get("query").and_then(|q| q.get("pages")) {
        Some(p) => p,
        None => {
            info!("No page data found for actor: {}", page_title);
            return Ok(None);
        }
    };
    
    // Get the first page (there should only be one)
    let page_id = match pages.as_object().and_then(|o| o.keys().next()) {
        Some(id) => id,
        None => {
            info!("No page ID found for actor: {}", page_title);
            return Ok(None);
        }
    };
    
    // Extract the extract (page content)
    let raw_extract = match pages.get(page_id).and_then(|p| p.get("extract")).and_then(|e| e.as_str()) {
        Some(e) => e,
        None => {
            info!("No extract found for actor page: {}", page_title);
            return Ok(None);
        }
    };
    
    // Check if this is a person
    let is_person = raw_extract.contains(" born ") || raw_extract.contains(" died ") || 
                    Regex::new(r"\([^)]*\d{4}[^)]*\)").ok().map_or(false, |re| re.is_match(raw_extract));
    
    if !is_person {
        info!("Actor page doesn't appear to be about a person: {}", page_title);
        return Ok(None);
    }
    
    // Determine if the actor is alive or dead
    let (_, death_date, _) = extract_dates_from_parentheses(raw_extract);
    let contains_was = raw_extract.contains(" was ");
    let contains_is = raw_extract.contains(" is ");
    let is_dead = death_date.is_some() || 
                 (contains_was && !contains_is && raw_extract.contains("(") && raw_extract.contains(")"));
    
    // Create a brief description of the actor
    let mut actor_info = format!("**{}**", page_title);
    
    if is_dead {
        if let Some(date) = death_date {
            actor_info.push_str(&format!(", who died on {}", date));
        } else {
            actor_info.push_str(", who has passed away");
        }
    } else {
        actor_info.push_str(", who is still alive");
    }
    
    Ok(Some(actor_info))
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
    
    // Log only the first 100 characters of the extract for debugging
    if raw_extract.len() > 100 {
        info!("Extract preview (first 100 chars): {}...", &raw_extract[..100]);
    } else {
        info!("Extract preview: {}", raw_extract);
    }
    
    // Check if this is a fictional character
    let is_fictional = is_fictional_character(raw_extract, page_title);
    
    if is_fictional {
        info!("Detected fictional character: {}", page_title);
        
        // Try to find the actor associated with this character
        if let Some(actor_info) = find_actor_for_character(raw_extract, page_title, &client).await? {
            return Ok(Some(format!("**{}** is a fictional character. {}", page_title, actor_info)));
        } else {
            return Ok(Some(format!("**{}** is a fictional character, not a real person.", page_title)));
        }
    }
    
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
    
    info!("Extracted dates - Birth: {:?}, Death: {:?}", birth_date, death_date);
    
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
    
    // A person is considered dead if:
    // 1. We have a death date from parentheses, OR
    // 2. The text uses past tense ("was") and not present tense ("is") and has parentheses (likely birth-death dates)
    // 3. BUT we need to handle special cases where "was" is used in past events for living people
    let is_dead = if let Some(death_year) = death_date.as_ref().and_then(|d| {
        // Extract year from death date
        let year_regex = Regex::new(r"\b(\d{4})\b").ok()?;
        year_regex.captures(d)?.get(1).map(|m| m.as_str().parse::<i32>().ok())
    }).flatten() {
        // Check if the death year is in the future (which would be an error)
        let current_year = chrono::Local::now().year();
        if death_year > current_year {
            info!("Found death year {} which is in the future, ignoring", death_year);
            false
        } else {
            info!("Found valid death year: {}", death_year);
            true
        }
    } else if contains_was && !contains_is && raw_extract.contains("(") && raw_extract.contains(")") {
        // Check for special cases where "was" might be used for living people
        // If the text contains phrases like "is an American" or "is best known", the person is likely alive
        let alive_indicators = [
            " is an ", " is a ", " is best known", " is known", " is currently ",
            " has been ", " has toured", " has played", " has recorded", " has released",
            " continues to ", " lives in ", " resides in ", " is the ", " is also ",
            " is active", " is married", " is working", " is touring", " is recording",
            " is performing", " is based in", " is a member of", " is the founder",
            " is the author", " is the creator", " is the owner", " is the director",
            " is the producer", " is the host", " is the presenter", " is the leader",
            " is the ceo", " is the president", " is the chairman", " is the founder",
            " is the head", " is the chief", " is the manager", " is the coach",
            " is the instructor", " is the teacher", " is the professor",
            " since ", " as of ", " to date ", " to present", " present day",
            " currently ", " nowadays ", " these days ", " recently ",
            " today ", " now ", " still ", " ongoing ", " active ",
        ];
        
        let text_lower = raw_extract.to_lowercase();
        let has_alive_indicator = alive_indicators.iter().any(|&indicator| text_lower.contains(indicator));
        
        // If we find any alive indicators, the person is likely alive despite the "was" usage
        !has_alive_indicator
    } else {
        false
    };
    
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
            if let Some(ref cause) = cause_of_death {
                death_info.push_str(&format!(" Cause of death: {}.", cause));
            }
            
            // Calculate age at death if birth date is available
            if let Some(birth_date_str) = &birth_date {
                if let Some(birth) = parse_date(birth_date_str) {
                    if let Some(death) = parse_date(&date) {
                        let age = calculate_age(birth, death);
                        death_info = format!(". {} died on {} at the age of {}.", 
                            subject_pronoun.to_string().to_uppercase().chars().next().unwrap().to_string() + &subject_pronoun[1..], 
                            date, age);
                        
                        // Re-add cause of death if available
                        if let Some(ref cause) = cause_of_death {
                            death_info.push_str(&format!(" Cause of death: {}.", cause));
                        }
                    }
                }
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
            if let Some(ref cause) = cause_of_death {
                death_info.push_str(&format!(" Cause of death: {}.", cause));
            }
            
            // Calculate age at death if birth date is available
            if let Some(birth_date_str) = &birth_date {
                if let Some(birth) = parse_date(birth_date_str) {
                    if let Some(death) = parse_date(&date) {
                        let age = calculate_age(birth, death);
                        death_info = format!(". {} died on {} at the age of {}.", 
                            subject_pronoun.to_string().to_uppercase().chars().next().unwrap().to_string() + &subject_pronoun[1..], 
                            date, age);
                        
                        // Re-add cause of death if available
                        if let Some(ref cause) = cause_of_death {
                            death_info.push_str(&format!(" Cause of death: {}.", cause));
                        }
                    }
                }
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
    // Check for sequential parentheses pattern like "(born Kenneth Donald Rogers) (August 21, 1938 – March 20, 2020)"
    // First, find all parenthetical sections in the text
    let mut parentheses_sections = Vec::new();
    let mut start_idx = 0;
    
    while let Some(open_idx) = text[start_idx..].find('(') {
        let open_pos = start_idx + open_idx;
        let mut depth = 1;
        let mut close_pos = None;
        
        for (i, c) in text[open_pos + 1..].char_indices() {
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                depth -= 1;
                if depth == 0 {
                    close_pos = Some(open_pos + 1 + i);
                    break;
                }
            }
        }
        
        if let Some(close_idx) = close_pos {
            parentheses_sections.push((open_pos, close_idx, &text[open_pos..=close_idx]));
            start_idx = close_idx + 1;
        } else {
            break; // No matching closing parenthesis found
        }
    }
    
    // Look for a section that contains birth-death dates (has a year range with dash)
    for (_, _, section) in &parentheses_sections {
        // Remove the outer parentheses
        let content = &section[1..section.len()-1];
        
        // Check if this contains a date range with a dash or en-dash
        if content.contains('–') || content.contains('-') {
            let separator = if content.contains('–') { '–' } else { '-' };
            let parts: Vec<&str> = content.split(separator).collect();
            
            if parts.len() == 2 {
                let birth_part = parts[0].trim();
                let death_part = parts[1].trim();
                
                // Check if both parts look like dates (contain years)
                let year_regex = Regex::new(r"\d{4}").unwrap();
                if year_regex.is_match(birth_part) && year_regex.is_match(death_part) {
                    // Check if this is likely a birth-death range rather than a band membership or other date range
                    // Birth-death ranges typically:
                    // 1. Are near the beginning of the text (in the first 100 characters)
                    // 2. Are not preceded by words like "band", "group", "member", "career", etc.
                    // 3. Often have month/day information or are just years (1946-2021)
                    
                    // Calculate position in text
                    let section_pos = section.as_ptr() as usize - text.as_ptr() as usize;
                    let is_near_beginning = section_pos < 100;
                    
                    // Get preceding text to check for band/career indicators
                    let start_pos = if section_pos > 30 { section_pos - 30 } else { 0 };
                    let preceding_text = &text[start_pos..section_pos];
                    let preceding_text_lower = preceding_text.to_lowercase();
                    
                    // Define band/career indicators
                    let band_indicators = ["band", "group", "member", "career", "tour", "album", 
                                          "record", "release", "project", "formed", "founded", 
                                          "joined", "left", "played with", "performed with",
                                          "worked with", "collaborated", "session", "studio"];
                    
                    // Check if any indicators are present
                    let has_band_indicator = band_indicators.iter()
                        .any(|&indicator| preceding_text_lower.contains(indicator));
                    
                    // If it's near the beginning and doesn't have band indicators, it's likely birth-death
                    let is_likely_birth_death = is_near_beginning && !has_band_indicator;
                    
                    if is_likely_birth_death {
                        // Create cleaned text without this parenthetical section
                        let mut cleaned_text = text.to_string();
                        for (start, end, sect) in &parentheses_sections {
                            if sect == section {
                                cleaned_text = format!("{}{}", 
                                    &text[0..*start], 
                                    &text[*end+1..]);
                                break;
                            }
                        }
                        cleaned_text = cleaned_text.replace("  ", " ").trim().to_string();
                        
                        info!("Found birth-death dates in parentheses: {} - {}", birth_part, death_part);
                        return (Some(birth_part.to_string()), Some(death_part.to_string()), cleaned_text);
                    } else {
                        info!("Found date range but it appears to be for a band/career: {} - {}", birth_part, death_part);
                    }
                }
            }
        }
    }
    
    // If we didn't find a date range in any parenthetical section, try the original approach
    // Find the first opening parenthesis
    if let Some(open_paren_pos) = text.find('(') {
        // Find the matching closing parenthesis
        let mut depth = 1;
        let mut close_paren_pos = None;
        
        for (i, c) in text[open_paren_pos + 1..].char_indices() {
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                depth -= 1;
                if depth == 0 {
                    close_paren_pos = Some(open_paren_pos + 1 + i);
                    break;
                }
            }
        }
        
        if let Some(close_pos) = close_paren_pos {
            // Extract the entire parenthetical section
            let paren_section = &text[open_paren_pos..=close_pos];
            
            // Look for dates within this section
            let date_regex = Regex::new(r"(\w+ \d{1,2}, \d{4})").unwrap();
            let mut dates = Vec::new();
            
            for date_match in date_regex.find_iter(paren_section) {
                dates.push(date_match.as_str().to_string());
            }
            
            if dates.len() >= 2 {
                // Create cleaned text without the parentheses
                let mut cleaned_text = format!("{}{}", 
                    &text[0..open_paren_pos], 
                    &text[close_pos + 1..]);
                cleaned_text = cleaned_text.replace("  ", " ").trim().to_string();
                
                return (Some(dates[0].clone()), Some(dates[1].clone()), cleaned_text);
            }
        }
    }
    
    // If the direct approach didn't work, fall back to the regex approach
    let re = Regex::new(r"^(.*?)\(([^)]+)\)(.*)$").unwrap();
    
    if let Some(captures) = re.captures(text) {
        let before = captures.get(1).map_or("", |m| m.as_str());
        let parentheses_content = captures.get(2).map_or("", |m| m.as_str());
        let after = captures.get(3).map_or("", |m| m.as_str());
        
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
                    return (Some(birth_part.to_string()), Some(death_part.to_string()), cleaned_text);
                }
            }
        }
        
        // If direct extraction didn't work, try the more complex patterns
        let birth_date = extract_year_from_parentheses(parentheses_content, "born");
        let death_date = extract_year_from_parentheses(parentheses_content, "died");
        
        return (birth_date, death_date, cleaned_text);
    }
    
    // If no parentheses found, return the original text
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
            return date;
        }
        
        // If there's a dash, the birth date is likely before it
        if text.contains('–') || text.contains('-') {
            let parts: Vec<&str> = text.split(|c| c == '–' || c == '-').collect();
            if !parts.is_empty() {
                let potential_date = parts[0].trim();
                // Check if it looks like a date (contains a year)
                if Regex::new(r"\d{4}").unwrap().is_match(potential_date) {
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
            return date;
        }
        
        // If there's a dash, the death date is likely after it
        if text.contains('–') || text.contains('-') {
            let parts: Vec<&str> = text.split(|c| c == '–' || c == '-').collect();
            if parts.len() > 1 {
                let potential_date = parts[1].trim();
                // Check if it looks like a date (contains a year)
                if Regex::new(r"\d{4}").unwrap().is_match(potential_date) {
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
// Function to extract cause of death from text
fn extract_cause_of_death(text: &str) -> Option<String> {
    info!("Attempting to extract cause of death from text");
    
    // First, split the text into sentences for better context
    let sentences: Vec<&str> = text.split(|c| c == '.' || c == '!' || c == '?')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    
    // Look for sentences that mention death and might contain cause information
    let death_sentences: Vec<&str> = sentences.iter()
        .filter(|s| {
            let s_lower = s.to_lowercase();
            s_lower.contains("died") || s_lower.contains("death") || 
            s_lower.contains("passed away") || s_lower.contains("succumbed") ||
            s_lower.contains("lost his battle") || s_lower.contains("lost her battle")
        })
        .copied()
        .collect();
    
    if death_sentences.is_empty() {
        info!("No sentences mentioning death found");
        return None;
    }
    
    // Common patterns for cause of death - expanded with more variations
    let patterns = [
        // Direct cause patterns
        r"died (?:of|from|due to|after|following) ([^\.;:,]+)",
        r"death (?:was caused by|was due to|from|by) ([^\.;:,]+)",
        r"died .{0,30}? (?:of|from|due to|after|following) ([^\.;:,]+)",
        r"passed away (?:from|due to|after|following) ([^\.;:,]+)",
        r"succumbed to ([^\.;:,]+)",
        r"lost (?:his|her|their) (?:battle|fight|struggle) with ([^\.;:,]+)",
        r"died .{0,50}? complications (?:of|from) ([^\.;:,]+)",
        r"cause of death was ([^\.;:,]+)",
        r"death was attributed to ([^\.;:,]+)",
        r"died as a result of ([^\.;:,]+)",
        r"died because of ([^\.;:,]+)",
        r"death resulted from ([^\.;:,]+)",
    ];
    
    // First try with death-related sentences for better context
    for sentence in &death_sentences {
        for pattern in &patterns {
            if let Ok(re) = Regex::new(pattern) {
                if let Some(captures) = re.captures(sentence) {
                    if let Some(cause_match) = captures.get(1) {
                        let mut cause = cause_match.as_str().trim().to_string();
                        
                        // Skip if the cause is too long (likely not a real cause)
                        if cause.len() > 50 {
                            info!("Skipping cause that's too long: {}", cause);
                            continue;
                        }
                        
                        // Skip if the cause contains phrases that indicate it's not actually a cause of death
                        let false_indicators = [
                            "until his death", "until her death", "until their death",
                            "before his death", "before her death", "before their death",
                            "prior to his death", "prior to her death", "prior to their death",
                            "at the time of", "at the age of", "career", "professional", 
                            "embarking", "released", "musician", "album", "single"
                        ];
                        
                        let is_false_positive = false_indicators.iter().any(|&indicator| 
                            cause.to_lowercase().contains(indicator));
                        
                        if is_false_positive {
                            info!("Skipping false positive cause: {}", cause);
                            continue;
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
                        
                        info!("Found cause of death in death-related sentence: {}", cause);
                        return Some(cause);
                    }
                }
            }
        }
    }
    
    // If we still haven't found a cause, try to find specific diseases or conditions
    // that are commonly causes of death
    let common_causes = [
        "cancer", "heart attack", "stroke", "liver failure", "kidney failure",
        "respiratory failure", "pneumonia", "COVID-19", "coronavirus", "suicide",
        "accident", "complications", "heart failure", "cardiac arrest", "heart disease",
        "lung cancer", "brain cancer", "leukemia", "AIDS", "HIV", "overdose",
        "drug overdose", "alcohol", "cirrhosis", "alzheimer", "parkinson"
    ];
    
    for sentence in &death_sentences {
        let sentence_lower = sentence.to_lowercase();
        for cause in &common_causes {
            if sentence_lower.contains(cause) {
                // Get the context around the cause
                if let Some(pos) = sentence_lower.find(cause) {
                    // Get a window of text around the cause
                    let start = if pos > 10 { pos - 10 } else { 0 };
                    let end = (pos + cause.len() + 20).min(sentence.len());
                    let context = &sentence[start..end];
                    
                    // Extract just the cause and nearby words
                    let cause_with_context = extract_cause_with_context(context, cause);
                    
                    info!("Found common cause '{}' in death sentence", cause_with_context);
                    return Some(cause_with_context);
                }
            }
        }
    }
    
    info!("No cause of death found");
    None
}

// Helper function to extract a cause with some context
fn extract_cause_with_context(text: &str, cause: &str) -> String {
    let text_lower = text.to_lowercase();
    if let Some(pos) = text_lower.find(cause) {
        // Find the start of the phrase (after prepositions like "from", "of", etc.)
        let prepositions = ["from ", "of ", "with ", "due to ", "by ", "to "];
        let mut start = 0;
        for prep in &prepositions {
            if let Some(prep_pos) = text_lower[..pos].rfind(prep) {
                start = prep_pos + prep.len();
                break;
            }
        }
        
        // Find the end of the phrase (before punctuation or conjunctions)
        let mut end = text.len();
        let end_markers = [",", ";", ".", "and ", "but ", "which ", "when ", "while "];
        for marker in &end_markers {
            if let Some(marker_pos) = text_lower[pos..].find(marker) {
                end = pos + marker_pos;
                break;
            }
        }
        
        // Extract and clean up the phrase
        let mut result = text[start..end].trim().to_string();
        
        // Capitalize first letter
        if !result.is_empty() {
            let first_char = result.chars().next().unwrap().to_uppercase().collect::<String>();
            if result.len() > 1 {
                result = first_char + &result[1..];
            } else {
                result = first_char;
            }
        }
        
        return result;
    }
    
    // If we couldn't extract context, just return the cause capitalized
    cause.chars().next().unwrap().to_uppercase().collect::<String>() + &cause[1..]
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
