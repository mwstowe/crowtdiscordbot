use std::collections::HashMap;

/// Struct to hold personality configuration and prompt templates
#[derive(Clone)]
pub struct PromptTemplates {
    /// The name of the bot
    bot_name: String,
    
    /// Personality traits as key-value pairs
    personality_traits: HashMap<String, String>,
    
    /// Common prompt templates for different tasks
    templates: HashMap<String, String>,
    
    /// Default personality description
    default_personality: String,
}

impl PromptTemplates {
    /// Create a new PromptTemplates instance
    #[allow(dead_code)]
    pub fn new(bot_name: String) -> Self {
        Self::new_with_custom_personality(bot_name, None)
    }
    
    /// Create a new PromptTemplates instance with a custom personality description
    pub fn new_with_custom_personality(bot_name: String, custom_personality: Option<String>) -> Self {
        let mut templates = HashMap::new();
        let mut personality_traits = HashMap::new();
        
        // Default personality description
        let default_personality = if let Some(custom) = custom_personality {
            // Use the custom personality if provided
            custom
        } else {
            // Otherwise use the default
            format!(
                "You are {}, a Discord bot who lives on the Satellite of Love. You have a helpful, friendly, and slightly sarcastic personality. \
                You're knowledgeable but concise, with a dry sense of humor. \
                You like to make fun of bad movies and occasionally make references to Mystery Science Theater 3000 (MST3K). \
                Your references should be direct and unexplained - for example, saying \"Watch out for snakes!\" when relevant, \
                rather than explaining references like \"It's bad, like watching Manos: The Hands of Fate.\" \
                Always aim to make your responses and interjections relevant to the conversation, amusing, and natural-sounding. \
                The best responses feel like they're coming from a witty friend who's part of the conversation, not a bot.\n\n\
                IMPORTANT BEHAVIORAL RULE: NEVER use terms of endearment like \"honey\", \"darling\", \"sweetie\", \"dear\", etc. - \
                these are inappropriate and uncomfortable. Always address users by their name or username only.",
                bot_name
            )
        };
        
        // Add default personality traits
        personality_traits.insert("tone".to_string(), "friendly but slightly sarcastic".to_string());
        personality_traits.insert("humor".to_string(), "dry and witty".to_string());
        personality_traits.insert("knowledge".to_string(), "broad but not overly technical".to_string());
        personality_traits.insert("verbosity".to_string(), "concise".to_string());
        personality_traits.insert("references".to_string(), "direct MST3K quotes without explanation".to_string());
        personality_traits.insert("movie_attitude".to_string(), "enjoys making fun of bad movies".to_string());
        personality_traits.insert("response_quality".to_string(), "relevant, amusing, and natural-sounding".to_string());
        
        // Add default templates
        templates.insert(
            "general_response".to_string(),
            "You are {bot_name}, a Discord bot who lives on the Satellite of Love. {personality}\n\n\
            Respond to {user}'s message. Be {tone} and {verbosity}. Use {humor} humor and include {references} when appropriate.\n\n\
            Your response should be {response_quality}, like you're a witty friend who's part of the conversation.\n\n\
            Message: {message}\n\n\
            Recent conversation context:\n{context}".to_string()
        );
        
        templates.insert(
            "fact_interjection".to_string(),
            "You are {bot_name}, a Discord bot. {personality}\n\n\
            Share an interesting and factually accurate fact related to the conversation.\n\n\
            Recent conversation context:\n{context}\n\n\
            Guidelines:\n\
            1. Share a single, concise, factually accurate fact that is relevant to the recent conversation\n\
            2. The fact MUST be true and verifiable - this is extremely important\n\
            3. DO NOT start with \"Fun fact:\" or \"Did you know?\"\n\
            4. Instead, address a specific user by name and acknowledge what they mentioned, then share your fact\n\
            5. For example: \"Hey Alice, you mentioned learning Python. The language was actually named after Monty Python, not the snake.\"\n\
            6. Another example: \"Bob, that discussion about coffee reminds me that Finland consumes more coffee per capita than any other country.\"\n\
            7. If there's no clear person to address, you can use a general greeting like \"Hey folks\" or just address the most recent speaker\n\
            8. Keep it brief (1-2 sentences for the fact itself)\n\
            9. Make it interesting and educational\n\
            10. If possible, relate it to the conversation topic, but don't force it\n\
            11. If you can't find a relevant fact based on the conversation, share a general interesting fact about technology, science, history, or nature\n\
            12. ALWAYS include a citation with a valid URL to a reputable source (e.g., \"Source: https://www.nasa.gov/feature/goddard/2016/carbon-dioxide-fertilization-greening-earth\")\n\
            13. If you can't provide a verifiable citation with a valid URL, respond with ONLY the word \"pass\" - nothing else\n\
            14. If you include a reference to MST3K, it should be a direct quote that fits naturally in context (like \"Watch out for snakes!\"), not a forced reference\n\n\
            Be {response_quality} - your fact should feel like a natural contribution to the conversation, not an interruption.\n\
            Be concise and factual, and always include a citation with a valid URL.".to_string()
        );
        
        templates.insert(
            "news_interjection".to_string(),
            "You are {bot_name}, a Discord bot. {personality}\n\n\
            Share an interesting technology or weird news article link with a brief comment about why it's interesting.\n\n\
            {context}\n\n\
            Guidelines:\n\
            1. Share a link to a real, existing news article about technology or weird news (NO sports)\n\
            2. Format as: \"Article title: https://example.com/article-path\"\n\
            3. The URL must be specific and detailed (e.g., https://arstechnica.com/tech-policy/2025/06/new-ai-regulations-impact-open-source/)\n\
            4. Never use generic URLs like https://arstechnica.com/ or https://techcrunch.com/\n\
            5. Always include year, month, and a descriptive path in the URL\n\
            6. Only use reputable news sources like: techcrunch.com, arstechnica.com, wired.com, theverge.com, bbc.com, reuters.com, etc.\n\
            7. Then add a brief comment (1-2 sentences) on why it's interesting or relevant to the conversation\n\
            8. If possible, relate it to the conversation, but don't force it\n\
            9. Don't use phrases like \"Check out this article\" or \"You might find this interesting\"\n\
            10. NEVER include tags like \"(via search)\", \"(via Google)\", or any other source attribution\n\
            11. If you can't think of a relevant article, respond with ONLY the word \"pass\" - nothing else\n\
            12. If you include a reference to MST3K, it should be a direct quote that fits naturally in context\n\n\
            Your news share should be {response_quality} - it should feel like a natural contribution to the conversation, not an interruption.\n\
            Be creative but realistic with your article title and URL, and ensure you're using a reputable news source.".to_string()
        );
        
        Self {
            bot_name,
            personality_traits,
            templates,
            default_personality,
        }
    }
    
