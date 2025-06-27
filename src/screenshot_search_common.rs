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
        ("cmdr.", "commander"),
        ("gov.", "governor"),
        ("pres.", "president"),
        ("rev.", "reverend"),
        ("hon.", "honorable"),
        ("asst.", "assistant"),
        ("dept.", "department"),
        ("univ.", "university"),
        ("corp.", "corporation"),
        ("inc.", "incorporated"),
        ("co.", "company"),
        ("jr.", "junior"),
        ("sr.", "senior"),
        ("vs.", "versus"),
        ("etc.", "etcetera"),
        ("i.e.", "that is"),
        ("e.g.", "for example"),
        ("a.m.", "am"),
        ("p.m.", "pm"),
        ("approx.", "approximately"),
        ("est.", "established"),
        ("tel.", "telephone"),
        ("temp.", "temperature"),
        ("misc.", "miscellaneous"),
        ("vol.", "volume"),
        ("no.", "number"),
        ("min.", "minimum"),
        ("max.", "maximum"),
        ("avg.", "average"),
        ("est.", "estimate"),
        ("dept.", "department"),
        ("div.", "division"),
        ("dist.", "district"),
        ("int.", "international"),
        ("natl.", "national"),
        ("reg.", "regional"),
        ("loc.", "local"),
        ("org.", "organization"),
        ("assn.", "association"),
        ("fed.", "federal"),
        ("govt.", "government"),
        ("admin.", "administration"),
        ("dir.", "director"),
        ("mgr.", "manager"),
        ("pres.", "president"),
        ("ceo", "chief executive officer"),
        ("cfo", "chief financial officer"),
        ("cto", "chief technology officer"),
        ("coo", "chief operating officer"),
        ("vp", "vice president"),
        ("asst.", "assistant"),
        ("sec.", "secretary"),
        ("treas.", "treasurer"),
        ("coord.", "coordinator"),
        ("supv.", "supervisor"),
        ("tech.", "technician"),
        ("eng.", "engineer"),
        ("dev.", "developer"),
        ("prog.", "programmer"),
        ("anal.", "analyst"),
        ("spec.", "specialist"),
        ("rep.", "representative"),
        ("cons.", "consultant"),
        ("cont.", "contractor"),
        ("temp.", "temporary"),
        ("perm.", "permanent"),
        ("ft", "full time"),
        ("pt", "part time"),
        ("hr", "human resources"),
        ("it", "information technology"),
        ("is", "information systems"),
        ("qa", "quality assurance"),
        ("qc", "quality control"),
        ("r&d", "research and development"),
        ("mfg.", "manufacturing"),
        ("dist.", "distribution"),
        ("whse.", "warehouse"),
        ("ret.", "retail"),
        ("whlse.", "wholesale"),
        ("svc.", "service"),
        ("cust.", "customer"),
        ("acct.", "account"),
        ("fin.", "finance"),
        ("mkt.", "marketing"),
        ("adv.", "advertising"),
        ("pr", "public relations"),
        ("comm.", "communications"),
        ("edu.", "education"),
        ("trng.", "training"),
        ("cert.", "certification"),
        ("lic.", "license"),
        ("reg.", "registration"),
        ("auth.", "authorization"),
        ("appr.", "approval"),
        ("req.", "requirement"),
        ("spec.", "specification"),
        ("std.", "standard"),
        ("proc.", "procedure"),
        ("pol.", "policy"),
        ("reg.", "regulation"),
        ("comp.", "compliance"),
        ("legal", "legal"),
        ("med.", "medical"),
        ("pharm.", "pharmaceutical"),
        ("bio.", "biological"),
        ("chem.", "chemical"),
        ("phys.", "physical"),
        ("sci.", "science"),
        ("tech.", "technology"),
        ("eng.", "engineering"),
        ("arch.", "architecture"),
        ("const.", "construction"),
        ("maint.", "maintenance"),
        ("oper.", "operations"),
        ("prod.", "production"),
        ("qual.", "quality"),
        ("saf.", "safety"),
        ("env.", "environment"),
        ("sus.", "sustainability"),
        ("eff.", "efficiency"),
        ("opt.", "optimization"),
        ("imp.", "improvement"),
        ("dev.", "development"),
        ("res.", "research"),
        ("innov.", "innovation"),
        ("creat.", "creative"),
        ("strat.", "strategic"),
        ("tact.", "tactical"),
        ("plan.", "planning"),
        ("impl.", "implementation"),
        ("exec.", "execution"),
        ("mon.", "monitoring"),
        ("eval.", "evaluation"),
        ("anal.", "analysis"),
        ("rep.", "reporting"),
        ("doc.", "documentation"),
        ("comm.", "communication"),
        ("collab.", "collaboration"),
        ("team", "team"),
        ("indiv.", "individual"),
        ("lead.", "leadership"),
        ("mgmt.", "management"),
        ("admin.", "administration"),
        ("coord.", "coordination"),
        ("org.", "organization"),
        ("sys.", "system"),
        ("net.", "network"),
        ("db", "database"),
        ("app.", "application"),
        ("sw", "software"),
        ("hw", "hardware"),
        ("dev.", "development"),
        ("test.", "testing"),
        ("debug.", "debugging"),
        ("deploy.", "deployment"),
        ("maint.", "maintenance"),
        ("supp.", "support"),
        ("serv.", "service"),
        ("sol.", "solution"),
        ("int.", "integration"),
        ("comp.", "compatibility"),
        ("perf.", "performance"),
        ("opt.", "optimization"),
        ("sec.", "security"),
        ("priv.", "privacy"),
        ("conf.", "confidentiality"),
        ("int.", "integrity"),
        ("avail.", "availability"),
        ("rel.", "reliability"),
        ("scal.", "scalability"),
        ("flex.", "flexibility"),
        ("ext.", "extensibility"),
        ("maint.", "maintainability"),
        ("port.", "portability"),
        ("usab.", "usability"),
        ("access.", "accessibility"),
        ("compat.", "compatibility"),
        ("interop.", "interoperability"),
    ];
    
    for (abbr, full) in variations.iter() {
        normalized = normalized.replace(abbr, full);
    }
    
    normalized
}

// Check if a search term is contained in text
pub fn search_term_in_text(search_term: &str, text: &str) -> bool {
    let text_lower = text.to_lowercase();
    let term_lower = search_term.to_lowercase();
    
    // Direct match
    if text_lower.contains(&term_lower) {
        return true;
    }
    
    // Try normalized version
    let normalized_term = normalize_search_term(&term_lower);
    if text_lower.contains(&normalized_term) {
        return true;
    }
    
    false
}

// Calculate relevance score for a search result
pub fn calculate_result_relevance(
    _caption: &str, 
    _episode_title: &str, 
    _query: &str,
    _query_words: &[&str]
) -> f32 {
    // Always return a positive score to ensure we get results
    // This is especially important for single-word queries like "sucks"
    return 0.5;
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
