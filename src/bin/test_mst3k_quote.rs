extern crate regex;
use regex::Regex;
use rand::seq::SliceRandom;

fn main() {
    // Test quotes in the format "<Speaker> Line <Speaker> Line"
    let test_quotes = [
        "<Mike> Shut up <Tom> Noooo",
        "<Crow> Watch out for snakes! <Joel> Where? <Crow> Over there!",
        "<Tom> It's the amazing Rando! <Mike> Who? <Tom> The amazing Rando!",
        "<Joel> Normal view... <Tom> Normal view... <Crow> NORMAL VIEW!",
        "<Mike> Rowsdower? <Crow> Rowsdower.",
        "<Single line without brackets>",
        "Quote without any brackets at all",
    ];
    
    println!("Testing MST3K quote extraction:");
    
    for quote in &test_quotes {
        println!("\nOriginal quote: {}", quote);
        
        // Extract individual lines from the quote
        let re = Regex::new(r"<([^>]+)>\s*([^<]+)").unwrap();
        
        // Find all speaker-line pairs
        let mut lines = Vec::new();
        for cap in re.captures_iter(quote) {
            if let (Some(speaker), Some(line_match)) = (cap.get(1), cap.get(2)) {
                let speaker_name = speaker.as_str();
                let line = line_match.as_str().trim();
                println!("  Found: Speaker '{}' says '{}'", speaker_name, line);
                if !line.is_empty() {
                    lines.push(line.to_string());
                }
            }
        }
        
        // If we found any lines, pick one randomly
        if !lines.is_empty() {
            let selected_line = lines.choose(&mut rand::thread_rng()).unwrap();
            println!("  Selected line: {}", selected_line);
        } else {
            println!("  No lines extracted, would use whole quote as fallback");
        }
    }
}
