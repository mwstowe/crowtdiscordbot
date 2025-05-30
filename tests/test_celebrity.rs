extern crate regex;
use regex::Regex;

fn main() {
    let text = "Edward Lodewijk Van Halen ( van HAY-lən, Dutch: [ˈɛtʋɑrt ˈloːdəʋɛik fɑn ˈɦaːlə(n)]; January 26, 1955 – October 6, 2020) was an American musician. He was the guitarist, keyboardist, backing vocalist and primary songwriter of the rock band Van Halen, which he founded with his brother Alex in 1972.";
    
    println!("Testing with text: {}", text);
    
    // Test our new regex pattern
    let birth_death_regex = Regex::new(r"\(\s*[^)]*?(\w+\s+\d{1,2},?\s+\d{4})\s*(?:–|-)\s*(\w+\s+\d{1,2},?\s+\d{4})[^)]*\)").unwrap();
    
    if let Some(captures) = birth_death_regex.captures(text) {
        if let (Some(birth_match), Some(death_match)) = (captures.get(1), captures.get(2)) {
            let birth_date = birth_match.as_str().trim();
            let death_date = death_match.as_str().trim();
            
            println!("REGEX EXTRACTION SUCCESS");
            println!("Birth: {}", birth_date);
            println!("Death: {}", death_date);
            
            // Remove the entire parenthetical section
            let start_idx = captures.get(0).unwrap().start();
            let end_idx = captures.get(0).unwrap().end();
            
            // Create cleaned text without the parentheses
            let mut cleaned_text = format!("{}{}", &text[0..start_idx], &text[end_idx..]);
            cleaned_text = cleaned_text.replace("  ", " ").trim().to_string();
            
            println!("Cleaned text: {}", cleaned_text);
        } else {
            println!("Failed to extract birth and death dates");
        }
    } else {
        println!("No match found with the regex pattern");
        
        // Let's try a more specific pattern for this case
        let specific_regex = Regex::new(r"\([^)]*January 26, 1955 – October 6, 2020[^)]*\)").unwrap();
        if let Some(match_result) = specific_regex.find(text) {
            println!("Found with specific regex: {}", &text[match_result.start()..match_result.end()]);
        } else {
            println!("No match found with specific regex either");
        }
    }
}
