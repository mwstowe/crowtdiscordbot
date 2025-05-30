extern crate regex;
use regex::Regex;
use chrono::Datelike;

// Mock the info! macro
macro_rules! info {
    ($($arg:tt)*) => {
        println!($($arg)*)
    };
}

// Copy of the extract_year_from_parentheses function
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

// Copy of the extract_dates_from_parentheses function
fn extract_dates_from_parentheses(text: &str) -> (Option<String>, Option<String>, String) {
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
                
                info!("EXTRACTION SUCCESS - Birth: {}, Death: {}", dates[0], dates[1]);
                info!("Cleaned text: {}", cleaned_text);
                
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

fn main() {
    let text = "Edward Lodewijk Van Halen ( van HAY-lən, Dutch: [ˈɛtʋɑrt ˈloːdəʋɛik fɑn ˈɦaːlə(n)]; January 26, 1955 – October 6, 2020) was an American musician. He was the guitarist, keyboardist, backing vocalist and primary songwriter of the rock band Van Halen, which he founded with his brother Alex in 1972.";
    
    println!("Testing with text: {}", text);
    
    let (birth_date, death_date, cleaned_text) = extract_dates_from_parentheses(text);
    
    println!("\nFinal results:");
    println!("Birth date: {:?}", birth_date);
    println!("Death date: {:?}", death_date);
    println!("Cleaned text: {}", cleaned_text);
}