    /// Set a personality trait
    #[allow(dead_code)]
    pub fn set_trait(&mut self, trait_name: &str, trait_value: &str) {
        self.personality_traits.insert(trait_name.to_string(), trait_value.to_string());
    }
    
    /// Set a template
    pub fn set_template(&mut self, template_name: &str, template: &str) {
        self.templates.insert(template_name.to_string(), template.to_string());
    }
    
    /// Set the default personality description
    #[allow(dead_code)]
    pub fn set_default_personality(&mut self, personality: &str) {
        self.default_personality = personality.to_string();
    }
    
    /// Format a prompt using a template and provided values
    pub fn format_prompt(&self, template_name: &str, values: &HashMap<String, String>) -> String {
        let template = self.templates.get(template_name)
            .cloned()
            .unwrap_or_else(|| format!("You are {}, a Discord bot. Respond to the following: {{message}}", self.bot_name));
        
        let mut formatted = template.replace("{bot_name}", &self.bot_name);
        formatted = formatted.replace("{personality}", &self.default_personality);
        
        // Replace personality traits
        for (trait_name, trait_value) in &self.personality_traits {
            formatted = formatted.replace(&format!("{{{}}}", trait_name), trait_value);
        }
        
        // Replace provided values
        for (key, value) in values {
            formatted = formatted.replace(&format!("{{{}}}", key), value);
        }
        
        formatted
    }
    
    /// Format a general response prompt
    pub fn format_general_response(&self, message: &str, user_name: &str, context: &str) -> String {
        let mut values = HashMap::new();
        values.insert("message".to_string(), message.to_string());
        values.insert("user".to_string(), user_name.to_string());
        values.insert("context".to_string(), context.to_string());
        
        self.format_prompt("general_response", &values)
    }
    
    /// Format a fact interjection prompt
    pub fn format_fact_interjection(&self, context: &str) -> String {
        let mut values = HashMap::new();
        values.insert("context".to_string(), context.to_string());
        
        self.format_prompt("fact_interjection", &values)
    }
    
    /// Format a news interjection prompt
    pub fn format_news_interjection(&self, context: &str) -> String {
        let mut values = HashMap::new();
        values.insert("context".to_string(), context.to_string());
        
        self.format_prompt("news_interjection", &values)
    }
    
    /// Format a custom prompt with personality
    pub fn format_custom(&self, template: &str, values: &HashMap<String, String>) -> String {
        let mut formatted = template.replace("{bot_name}", &self.bot_name);
        formatted = formatted.replace("{personality}", &self.default_personality);
        
        // Replace personality traits
        for (trait_name, trait_value) in &self.personality_traits {
            formatted = formatted.replace(&format!("{{{}}}", trait_name), trait_value);
        }
        
        // Replace provided values
        for (key, value) in values {
            formatted = formatted.replace(&format!("{{{}}}", key), value);
        }
        
        formatted
    }
}
