use anyhow::Result;
use rand::seq::SliceRandom;
use rand::Rng;

pub struct CrimeFightingGenerator;

impl CrimeFightingGenerator {
    pub fn new() -> Self {
        Self {}
    }

    pub fn generate_duo(&self, speaker1: &str, speaker2: &str) -> Result<String> {
        // Generate random descriptions
        let mut rng = rand::thread_rng();
        
        // Male descriptors (he1)
        let descriptions1 = [
            "a superhumanly strong",
            "an underprivileged",
            "a globe-trotting",
            "an impetuous",
            "a shy",
            "a suave",
            "a notorious",
            "a one-legged",
            "an all-American",
            "a short-sighted",
            "an otherworldly",
            "a hate-fueled",
            "a scrappy",
            "an unconventional",
            "a jaded",
            "a leather-clad",
            "a fiendish",
            "a Nobel prize-winning",
            "a suicidal",
            "a maverick",
            "a bookish",
            "an old-fashioned",
            "a witless",
            "a lounge-singing",
            "a war-weary",
            "a scarfaced",
            "a gun-slinging",
            "an obese",
            "a time-tossed",
            "a benighted",
            "an uncontrollable",
            "an immortal",
            "an oversexed",
            "a world-famous",
            "an ungodly",
            "a fast talking",
            "a deeply religious",
            "a lonely",
            "a sword-wielding",
            "a genetically engineered",
        ];
        
        // Male descriptors (he2)
        let descriptors1 = [
            "white trash",
            "zombie",
            "shark-wrestling",
            "playboy",
            "guitar-strumming",
            "Jewish",
            "sweet-toothed",
            "bohemian",
            "crooked",
            "chivalrous",
            "moralistic",
            "amnesiac",
            "devious",
            "drug-addicted",
            "voodoo",
            "Catholic",
            "overambitious",
            "coffee-fuelled",
            "pirate",
            "misogynist",
            "skateboarding",
            "arachnophobic",
            "Amish",
            "small-town",
            "Republican",
            "one-eyed",
            "gay",
            "guerilla",
            "vegetarian",
            "dishevelled",
            "alcoholic",
            "flyboy",
            "ninja",
            "albino",
            "hunchbacked",
            "neurotic",
            "umbrella-wielding",
            "native American",
            "soccer-playing",
            "day-dreaming",
        ];
        
        // Male occupations (he3)
        let occupations1 = [
            "grifter",
            "stage actor",
            "paramedic",
            "gentleman spy",
            "jungle king",
            "hairdresser",
            "photographer",
            "ex-con",
            "vagrant",
            "filmmaker",
            "werewolf",
            "senator",
            "romance novelist",
            "shaman",
            "cop",
            "rock star",
            "farmboy",
            "cat burglar",
            "cowboy",
            "cyborg",
            "inventor",
            "assassin",
            "boxer",
            "dog-catcher",
            "master criminal",
            "gangster",
            "firefighter",
            "househusband",
            "dwarf",
            "librarian",
            "paranormal investigator",
            "Green Beret",
            "waffle chef",
            "vampire hunter",
            "messiah",
            "astronaut",
            "sorceror",
            "card sharp",
            "matador",
            "barbarian",
        ];
        
        // Male traits (he4)
        let traits1 = [
            "with a robot buddy named Sparky",
            "whom everyone believes is mad",
            "gone bad",
            "with a mysterious suitcase handcuffed to his arm",
            "living undercover at Ringling Bros. Circus",
            "searching for his wife's true killer",
            "who dotes on his loving old ma",
            "looking for 'the Big One'",
            "who knows the secret of the alien invasion",
            "on the edge",
            "on a mission from God",
            "with a passion for justice",
            "with a troubled past",
            "with a score to settle",
            "with nothing left to lose",
            "with a secret identity",
            "with supernatural abilities",
            "with advanced martial arts training",
            "with a tragic backstory",
            "with a vendetta against crime",
        ];
        
        // Female descriptors (she1)
        let descriptions2 = [
            "a radical",
            "a green-fingered",
            "a tortured",
            "a time-travelling",
            "a vivacious",
            "a scantily clad",
            "a mistrustful",
            "a violent",
            "a transdimensional",
            "a strong-willed",
            "a ditzy",
            "a man-hating",
            "a high-kicking",
            "a blind",
            "an elegant",
            "a supernatural",
            "a foxy",
            "a bloodthirsty",
            "a cynical",
            "a beautiful",
            "a plucky",
            "a sarcastic",
            "a psychotic",
            "a hard-bitten",
            "a manipulative",
            "an orphaned",
            "a cosmopolitan",
            "a chain-smoking",
            "a cold-hearted",
            "a warm-hearted",
            "a sharp-shooting",
            "an enchanted",
            "a wealthy",
            "a pregnant",
            "a mentally unstable",
            "a virginal",
            "a brilliant",
            "a disco-crazy",
            "a provocative",
            "an artistic",
            "a steroid-ripped",
        ];
        
        // Female descriptors (she2)
        let descriptors2 = [
            "tempestuous",
            "Buddhist",
            "foul-mouthed",
            "nymphomaniac",
            "green-skinned",
            "impetuous",
            "African-American",
            "punk",
            "hypochondriac",
            "junkie",
            "blonde",
            "goth",
            "insomniac",
            "gypsy",
            "mutant",
            "renegade",
            "tomboy",
            "French-Canadian",
            "motormouth",
            "belly-dancing",
            "communist",
            "hip-hop",
            "thirtysomething",
            "cigar-chomping",
            "extravagent",
            "out-of-work",
            "Bolivian",
            "mute",
            "cat-loving",
            "snooty",
            "wisecracking",
            "red-headed",
            "winged",
            "kleptomaniac",
            "antique-collecting",
            "psychic",
            "gold-digging",
            "bisexual",
            "paranoid",
            "streetsmart",
            "jittery",
        ];
        
        // Female occupations (she3)
        let occupations2 = [
            "archaeologist",
            "pearl diver",
            "mechanic",
            "detective",
            "hooker",
            "femme fatale",
            "former first lady",
            "barmaid",
            "fairy princess",
            "magician's assistant",
            "schoolgirl",
            "college professor",
            "angel",
            "bounty hunter",
            "opera singer",
            "cab driver",
            "soap star",
            "doctor",
            "politician",
            "lawyer",
            "nun",
            "snake charmer",
            "journalist",
            "bodyguard",
            "vampire",
            "stripper",
            "Valkyrie",
            "wrestler",
            "mermaid",
            "single mother",
            "safe cracker",
            "traffic cop",
            "research scientist",
            "queen of the dead",
            "Hell's Angel",
            "museum curator",
            "advertising executive",
            "widow",
            "mercenary",
            "socialite",
            "serial killer",
        ];
        
        // Female traits (she4)
        let traits2 = [
            "on her way to prison for a murder she didn't commit",
            "trying to make a difference in a man's world",
            "with the soul of a mighty warrior",
            "looking for love in all the wrong places",
            "with an MBA from Harvard",
            "who hides her beauty behind a pair of thick-framed spectacles",
            "with the power to see death",
            "descended from a line of powerful witches",
            "with a mysterious tattoo",
            "with a dark secret",
            "with a hidden agenda",
            "with a troubled past",
            "with a heart of gold",
            "with a photographic memory",
            "with a chip on her shoulder",
            "with a score to settle",
            "with a mysterious past",
            "with a secret technique",
            "with a passion for justice",
            "with unconventional methods",
        ];
        
        // Verbs for "They [verb] [noun]!"
        let verbs = [
            "fight",
            "battle",
            "conquer",
            "struggle against",
            "like to swim in",
            "embrace",
            "repossess",
            "poison",
            "speak out against",
            "suck",
            "bang",
            "lick",
            "annihilate",
            "surf with",
            "spank",
            "terrorize",
            "sell crack to",
            "fight",
            "pleasure",
            "kill",
            "appreciate",
            "write songs about",
            "write angry letters to",
            "learn to love",
            "grab",
            "love",
        ];
        
        // Nouns for "They [verb] [noun]!"
        let nouns = [
            "crime",
            "#dapcentral",
            "eDonkey",
            "the homeless",
            "poverty",
            "conservatism",
            "little boys",
            "equal rights",
            "pants",
            "sheep",
            "toads",
            "the Internet",
            "the city",
            "mayonnaise",
            "the DAP",
            "you",
            "vintage porn",
            "the ladies",
            "Congress",
            "the MPAA",
            "Nazis",
            "clowns",
            "the Pope",
            "the vast oceans of Mars",
            "evil",
            "injustice",
            "corruption",
            "the underworld",
            "the establishment",
            "the system",
        ];
        
        // Choose a random template format (like the original gbot)
        let frame = rng.gen_range(0..10);
        
        // Select random elements from each array
        let desc1 = descriptions1.choose(&mut rng).unwrap();
        let desc2 = descriptions2.choose(&mut rng).unwrap();
        let descriptor1 = descriptors1.choose(&mut rng).unwrap();
        let descriptor2 = descriptors2.choose(&mut rng).unwrap();
        let occ1 = occupations1.choose(&mut rng).unwrap();
        let occ2 = occupations2.choose(&mut rng).unwrap();
        let trait1 = traits1.choose(&mut rng).unwrap();
        let trait2 = traits2.choose(&mut rng).unwrap();
        let verb = verbs.choose(&mut rng).unwrap();
        let noun = nouns.choose(&mut rng).unwrap();
        
        // Format the crime fighting duo description based on the template
        let duo_description = if frame < 3 {
            // Template 1: First person has more descriptors, second person has fewer
            format!(
                "{} is {} {} {} {}. {} is {} {}. They {} {}!",
                speaker1, desc1, descriptor1, occ1, trait1,
                speaker2, desc2, occ2,
                verb, noun
            )
        } else if frame < 5 {
            // Template 2: First person has fewer descriptors, second person has more
            format!(
                "{} is {} {}. {} is {} {} {} {}. They {} {}!",
                speaker1, desc1, occ1,
                speaker2, desc2, descriptor2, occ2, trait2,
                verb, noun
            )
        } else {
            // Template 3: Both people have full descriptors (original pattern)
            format!(
                "{} is {} {} {} {}. {} is {} {} {} {}. They {} {}!",
                speaker1, desc1, descriptor1, occ1, trait1,
                speaker2, desc2, descriptor2, occ2, trait2,
                verb, noun
            )
        };
        
        Ok(duo_description)
    }
}
