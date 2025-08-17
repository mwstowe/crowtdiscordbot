// Common text formatting utilities for caption formatting

// Format a caption to proper sentence case and separate different speakers
pub fn format_caption(caption: &str, proper_nouns: &[&str]) -> String {
    // Split by newlines to get potential different speakers
    let lines: Vec<&str> = caption
        .split('\n')
        .filter(|line| !line.trim().is_empty())
        .collect();

    // Process each line
    let mut formatted_lines: Vec<String> = Vec::new();
    let mut current_speaker_lines: Vec<String> = Vec::new();

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Check if this is likely a new speaker (empty line before or all caps line)
        let is_new_speaker = current_speaker_lines.is_empty()
            || trimmed == trimmed.to_uppercase() && trimmed.chars().any(|c| c.is_alphabetic());

        if is_new_speaker && !current_speaker_lines.is_empty() {
            // Join previous speaker's lines and add to formatted lines
            formatted_lines.push(format!("\"{}\"", current_speaker_lines.join(" ")));
            current_speaker_lines.clear();
        }

        // Format the line with proper capitalization
        let formatted_line = format_text_with_proper_capitalization(trimmed, proper_nouns);

        // Add this line to current speaker
        current_speaker_lines.push(formatted_line);
    }

    // Add the last speaker's lines
    if !current_speaker_lines.is_empty() {
        formatted_lines.push(format!("\"{}\"", current_speaker_lines.join(" ")));
    }

    // Join all formatted parts
    formatted_lines.join(" ")
}

// Format text with proper capitalization for sentences and proper nouns
pub fn format_text_with_proper_capitalization(text: &str, proper_nouns: &[&str]) -> String {
    // Check if the text is all caps (or mostly all caps)
    let is_all_caps = is_mostly_uppercase(text);

    // Words that should always be lowercase (except at start of sentence)
    const LOWERCASE_WORDS: &[&str] = &[
        "a", "an", "the", "and", "but", "or", "for", "nor", "on", "at", "to", "from", "by", "with",
        "in", "of", "as", "is", "am", "are", "was", "were", "be", "been", "being",
    ];

    // Split the text into sentences
    let sentences: Vec<&str> = text
        .split(|c| c == '.' || c == '!' || c == '?')
        .filter(|s| !s.trim().is_empty())
        .collect();

    let mut formatted_sentences = Vec::new();

    for sentence in sentences {
        // Trim any leading whitespace
        let sentence = sentence.trim_start();
        if sentence.is_empty() {
            continue;
        }

        // Convert to lowercase if it's all caps
        let sentence_to_process = if is_all_caps {
            sentence.to_lowercase()
        } else {
            sentence.to_string()
        };

        // Split the sentence into words
        let words: Vec<&str> = sentence_to_process.split_whitespace().collect();
        if words.is_empty() {
            continue;
        }

        let mut formatted_words = Vec::new();

        // Process each word
        for (i, word) in words.iter().enumerate() {
            let word_lower = word.to_lowercase();

            // Capitalize the first word of the sentence
            if i == 0 {
                let capitalized = capitalize_first_letter(word);
                formatted_words.push(capitalized);
                continue;
            }

            // Handle special case for "I" pronoun
            if word_lower == "i" {
                formatted_words.push("I".to_string());
                continue;
            }

            // Handle special case for "Mr." and "Mister"
            if word_lower == "mr." || word_lower == "mister" {
                formatted_words
                    .push(if word_lower == "mr." { "Mr." } else { "Mister" }.to_string());
                continue;
            }

            // Handle special case for "Mrs." and "Ms."
            if word_lower == "mrs." || word_lower == "ms." {
                formatted_words.push(if word_lower == "mrs." { "Mrs." } else { "Ms." }.to_string());
                continue;
            }

            // Handle special case for "Dr."
            if word_lower == "dr." {
                formatted_words.push("Dr.".to_string());
                continue;
            }

            // Check if it's a proper noun
            let mut is_proper_noun = false;
            for &proper_noun in proper_nouns {
                if word_lower == proper_noun {
                    formatted_words.push(capitalize_first_letter(word));
                    is_proper_noun = true;
                    break;
                }
            }

            if is_proper_noun {
                continue;
            }

            // Check if it's a word that should be lowercase
            let mut is_lowercase_word = false;
            for &lowercase_word in LOWERCASE_WORDS {
                if word_lower == lowercase_word {
                    formatted_words.push(word_lower.clone());
                    is_lowercase_word = true;
                    break;
                }
            }

            if is_lowercase_word {
                continue;
            }

            // Preserve special characters and formatting
            if contains_special_formatting(word) {
                formatted_words.push(word.to_string());
                continue;
            }

            // For other words, preserve their original case if not all caps
            if is_all_caps {
                formatted_words.push(word_lower.clone());
            } else {
                formatted_words.push(word.to_string());
            }
        }

        // Join the words back into a sentence
        formatted_sentences.push(formatted_words.join(" "));
    }

    // Join the sentences with appropriate punctuation
    let mut result = String::new();
    let mut first = true;

    for sentence in formatted_sentences {
        if !first {
            result.push_str(". ");
        }
        result.push_str(&sentence);
        first = false;
    }

    // Add final punctuation if needed
    if !result.is_empty()
        && !result.ends_with('.')
        && !result.ends_with('!')
        && !result.ends_with('?')
    {
        result.push('.');
    }

    result
}

