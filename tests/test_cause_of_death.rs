extern crate regex;
use regex::Regex;

// Mock the info! macro
macro_rules! info {
    ($($arg:tt)*) => {
        println!($($arg)*)
    };
}

// Copy of the extract_cause_of_death function
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
                    
                    // Skip if the cause contains phrases that indicate it's not actually a cause of death
                    let false_indicators = [
                        "until his death", "until her death", "until their death",
                        "before his death", "before her death", "before their death",
                        "prior to his death", "prior to her death", "prior to their death",
                        "at the time of", "at the age of"
                    ];
                    
                    let is_false_positive = false_indicators.iter().any(|&indicator| cause.to_lowercase().contains(indicator));
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
                            
                            // Skip if empty or too long (likely not a real cause)
                            if cause.is_empty() || cause.len() > 100 {
                                continue;
                            }
                            
                            // Skip if the cause contains phrases that indicate it's not actually a cause of death
                            let false_indicators = [
                                "until his death", "until her death", "until their death",
                                "before his death", "before her death", "before their death",
                                "prior to his death", "prior to her death", "prior to their death",
                                "at the time of", "at the age of"
                            ];
                            
                            let is_false_positive = false_indicators.iter().any(|&indicator| cause.to_lowercase().contains(indicator));
                            if is_false_positive {
                                info!("Skipping false positive cause: {}", cause);
                                continue;
                            }
                            
                            info!("Found potential cause of death in sentence: {}", cause);
                            return Some(cause.to_string());
                        }
                    }
                }
            }
        }
    }
    
    info!("No cause of death found");
    None
}

fn main() {
    // Test with Ernest Borgnine's text
    let text = "Borgnine earned his third Primetime Emmy Award nomination at age 92 for his work on the 2009 series finale of ER. He was also known as the original voice of Mermaid Man on SpongeBob SquarePants from 1999 until his death in 2012.";
    
    println!("Testing cause of death extraction with Ernest Borgnine text:");
    println!("{}", text);
    
    let cause = extract_cause_of_death(text);
    println!("\nExtracted cause of death: {:?}", cause);
    
    // Test with a text that should have a real cause of death
    let text2 = "Robin Williams died by suicide in 2014 after struggling with Lewy body dementia.";
    
    println!("\nTesting with text that should have a real cause:");
    println!("{}", text2);
    
    let cause2 = extract_cause_of_death(text2);
    println!("\nExtracted cause of death: {:?}", cause2);
}
