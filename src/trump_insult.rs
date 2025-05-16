use rand::seq::SliceRandom;

pub struct TrumpInsultGenerator {
    adjectives: Vec<String>,
    nouns: Vec<String>,
}

impl TrumpInsultGenerator {
    pub fn new() -> Self {
        // List of insulting adjectives
        let adjectives = vec![
            "orange", "incompetent", "narcissistic", "delusional", "corrupt", 
            "pathetic", "fraudulent", "unhinged", "moronic", "treasonous",
            "bloated", "rambling", "incoherent", "dishonest", "petulant",
            "infantile", "racist", "xenophobic", "misogynistic", "vindictive",
            "self-absorbed", "thin-skinned", "cowardly", "draft-dodging", "bankrupt",
            "failed", "impeached", "disgraced", "indicted", "convicted",
            "spray-tanned", "bumbling", "embarrassing", "shameless", "desperate",
            "whining", "lying", "cheating", "grifting", "gaslighting"
        ].into_iter().map(String::from).collect();
        
        // List of insulting nouns
        let nouns = vec![
            "shitweasel", "grifter", "conman", "traitor", "buffoon",
            "manchild", "sociopath", "narcissist", "criminal", "fraud",
            "tyrant", "dictator", "fascist", "demagogue", "charlatan",
            "liar", "cheat", "bully", "coward", "loser",
            "disgrace", "embarrassment", "failure", "joke", "menace",
            "disaster", "catastrophe", "nightmare", "monstrosity", "abomination",
            "felon", "crook", "scammer", "swindler", "huckster",
            "blowhard", "windbag", "gasbag", "blatherskite", "ignoramus"
        ].into_iter().map(String::from).collect();
        
        Self {
            adjectives,
            nouns,
        }
    }
    
    pub fn generate_insult(&self) -> String {
        let mut rng = rand::thread_rng();
        
        // Use default values that don't involve temporary strings
        let default_adj = "incompetent";
        let default_noun = "grifter";
        
        let adjective = self.adjectives.choose(&mut rng)
            .map(|s| s.as_str())
            .unwrap_or(default_adj);
            
        let noun = self.nouns.choose(&mut rng)
            .map(|s| s.as_str())
            .unwrap_or(default_noun);
            
        format!("{} {}", adjective, noun)
    }
}
