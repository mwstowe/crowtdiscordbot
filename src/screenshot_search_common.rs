use tracing::info;

// Common search utilities for screenshot search modules

// Normalize a search term to handle common variations
pub fn normalize_search_term(term: &str) -> String {
    let mut normalized = term.to_lowercase();
    
    // Handle common abbreviations and variations
    let variations = [
        ("mr.", "mister"),
        ("mrs.", "missus"),
        ("dr.", "doctor"),
        ("st.", "saint"),
        ("prof.", "professor"),
        ("lt.", "lieutenant"),
        ("gen.", "general"),
        ("capt.", "captain"),
        ("sgt.", "sergeant"),
        ("col.", "colonel"),
        ("rev.", "reverend"),
        ("hon.", "honorable"),
        ("gov.", "governor"),
        ("sen.", "senator"),
        ("rep.", "representative"),
        ("pres.", "president"),
        ("supt.", "superintendent"),
        ("dept.", "department"),
        ("corp.", "corporation"),
        ("inc.", "incorporated"),
        ("co.", "company"),
        ("jr.", "junior"),
        ("sr.", "senior"),
        ("vs.", "versus"),
        ("etc.", "etcetera"),
        ("i.e.", "that is"),
        ("e.g.", "for example"),
        ("a.m.", "morning"),
        ("p.m.", "evening"),
        ("approx.", "approximately"),
        ("est.", "established"),
        ("misc.", "miscellaneous"),
        ("no.", "number"),
        ("tel.", "telephone"),
        ("temp.", "temperature"),
        ("vol.", "volume"),
        ("doesn't", "does not"),
        ("don't", "do not"),
        ("won't", "will not"),
        ("can't", "cannot"),
        ("isn't", "is not"),
        ("aren't", "are not"),
        ("wasn't", "was not"),
        ("weren't", "were not"),
        ("haven't", "have not"),
        ("hasn't", "has not"),
        ("hadn't", "had not"),
        ("couldn't", "could not"),
        ("shouldn't", "should not"),
        ("wouldn't", "would not"),
        ("mustn't", "must not"),
        ("mightn't", "might not"),
        ("needn't", "need not"),
        ("shan't", "shall not"),
        ("i'm", "i am"),
        ("you're", "you are"),
        ("he's", "he is"),
        ("she's", "she is"),
        ("it's", "it is"),
        ("we're", "we are"),
        ("they're", "they are"),
        ("i've", "i have"),
        ("you've", "you have"),
        ("we've", "we have"),
        ("they've", "they have"),
        ("i'd", "i would"),
        ("you'd", "you would"),
        ("he'd", "he would"),
        ("she'd", "she would"),
        ("we'd", "we would"),
        ("they'd", "they would"),
        ("i'll", "i will"),
        ("you'll", "you will"),
        ("he'll", "he will"),
        ("she'll", "she will"),
        ("we'll", "we will"),
        ("they'll", "they will"),
    ];
    
    for (abbrev, full) in variations.iter() {
        // Replace the abbreviation with the full form
        normalized = normalized.replace(abbrev, full);
        
        // Also try the reverse for cases where the user searches for the full form
        normalized = normalized.replace(full, abbrev);
    }
    
    // Handle plural forms (basic English rules)
    let words: Vec<&str> = normalized.split_whitespace().collect();
    let mut normalized_words = Vec::new();
    
    for word in words {
        normalized_words.push(word.to_string());
        
        // Add singular form if word ends with 's'
        if word.ends_with('s') && word.len() > 1 {
            normalized_words.push(word[0..word.len()-1].to_string());
        }
        
        // Add plural form by adding 's'
        if !word.ends_with('s') {
            normalized_words.push(format!("{}s", word));
        }
        
        // Handle 'y' to 'ies' plurals
        if word.ends_with('y') && word.len() > 1 {
            normalized_words.push(format!("{}ies", &word[0..word.len()-1]));
        }
        
        // Handle 'ies' to 'y' singulars
        if word.ends_with("ies") && word.len() > 3 {
            normalized_words.push(format!("{}y", &word[0..word.len()-3]));
        }
        
        // Handle 'es' plurals
        if (word.ends_with("sh") || word.ends_with("ch") || 
            word.ends_with('x') || word.ends_with('z') || 
            word.ends_with('s')) && !word.ends_with("es") {
            normalized_words.push(format!("{}es", word));
        }
        
        // Handle 'es' to singular
        if word.ends_with("es") && word.len() > 2 {
            normalized_words.push(word[0..word.len()-2].to_string());
        }
        
        // Handle 'ves' to 'f' singulars
        if word.ends_with("ves") && word.len() > 3 {
            normalized_words.push(format!("{}f", &word[0..word.len()-3]));
            normalized_words.push(format!("{}fe", &word[0..word.len()-3]));
        }
        
        // Handle 'f' or 'fe' to 'ves' plurals
        if (word.ends_with('f') || word.ends_with("fe")) && word.len() > 1 {
            let stem = if word.ends_with('f') {
                &word[0..word.len()-1]
            } else {
                &word[0..word.len()-2]
            };
            normalized_words.push(format!("{}ves", stem));
        }
    }
    
    normalized_words.join(" ")
}

