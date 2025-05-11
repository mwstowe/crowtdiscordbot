use anyhow::Result;
use rand::seq::SliceRandom;

pub struct CrimeFightingGenerator;

impl CrimeFightingGenerator {
    pub fn new() -> Self {
        Self {}
    }

    pub fn generate_duo(&self, speaker1: &str, speaker2: &str) -> Result<String> {
        // Generate random descriptions
        let mut rng = rand::thread_rng();
        
        let descriptions1 = [
            "a superhumanly strong",
            "a brilliant but troubled",
            "a time-traveling",
            "a genetically enhanced",
            "a cybernetically augmented",
            "a telepathic",
            "a shape-shifting",
            "a dimension-hopping",
            "a technologically advanced",
            "a magically empowered",
        ];
        
        let occupations1 = [
            "former detective",
            "ex-spy",
            "disgraced scientist",
            "retired superhero",
            "rogue AI researcher",
            "reformed villain",
            "exiled royal",
            "amnesiac assassin",
            "interdimensional refugee",
            "time-displaced warrior",
        ];
        
        let traits1 = [
            "with a mysterious past",
            "with a score to settle",
            "with nothing left to lose",
            "with a secret identity",
            "with supernatural abilities",
            "with advanced martial arts training",
            "with a tragic backstory",
            "with a vendetta against crime",
            "with a photographic memory",
            "with unfinished business",
        ];
        
        let descriptions2 = [
            "a sarcastic",
            "a no-nonsense",
            "a radical",
            "a by-the-book",
            "a rebellious",
            "a tech-savvy",
            "a streetwise",
            "a wealthy",
            "a mysterious",
            "an eccentric",
        ];
        
        let occupations2 = [
            "hacker",
            "martial artist",
            "forensic scientist",
            "archaeologist",
            "journalist",
            "medical examiner",
            "weapons expert",
            "psychologist",
            "conspiracy theorist",
            "paranormal investigator",
        ];
        
        let traits2 = [
            "with a secret technique",
            "with a passion for justice",
            "with unconventional methods",
            "with a troubled past",
            "with powerful connections",
            "with a unique perspective",
            "with specialized equipment",
            "with a hidden agenda",
            "with incredible luck",
            "with unwavering determination",
        ];
        
        // Select random descriptions
        let desc1 = descriptions1.choose(&mut rng).unwrap();
        let occ1 = occupations1.choose(&mut rng).unwrap();
        let trait1 = traits1.choose(&mut rng).unwrap();
        
        let desc2 = descriptions2.choose(&mut rng).unwrap();
        let occ2 = occupations2.choose(&mut rng).unwrap();
        let trait2 = traits2.choose(&mut rng).unwrap();
        
        // Format the crime fighting duo description
        let duo_description = format!(
            "{} is {} {} {}. {} is {} {} {}. They fight crime!",
            speaker1, desc1, occ1, trait1,
            speaker2, desc2, occ2, trait2
        );
        
        Ok(duo_description)
    }
}