// Helper function to check if text is mostly uppercase
fn is_mostly_uppercase(text: &str) -> bool {
    let uppercase_count = text.chars().filter(|c| c.is_uppercase()).count();
    let lowercase_count = text.chars().filter(|c| c.is_lowercase()).count();

    // If more than 70% of alphabetic characters are uppercase, consider it all caps
    if uppercase_count + lowercase_count > 0 {
        (uppercase_count as f32 / (uppercase_count + lowercase_count) as f32) > 0.7
    } else {
        false
    }
}

// Helper function to check if a word contains special formatting like ♪ or other symbols
fn contains_special_formatting(word: &str) -> bool {
    word.contains('♪')
        || word.contains('♫')
        || word.contains('*')
        || word.contains('[')
        || word.contains(']')
        || word.contains('(')
        || word.contains(')')
}

// Helper function to capitalize the first letter of a word
pub fn capitalize_first_letter(word: &str) -> String {
    if word.is_empty() {
        return String::new();
    }

    let mut chars = word.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

// Common proper nouns for The Simpsons
pub const SIMPSONS_PROPER_NOUNS: &[&str] = &[
    "homer",
    "marge",
    "bart",
    "lisa",
    "maggie",
    "ned",
    "flanders",
    "moe",
    "barney",
    "smithers",
    "burns",
    "wiggum",
    "skinner",
    "krusty",
    "milhouse",
    "ralph",
    "nelson",
    "patty",
    "selma",
    "apu",
    "springfield",
    "shelbyville",
    "itchy",
    "scratchy",
    "troy",
    "mcclure",
    "lionel",
    "hutz",
    "dr.",
    "nick",
    "comic",
    "book",
    "guy",
    "willie",
    "otto",
    "edna",
    "krabappel",
    "martin",
    "duffman",
    "lenny",
    "carl",
    "hibbert",
    "quimby",
    "kent",
    "brockman",
    "sideshow",
    "bob",
    "mel",
    "jimbo",
    "dolph",
    "kearney",
    "groundskeeper",
    "superintendent",
    "chalmers",
    "america",
    "american",
    "usa",
    "u.s.a.",
    "u.s.",
    "god",
    "jesus",
    "christmas",
    "thanksgiving",
    "halloween",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
    "plow",
    "mr. plow",
];

// Common proper nouns for Futurama
pub const FUTURAMA_PROPER_NOUNS: &[&str] = &[
    "fry",
    "leela",
    "bender",
    "professor",
    "farnsworth",
    "zoidberg",
    "amy",
    "hermes",
    "zapp",
    "brannigan",
    "kif",
    "nibbler",
    "mom",
    "robot",
    "hypnotoad",
    "scruffy",
    "nixon",
    "calculon",
    "lrrr",
    "morbo",
    "linda",
    "url",
    "roberto",
    "flexo",
    "cubert",
    "dwight",
    "labarbara",
    "wernstrom",
    "bubblegum",
    "tate",
    "planet",
    "express",
    "omicron",
    "persei",
    "earth",
    "mars",
    "neptune",
    "uranus",
    "mercury",
    "venus",
    "jupiter",
    "saturn",
    "pluto",
    "new",
    "york",
    "new",
    "jersey",
    "robot",
    "devil",
    "hedonismbot",
    "slurms",
    "mckenzie",
    "destructor",
    "crushinator",
    "donbot",
    "clamps",
    "joey",
    "mousepad",
    "elzar",
    "ndnd",
    "yivo",
    "america",
    "american",
    "usa",
    "u.s.a.",
    "u.s.",
    "god",
    "jesus",
    "christmas",
    "thanksgiving",
    "halloween",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

// Common proper nouns for Rick and Morty
pub const RICK_AND_MORTY_PROPER_NOUNS: &[&str] = &[
    "rick",
    "morty",
    "summer",
    "beth",
    "jerry",
    "smith",
    "sanchez",
    "unity",
    "birdperson",
    "squanchy",
    "meeseeks",
    "gazorpazorp",
    "gearhead",
    "tammy",
    "evil",
    "morty",
    "jessica",
    "snuffles",
    "snowball",
    "mr.",
    "meeseeks",
    "abradolf",
    "lincler",
    "scary",
    "terry",
    "krombopulos",
    "michael",
    "mr.",
    "poopybutthole",
    "noob",
    "noob",
    "pencilvester",
    "america",
    "american",
    "usa",
    "u.s.a.",
    "u.s.",
    "god",
    "jesus",
    "christmas",
    "thanksgiving",
    "halloween",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
    "earth",
    "c-137",
    "citadel",
    "federation",
    "galactic",
    "federation",
    "council",
    "ricks",
    "dimension",
    "portal",
    "gun",
];
