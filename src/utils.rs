use regex::Regex;
use lazy_static::lazy_static;

lazy_static! {
    // Regex to match common pronoun patterns in usernames
    // Matches patterns like (he/him), [she/her], (they/them), etc.
    static ref PRONOUN_REGEX: Regex = Regex::new(r"[\(\[\{]([a-z]+/[a-z]+(?:/[a-z]+)*)[\)\]\}]").unwrap();
}

/// Extract pronouns from a username or display name
/// Returns the pronouns if found, None otherwise
pub fn extract_pronouns(name: &str) -> Option<String> {
    if let Some(captures) = PRONOUN_REGEX.captures(name) {
        if let Some(pronoun_match) = captures.get(1) {
            return Some(pronoun_match.as_str().to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_pronouns() {
        // Test with parentheses
        assert_eq!(extract_pronouns("Alice (she/her)"), Some("she/her".to_string()));
        
        // Test with brackets
        assert_eq!(extract_pronouns("Bob [he/him]"), Some("he/him".to_string()));
        
        // Test with curly braces
        assert_eq!(extract_pronouns("Charlie {they/them}"), Some("they/them".to_string()));
        
        // Test with no pronouns
        assert_eq!(extract_pronouns("Dave"), None);
        
        // Test with multiple pronouns
        assert_eq!(extract_pronouns("Eve (she/her/hers)"), Some("she/her/hers".to_string()));
        
        // Test with text before and after
        assert_eq!(extract_pronouns("Frank (he/him) Admin"), Some("he/him".to_string()));
    }
}
