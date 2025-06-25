use tracing::info;
use anyhow::Result;

// Helper function to check if a word is a common word that should be ignored in some contexts
pub fn is_common_word(word: &str) -> bool {
    const COMMON_WORDS: &[&str] = &[
        "the", "and", "that", "this", "with", "for", "was", "not", 
        "you", "have", "are", "they", "what", "from", "but", "its",
        "his", "her", "their", "your", "our", "who", "which", "when",
        "where", "why", "how", "all", "any", "some", "many", "much",
        "more", "most", "other", "such", "than", "then", "too", "very",
        "just", "now", "also", "into", "only", "over", "under", "same",
        "about", "after", "before", "between", "during", "through", "above",
        "below", "down", "off", "out", "since", "upon", "while", "within",
        "without", "across", "along", "among", "around", "behind", "beside",
        "beyond", "near", "toward", "against", "despite", "except", "like",
        "until", "because", "although", "unless", "whereas", "whether"
    ];
    
    COMMON_WORDS.contains(&word)
}

// Generate variations for "as X as Y" phrases
pub fn generate_as_phrase_variations(query: &str) -> Vec<String> {
    let mut variations = Vec::new();
    
    // For phrases like "as safe as they said"
    if query.contains(" as ") {
        let words: Vec<&str> = query.split_whitespace().collect();
        
        // Find all "as" positions
        let as_positions: Vec<usize> = words.iter()
            .enumerate()
            .filter(|(_, &word)| word.to_lowercase() == "as")
            .map(|(i, _)| i)
            .collect();
            
        // If we have at least two "as" words
        if as_positions.len() >= 2 {
            for i in 0..as_positions.len() - 1 {
                let pos1 = as_positions[i];
                let pos2 = as_positions[i + 1];
                
                // If they're part of an "as X as Y" pattern
                if pos2 > pos1 + 1 {
                    // Try just the phrase between the "as" words
                    let middle_phrase: Vec<&str> = words[(pos1 + 1)..pos2].to_vec();
                    variations.push(middle_phrase.join(" "));
                    
                    // Try the phrase with the second "as"
                    let extended_phrase: Vec<&str> = words[(pos1 + 1)..=pos2].to_vec();
                    variations.push(extended_phrase.join(" "));
                    
                    // Try the phrase after the second "as"
                    if pos2 < words.len() - 1 {
                        let after_phrase: Vec<&str> = words[(pos2 + 1)..].to_vec();
                        variations.push(after_phrase.join(" "));
                    }
                    
                    // Try the full "as X as Y" phrase
                    let full_phrase: Vec<&str> = words[pos1..].to_vec();
                    variations.push(full_phrase.join(" "));
                }
            }
        }
    }
    
    variations
}

// Generate variations for common speech patterns
pub fn generate_speech_pattern_variations(query: &str) -> Vec<String> {
    let mut variations = Vec::new();
    let words: Vec<&str> = query.split_whitespace().collect();
    
    // For phrases with "that" or "which" - try removing them
    if query.contains(" that ") || query.contains(" which ") {
        let filtered_words: Vec<&str> = words.iter()
            .filter(|&&word| word.to_lowercase() != "that" && word.to_lowercase() != "which")
            .copied()
            .collect();
        variations.push(filtered_words.join(" "));
    }
    
    // For phrases with "isn't" or "wasn't" - try the expanded form
    if query.contains("isn't") {
        variations.push(query.replace("isn't", "is not"));
    }
    if query.contains("wasn't") {
        variations.push(query.replace("wasn't", "was not"));
    }
    
    // For phrases with "they said" or "they say" - try removing them
    if query.contains(" they said") || query.contains(" they say") {
        let without_they_said = query
            .replace(" they said", "")
            .replace(" they say", "");
        variations.push(without_they_said);
    }
    
    // For phrases with brackets or parentheses - try without them
    if query.contains('[') || query.contains(']') || 
       query.contains('(') || query.contains(')') {
        let without_brackets = query
            .replace('[', "")
            .replace(']', "")
            .replace('(', "")
            .replace(')', "");
        variations.push(without_brackets);
        
        // Also try extracting just what's inside brackets
        if let Some(start) = query.find('[') {
            if let Some(end) = query.find(']') {
                if end > start {
                    let inside_brackets = &query[(start + 1)..end];
                    variations.push(inside_brackets.to_string());
                }
            }
        }
        
        if let Some(start) = query.find('(') {
            if let Some(end) = query.find(')') {
                if end > start {
                    let inside_parens = &query[(start + 1)..end];
                    variations.push(inside_parens.to_string());
                }
            }
        }
    }
    
    // For phrases with "you know" - try removing it
    if query.contains("you know") {
        variations.push(query.replace("you know", ""));
    }
    
    variations
}

// Generate fuzzy variations of the query
pub fn generate_fuzzy_variations(query: &str) -> Vec<String> {
    let mut variations = Vec::new();
    let words: Vec<&str> = query.split_whitespace().collect();
    
    // Skip very short queries
    if words.len() <= 1 {
        return variations;
    }
    
    // Try with only significant words (longer than 3 chars and not common)
    let significant_words: Vec<&str> = words.iter()
        .filter(|&&word| {
            let word_lower = word.to_lowercase();
            word_lower.len() > 3 && !is_common_word(&word_lower)
        })
        .copied()
        .collect();
        
    if !significant_words.is_empty() {
        variations.push(significant_words.join(" "));
    }
    
    // Try with different word orders for key phrases
    if words.len() >= 3 {
        // For each triplet of words, try different permutations
        for i in 0..words.len() - 2 {
            let w1 = words[i];
            let w2 = words[i + 1];
            let w3 = words[i + 2];
            
            // Original order: w1 w2 w3
            // Try: w1 w3 w2
            variations.push(format!("{} {} {}", w1, w3, w2));
            
            // Try: w2 w1 w3
            variations.push(format!("{} {} {}", w2, w1, w3));
            
            // Try: w3 w1 w2
            variations.push(format!("{} {} {}", w3, w1, w2));
        }
    }
    
    // For phrases with "not as X as Y" - try "not X as Y"
    if query.contains("not as") && query.contains(" as ") {
        let not_as_variation = query
            .replace("not as", "not")
            .replace(" as ", " ");
        variations.push(not_as_variation);
    }
    
    variations
}
