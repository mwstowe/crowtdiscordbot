use rand::seq::SliceRandom;
use rand::Rng;

pub struct BandGenreGenerator {
    adjectives: Vec<String>,
    genres: Vec<String>,
    modifiers: Vec<String>,
    absurd_genres: Vec<String>,
}

impl BandGenreGenerator {
    pub fn new() -> Self {
        let adjectives = vec![
            "post".to_string(),
            "neo".to_string(),
            "proto".to_string(),
            "retro".to_string(),
            "avant".to_string(),
            "experimental".to_string(),
            "progressive".to_string(),
            "psychedelic".to_string(),
            "ambient".to_string(),
            "industrial".to_string(),
            "cosmic".to_string(),
            "ethereal".to_string(),
            "dystopian".to_string(),
            "utopian".to_string(),
            "quantum".to_string(),
            "cyber".to_string(),
            "digital".to_string(),
            "analog".to_string(),
            "organic".to_string(),
            "synthetic".to_string(),
        ];

        let genres = vec![
            "punk".to_string(),
            "metal".to_string(),
            "jazz".to_string(),
            "folk".to_string(),
            "rock".to_string(),
            "pop".to_string(),
            "wave".to_string(),
            "core".to_string(),
            "funk".to_string(),
            "soul".to_string(),
            "blues".to_string(),
            "grunge".to_string(),
            "disco".to_string(),
            "techno".to_string(),
            "house".to_string(),
            "trance".to_string(),
            "dubstep".to_string(),
            "ska".to_string(),
            "reggae".to_string(),
            "rap".to_string(),
        ];

        let modifiers = vec![
            "fusion".to_string(),
            "revival".to_string(),
            "wave".to_string(),
            "core".to_string(),
            "gaze".to_string(),
            "step".to_string(),
            "hop".to_string(),
            "beat".to_string(),
            "tronica".to_string(),
            "scape".to_string(),
        ];

        let absurd_genres = vec![
            "recursive polka".to_string(),
            "interpretive silence".to_string(),
            "quantum yodeling".to_string(),
            "bureaucratic noise".to_string(),
            "existential elevator music".to_string(),
            "passive-aggressive ambient".to_string(),
            "minimalist maximalism".to_string(),
            "corporate zen".to_string(),
            "caffeinated slowcore".to_string(),
            "anti-music".to_string(),
            "theoretical jazz".to_string(),
            "accidental rhythm".to_string(),
            "recursive recursion".to_string(),
            "meta-meta".to_string(),
            "post-everything".to_string(),
        ];

        Self {
            adjectives,
            genres,
            modifiers,
            absurd_genres,
        }
    }

    pub fn generate_genre(&self, band_name: &str) -> String {
        let mut rng = rand::thread_rng();

        // 20% chance of using an absurd genre
        let genre = if rng.gen_bool(0.2) {
            self.absurd_genres.choose(&mut rng).unwrap().clone()
        } else {
            // Build a compound genre
            let mut parts = Vec::new();

            // 80% chance of adding an adjective
            if rng.gen_bool(0.8) {
                parts.push(self.adjectives.choose(&mut rng).unwrap().clone());
            }

            // Always add a base genre
            parts.push(self.genres.choose(&mut rng).unwrap().clone());

            // 50% chance of adding a modifier
            if rng.gen_bool(0.5) {
                parts.push(self.modifiers.choose(&mut rng).unwrap().clone());
            }

            // 30% chance of adding another genre for fusion
            if rng.gen_bool(0.3) {
                parts.push("-".to_string());
                parts.push(self.genres.choose(&mut rng).unwrap().clone());
            }

            parts.join("-")
        };
        
        format!("What kind of music does {} play? {}", band_name, genre)
    }
}
