use rand::seq::SliceRandom;
use rand::Rng;

pub struct TrumpInsultGenerator {
    adjectives: Vec<String>,
    nouns: Vec<String>,
    real_insults: Vec<String>,
}

impl TrumpInsultGenerator {
    pub fn new() -> Self {
        // List of insulting adjectives
        let adjectives = vec![
            "orange",
            "incompetent",
            "narcissistic",
            "delusional",
            "corrupt",
            "pathetic",
            "fraudulent",
            "unhinged",
            "moronic",
            "treasonous",
            "bloated",
            "rambling",
            "incoherent",
            "dishonest",
            "petulant",
            "infantile",
            "racist",
            "xenophobic",
            "misogynistic",
            "vindictive",
            "self-absorbed",
            "thin-skinned",
            "cowardly",
            "draft-dodging",
            "bankrupt",
            "failed",
            "impeached",
            "disgraced",
            "indicted",
            "convicted",
            "spray-tanned",
            "bumbling",
            "embarrassing",
            "shameless",
            "desperate",
            "whining",
            "lying",
            "cheating",
            "grifting",
            "gaslighting",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        // List of insulting nouns
        let nouns = vec![
            "shitweasel",
            "grifter",
            "conman",
            "traitor",
            "buffoon",
            "manchild",
            "sociopath",
            "narcissist",
            "criminal",
            "fraud",
            "tyrant",
            "dictator",
            "fascist",
            "demagogue",
            "charlatan",
            "liar",
            "cheat",
            "bully",
            "coward",
            "loser",
            "disgrace",
            "embarrassment",
            "failure",
            "joke",
            "menace",
            "disaster",
            "catastrophe",
            "nightmare",
            "monstrosity",
            "abomination",
            "felon",
            "crook",
            "scammer",
            "swindler",
            "huckster",
            "blowhard",
            "windbag",
            "gasbag",
            "blatherskite",
            "ignoramus",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        // List of real Trump insults/nicknames from the internet
        // Source: https://medium.com/@monalazzar/129-insulting-trump-nicknames-you-must-know-choose-your-favorite-ef2bffb133b6
        let real_insults = vec![
            "Agent Orange",
            "America's Berlusconi",
            "Angry Cheeto",
            "Barbecued Brutus",
            "Bigly Bigot",
            "Billionaire Brat",
            "Bloviating Billionaire",
            "Bratman",
            "Bumbledore",
            "Cinnamon Hitler",
            "Clown Prince of Politics",
            "Commander-in-Grief",
            "Conspiracy Theorist-in-Chief",
            "Corruptor-in-Chief",
            "Creep Throat",
            "Crybaby Trump",
            "Cult Leader",
            "Dainty Donald",
            "Dime Store Dictator",
            "Dire Abby",
            "Dishonest Don",
            "Don the Con",
            "Donald Chump",
            "Donald Dump",
            "Donald Drumpf",
            "Donnie Darko",
            "Donnie Demento",
            "Donnie Dollhands",
            "Donnie Drumpster Fire",
            "Donnie TicTac",
            "Draft Dodger Don",
            "Feral Cheeto",
            "Forrest Trump",
            "Fraud Trump",
            "Fuckface Von Clownstick",
            "Gaslight Anthem",
            "Genghis Can't",
            "Godzilla with Less Foreign Policy Experience",
            "Golden Wrecking Ball",
            "Great Orange Hope",
            "Hair Apparent",
            "Hair Führer",
            "Herr Drumpf",
            "Human Cheeto",
            "Human Tanning Bed Warning Label",
            "Humble Braggart",
            "Impulse Power",
            "Individual-1",
            "King Leer",
            "King Mierdas",
            "King of Debt",
            "Lord Dampnut",
            "Lord Voldemort",
            "Mango Mussolini",
            "Man-Baby",
            "Manchurian Candidate",
            "Narcissistic Nectarine",
            "New York Pork Dork",
            "Orange Caligula",
            "Orange Julius Caesar",
            "Orange Menace",
            "Peach Emperor",
            "Perjurer-in-Chief",
            "POTUS WRECKS",
            "Prima Donald",
            "Putin's Puppet",
            "Rage-Tweeting Racist",
            "Rapey Von Tinyhands",
            "Resident Dump",
            "Screaming Carrot Demon",
            "Sexual-Predator-in-Chief",
            "Short-Fingered Vulgarian",
            "Snake Oil Salesman",
            "Snowflake-in-Chief",
            "Tangerine Tornado",
            "Tangerine Tyrant",
            "Teflon Don",
            "The Angry Pumpkin",
            "The Fanta Fascist",
            "The Fraud of Fifth Avenue",
            "The Grabber",
            "The Groper",
            "The Human Tanning Bed Warning Label",
            "The Insulter",
            "The Lyin' King",
            "The Microphallus",
            "The Mutinous Cheeto",
            "The Orange One",
            "The Real Covfefe",
            "The Talking Yam",
            "The Tiny-Handed Tyrant",
            "The Toddler-in-Chief",
            "Tiny Hands Trump",
            "Treasonous Trump",
            "Trumplethinskin",
            "Trumpty Dumpty",
            "Tweetolini",
            "Twitter Twit",
            "Two-Bit Dictator",
            "Unindicted Co-conspirator",
            "Vanilla ISIS",
            "Very Stable Genius",
            "Wanna-be Dictator",
            "Wannabe Dictator",
            "Whiny Donald",
            "Xenophobic Potato",
            "Xenophobic Sweet Potato",
            "Yeti Pubes",
            "Zaphod Beeblebrox",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        Self {
            adjectives,
            nouns,
            real_insults,
        }
    }

    pub fn generate_insult(&self) -> String {
        let mut rng = rand::thread_rng();

        // 50% chance to use a real insult from the internet
        // 50% chance to generate a random adjective-noun combination
        if rng.gen_bool(0.5) {
            // Use a real insult
            self.real_insults
                .choose(&mut rng)
                .map(|s| s.to_string())
                .unwrap_or_else(|| "Mango Mussolini".to_string())
        } else {
            // Generate a random adjective-noun combination
            let default_adj = "incompetent";
            let default_noun = "grifter";

            let adjective = self
                .adjectives
                .choose(&mut rng)
                .map(|s| s.as_str())
                .unwrap_or(default_adj);

            let noun = self
                .nouns
                .choose(&mut rng)
                .map(|s| s.as_str())
                .unwrap_or(default_noun);

            format!("{} {}", adjective, noun)
        }
    }
}