// Check if a search term is contained in a text, with variations
pub fn search_term_in_text(search_term: &str, text: &str) -> bool {
    let search_lower = search_term.to_lowercase();
    let text_lower = text.to_lowercase();
    
    // Direct match
    if text_lower.contains(&search_lower) {
        return true;
    }
    
    // Try with normalized variations
    let normalized_search = normalize_search_term(&search_lower);
    if text_lower.contains(&normalized_search) {
        return true;
    }
    
    // Try with word-by-word matching
    let search_words: Vec<&str> = search_lower.split_whitespace().collect();
    if search_words.iter().all(|&word| text_lower.contains(word)) {
        return true;
    }
    
    // Try with normalized word-by-word matching
    let normalized_words: Vec<&str> = normalized_search.split_whitespace().collect();
    normalized_words.iter().all(|&word| text_lower.contains(word))
}

// Calculate relevance score for a search result
pub fn calculate_result_relevance(
    caption: &str, 
    episode_title: &str, 
    query: &str,
    query_words: &[&str]
) -> f32 {
    if query_words.is_empty() {
        return 1.0; // Empty query matches everything
    }
    
    let caption_lower = caption.to_lowercase();
    let episode_title_lower = episode_title.to_lowercase();
    let query_lower = query.to_lowercase();
    
    // Count how many query words appear in the caption and title
    let mut caption_matches = 0;
    let mut title_matches = 0;
    let mut consecutive_word_bonus = 0.0;
    
    // Check for consecutive words in caption (much higher relevance)
    if query_words.len() > 1 {
        let full_query = query_words.join(" ");
        if caption_lower.contains(&full_query) {
            consecutive_word_bonus = 2.0; // Big bonus for consecutive words
        }
    }
    
    // Check for normalized variations
    let normalized_query = normalize_search_term(&query_lower);
    if caption_lower.contains(&normalized_query) {
        consecutive_word_bonus = 2.0; // Same big bonus for normalized match
    }
    
    // Count individual word matches
    for &word in query_words {
        if caption_lower.contains(word) {
            caption_matches += 1;
        }
        if episode_title_lower.contains(word) {
            title_matches += 1;
        }
        
        // Try normalized variations of each word
        let normalized_word = normalize_search_term(word);
        if caption_lower.contains(&normalized_word) && !caption_lower.contains(word) {
            caption_matches += 1;
        }
        if episode_title_lower.contains(&normalized_word) && !episode_title_lower.contains(word) {
            title_matches += 1;
        }
    }
    
    // Calculate match percentages
    let caption_match_percentage = caption_matches as f32 / query_words.len() as f32;
    let title_match_percentage = title_matches as f32 / query_words.len() as f32;
    
    // Calculate final score with weights
    let score = (caption_match_percentage * 0.7) + (title_match_percentage * 0.3) + consecutive_word_bonus;
    
    score
}

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
