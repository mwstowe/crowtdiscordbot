use anyhow::Result;

// A simple function to test the celebrity status extraction directly
pub fn test_david_lynch() -> Result<()> {
    println!("Testing celebrity status extraction for David Lynch");
    
    // The example text from Wikipedia
    let text = "David Keith Lynch (January 20, 1946 – January 16, 2025) was an American filmmaker, visual artist, musician, and actor.";
    
    println!("Input text: {}", text);
    
    // Extract dates from parentheses
    let re = regex::Regex::new(r"^(.*?)\(([^)]+)\)(.*)$").unwrap();
    
    if let Some(captures) = re.captures(text) {
        let before = captures.get(1).map_or("", |m| m.as_str());
        let parentheses_content = captures.get(2).map_or("", |m| m.as_str());
        let after = captures.get(3).map_or("", |m| m.as_str());
        
        println!("Found parentheses content: {}", parentheses_content);
        println!("Before parentheses: {}", before);
        println!("After parentheses: {}", after);
        
        // Extract birth and death dates from parentheses
        let birth_date = extract_birth_date(parentheses_content);
        let death_date = extract_death_date(parentheses_content);
        
        println!("Extracted birth date: {:?}", birth_date);
        println!("Extracted death date: {:?}", death_date);
        
        // Create cleaned text without the parentheses
        let cleaned_text = format!("{}{}", before, after);
        println!("Cleaned text: {}", cleaned_text);
        
        // Build the response as the bot would
        let mut response = format!("**David Lynch**: {}", cleaned_text);
        
        if let Some(date) = death_date {
            response.push_str(&format!(". They died on {}.", date));
        } else {
            response.push_str(". They have died, but I couldn't determine the exact date.");
        }
        
        println!("\nFinal response: {}", response);
    } else {
        println!("No parentheses found in text");
    }
    
    Ok(())
}

fn extract_birth_date(text: &str) -> Option<String> {
    // If there's a dash, the birth date is likely before it
    if text.contains('–') || text.contains('-') {
        let parts: Vec<&str> = text.split(|c| c == '–' || c == '-').collect();
        if !parts.is_empty() {
            let potential_date = parts[0].trim();
            // Check if it looks like a date (contains a year)
            if regex::Regex::new(r"\d{4}").unwrap().is_match(potential_date) {
                println!("Found birth date before dash: {}", potential_date);
                return Some(potential_date.to_string());
            }
        }
    }
    None
}

fn extract_death_date(text: &str) -> Option<String> {
    // If there's a dash, the death date is likely after it
    if text.contains('–') || text.contains('-') {
        let parts: Vec<&str> = text.split(|c| c == '–' || c == '-').collect();
        if parts.len() > 1 {
            let potential_date = parts[1].trim();
            // Check if it looks like a date (contains a year)
            if regex::Regex::new(r"\d{4}").unwrap().is_match(potential_date) {
                println!("Found death date after dash: {}", potential_date);
                return Some(potential_date.to_string());
            }
        }
    }
    None
}
