use anyhow::Result;
use chrono::{Datelike, NaiveDate};
use regex::Regex;
use reqwest::Client;
use serde_json::Value;
use serenity::all::Http;
use serenity::model::channel::Message;
use tracing::{error, info};

pub async fn handle_aliveordead_command(
    http: &Http,
    msg: &Message,
    celebrity_name: &str,
) -> Result<()> {
    info!("Handling !alive command for celebrity: {}", celebrity_name);

    // Show typing indicator while processing
    if let Err(e) = msg.channel_id.broadcast_typing(http).await {
        error!("Failed to send typing indicator: {:?}", e);
    }

    // Search for the celebrity using the Wikipedia API
    match search_celebrity(celebrity_name).await {
        Ok(Some((result, thumbnail_url))) => {
            // Send the result with an embed if we have a thumbnail
            if let Some(image_url) = thumbnail_url {
                use serenity::builder::CreateEmbed;
                use serenity::builder::CreateMessage;
                let embed = CreateEmbed::new().description(&result).thumbnail(image_url);
                let message = CreateMessage::new().embed(embed);
                if let Err(e) = msg.channel_id.send_message(http, message).await {
                    error!("Error sending celebrity embed: {:?}", e);
                    // Fallback to plain text
                    if let Err(e) = msg.channel_id.say(http, &result).await {
                        error!("Error sending celebrity status: {:?}", e);
                    }
                }
            } else if let Err(e) = msg.channel_id.say(http, result).await {
                error!("Error sending celebrity status: {:?}", e);
                msg.reply(http, "Sorry, I couldn't send the celebrity information.")
                    .await?;
            }
        }
        Ok(None) => {
            msg.reply(
                http,
                format!("Sorry, I couldn't find information about '{celebrity_name}'."),
            )
            .await?;
        }
        Err(e) => {
            error!("Error searching for celebrity: {:?}", e);
            msg.reply(
                http,
                "Sorry, I encountered an error while searching for that celebrity.",
            )
            .await?;
        }
    }

    Ok(())
}

// Function to determine gender and return appropriate pronouns
fn determine_gender(text: &str) -> (&'static str, &'static str, &'static str) {
    // Default to they/them/their
    let mut subject = "they";
    let mut object = "them";
    let mut possessive = "their";

    // Look for gendered pronouns in the text
    let text_lower = text.to_lowercase();

    // Check for male indicators
    if text_lower.contains(" he ")
        || text_lower.contains(" his ")
        || text_lower.contains(" him ")
        || text_lower.contains(" himself ")
        || text_lower.contains(" mr. ")
        || text_lower.contains(" mr ")
        || text_lower.contains(" actor ")
        || text_lower.contains(" father ")
        || text_lower.contains(" son ")
        || text_lower.contains(" brother ")
        || text_lower.contains(" husband ")
        || text_lower.contains(" boyfriend ")
    {
        subject = "he";
        object = "him";
        possessive = "his";
        info!("Gender detection: Male pronouns detected");
    }
    // Check for female indicators
    else if text_lower.contains(" she ")
        || text_lower.contains(" her ")
        || text_lower.contains(" hers ")
        || text_lower.contains(" herself ")
        || text_lower.contains(" mrs. ")
        || text_lower.contains(" mrs ")
        || text_lower.contains(" ms. ")
        || text_lower.contains(" ms ")
        || text_lower.contains(" miss ")
        || text_lower.contains(" actress ")
        || text_lower.contains(" mother ")
        || text_lower.contains(" daughter ")
        || text_lower.contains(" sister ")
        || text_lower.contains(" wife ")
        || text_lower.contains(" girlfriend ")
    {
        subject = "she";
        object = "her";
        possessive = "her";
        info!("Gender detection: Female pronouns detected");
    } else {
        info!("Gender detection: No clear gender indicators, using they/them");
    }

    (subject, object, possessive)
}

// Function to check if the text is about a fictional character
fn is_fictional_character(text: &str, title: &str) -> bool {
    let text_lower = text.to_lowercase();
    let title_lower = title.to_lowercase();

    // Special case for Gary Gygax and other known real people who might be incorrectly flagged
    let known_real_people = [
        "gary gygax",
        "ernest gary gygax",
        "dave arneson",
        "j. r. r. tolkien",
        "tolkien",
        "stan lee",
        "jack kirby",
        "george lucas",
        "gene roddenberry",
        "isaac asimov",
        "stephen king",
        "george r. r. martin",
        "jrr tolkien",
        "j.r.r. tolkien",
    ];

    for person in &known_real_people {
        if title_lower.contains(person) {
            info!(
                "Known real person detected: '{}', not a fictional character",
                person
            );
            return false;
        }
    }

    // Check for explicit "real person" indicators
    let real_person_indicators = [
        "was born",
        "was a",
        "is a",
        "american author",
        "british author",
        "writer",
        "author",
        "creator",
        "inventor",
        "founder",
        "developer",
        "designer",
        "producer",
        "director",
        "businessman",
        "businesswoman",
        "politician",
        "president",
        "prime minister",
        "ceo",
        "executive",
        "scientist",
        "researcher",
        "professor",
        "teacher",
        "educator",
        "artist",
        "musician",
        "composer",
        "singer",
        "actor",
        "actress",
        "athlete",
        "player",
        "coach",
        "manager",
        "born in",
        "died in",
        "graduated from",
        "attended",
        "studied at",
        "worked at",
        "worked for",
    ];

    for indicator in &real_person_indicators {
        if text_lower.contains(indicator) {
            // If we find a real person indicator, this is likely a real person
            // even if the text also mentions fictional characters they created
            info!(
                "Real person indicator found: '{}', not a fictional character",
                indicator
            );
            return false;
        }
    }

    // Common indicators of fictional characters
    let fictional_indicators = [
        "fictional character",
        "fictional protagonist",
        "fictional antagonist",
        "fictional superhero",
        "fictional supervillain",
        "fictional detective",
        "main character",
        "title character",
        "protagonist of",
        "antagonist of",
        "character in the",
        "character from the",
        "appears in",
    ];

    // Check for these indicators
    for indicator in &fictional_indicators {
        if text_lower.contains(indicator) {
            info!("Fictional character indicator found: '{}'", indicator);
            return true;
        }
    }

    // "Created by" is ambiguous - it could refer to a fictional character or a creation by a real person
    // Only count it as fictional if it doesn't look like a real person description
    if text_lower.contains("created by")
        && !text_lower.contains(" born ")
        && !text_lower.contains(" died ")
    {
        // Additional check: if it contains biographical information, it's likely a real person
        if !text_lower.contains("graduated")
            && !text_lower.contains("education")
            && !text_lower.contains("married")
            && !text_lower.contains("career")
        {
            info!("Fictional character indicator 'created by' found without biographical info");
            return true;
        }
    }

    // Check if the title contains common fictional character indicators
    let fictional_title_indicators = [
        "(character)",
        "(fictional character)",
        "(comics)",
        "(Marvel Comics)",
        "(DC Comics)",
        "(Disney)",
        "(film series)",
        "(film character)",
    ];

    for indicator in &fictional_title_indicators {
        if title_lower.contains(indicator) {
            info!("Fictional character indicator in title: '{}'", indicator);
            return true;
        }
    }

    false
}

// Function to find the actor associated with a fictional character
async fn find_actor_for_character(
    text: &str,
    character_name: &str,
    client: &Client,
) -> Result<Option<String>> {
    let text_lower = text.to_lowercase();

    // Special handling for MST3K characters - they're puppets, not portrayed by actors in the traditional sense
    let mst3k_characters = [
        "crow t. robot",
        "tom servo",
        "gypsy",
        "cambot",
        "magic voice",
    ];
    for mst3k_char in &mst3k_characters {
        if character_name.to_lowercase().contains(mst3k_char) {
            info!(
                "MST3K character detected: {}, skipping actor search",
                character_name
            );
            return Ok(None);
        }
    }

    // Look for common patterns that mention actors
    let actor_patterns = [
        r"portrayed by ([^\.]+)",
        r"played by ([^\.]+)",
        r"voiced by ([^\.]+)",
        r"role of [^\.]+? (?:is|was) ([^\.]+)",
        r"actor ([^\.]+) portrays",
        r"actress ([^\.]+) portrays",
        r"actor ([^\.]+) plays",
        r"actress ([^\.]+) plays",
    ];

    for pattern in &actor_patterns {
        if let Ok(re) = Regex::new(pattern) {
            if let Some(captures) = re.captures(&text_lower) {
                if let Some(actor_match) = captures.get(1) {
                    let actor_name = actor_match.as_str().trim();

                    // Skip if the "actor" name contains obvious non-actor indicators
                    let non_actor_indicators = [
                        "who died on",
                        "born",
                        "əs;",
                        "september",
                        "january",
                        "february",
                        "march",
                        "april",
                        "may",
                        "june",
                        "july",
                        "august",
                        "october",
                        "november",
                        "december",
                        "1963",
                        "1964",
                        "1965",
                        "1966",
                        "1967",
                        "1968",
                        "1969",
                        "1970",
                        "1971",
                        "1972",
                        "1973",
                        "1974",
                        "1975",
                        "director",
                        "producer",
                        "writer",
                        "creator",
                        "author",
                        "alex proyas",
                    ];

                    let contains_non_actor = non_actor_indicators
                        .iter()
                        .any(|&indicator| actor_name.to_lowercase().contains(indicator));

                    if contains_non_actor {
                        info!("Skipping invalid actor name: {}", actor_name);
                        continue;
                    }

                    // Clean up the actor name (remove "in the film" etc.)
                    let actor_name = actor_name.split(" in ").next().unwrap_or(actor_name).trim();
                    let actor_name = actor_name.split(" on ").next().unwrap_or(actor_name).trim();
                    let actor_name = actor_name
                        .split(" for ")
                        .next()
                        .unwrap_or(actor_name)
                        .trim();

                    // Additional validation - actor name should be reasonable length and format
                    if actor_name.len() < 3 || actor_name.len() > 50 || actor_name.contains("əs;")
                    {
                        info!("Skipping malformed actor name: {}", actor_name);
                        continue;
                    }

                    info!("Found potential actor: {}", actor_name);

                    // Get information about this actor
                    if let Ok(Some(actor_info)) = search_actor(actor_name, client).await {
                        return Ok(Some(format!(
                            "The character is most famously portrayed by {actor_info}."
                        )));
                    }
                }
            }
        }
    }

    // If we couldn't find an actor in the text, try a direct search
    let search_query = format!("{character_name} actor");
    info!("Trying direct search for actor: {}", search_query);

    // Search for the actor
    let search_url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&list=search&srsearch={}&format=json&srlimit=1",
        urlencoding::encode(&search_query)
    );

    let search_response = client.get(&search_url).send().await?;
    let search_json: Value = search_response.json().await?;

    // Extract the page title from search results
    if let Some(title) = search_json
        .get("query")
        .and_then(|q| q.get("search"))
        .and_then(|s| s.get(0))
        .and_then(|r| r.get("title"))
        .and_then(|t| t.as_str())
    {
        // Check if this looks like an actor's name (not the character again)
        if !title
            .to_lowercase()
            .contains(&character_name.to_lowercase())
        {
            info!("Found potential actor via search: {}", title);

            // Get information about this actor
            if let Ok(Some(actor_info)) = search_actor(title, client).await {
                return Ok(Some(format!(
                    "The character is most famously portrayed by {actor_info}."
                )));
            }
        }
    }

    Ok(None)
}

// Function to search for information about an actor
async fn search_actor(name: &str, client: &Client) -> Result<Option<String>> {
    // Search for the actor's page
    let search_url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&list=search&srsearch={}&format=json&srlimit=1",
        urlencoding::encode(name)
    );

    let search_response = client.get(&search_url).send().await?;
    let search_json: Value = search_response.json().await?;

    // Extract the page title from search results
    let page_title = match search_json
        .get("query")
        .and_then(|q| q.get("search"))
        .and_then(|s| s.get(0))
        .and_then(|r| r.get("title"))
        .and_then(|t| t.as_str())
    {
        Some(title) => title,
        None => {
            info!("No search results found for actor: {}", name);
            return Ok(None);
        }
    };

    // Now get the page content
    let page_url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&prop=extracts&exintro&explaintext&redirects=1&titles={}&format=json",
        urlencoding::encode(page_title)
    );

    let page_response = client.get(&page_url).send().await?;
    let page_json: Value = page_response.json().await?;

    // Extract the page ID
    let pages = match page_json.get("query").and_then(|q| q.get("pages")) {
        Some(p) => p,
        None => {
            info!("No page data found for actor: {}", page_title);
            return Ok(None);
        }
    };

    // Get the first page (there should only be one)
    let page_id = match pages.as_object().and_then(|o| o.keys().next()) {
        Some(id) => id,
        None => {
            info!("No page ID found for actor: {}", page_title);
            return Ok(None);
        }
    };

    // Extract the extract (page content)
    let raw_extract = match pages
        .get(page_id)
        .and_then(|p| p.get("extract"))
        .and_then(|e| e.as_str())
    {
        Some(e) => e,
        None => {
            info!("No extract found for actor page: {}", page_title);
            return Ok(None);
        }
    };

    // Check if this is a person
    let is_person = raw_extract.contains(" born ")
        || raw_extract.contains(" died ")
        || Regex::new(r"\([^)]*\d{4}[^)]*\)")
            .ok()
            .is_some_and(|re| re.is_match(raw_extract));

    if !is_person {
        info!(
            "Actor page doesn't appear to be about a person: {}",
            page_title
        );
        return Ok(None);
    }

    // Determine if the actor is alive or dead
    let (_, death_date, _) = extract_dates_from_parentheses(raw_extract);
    let contains_was = raw_extract.contains(" was ");
    let contains_is = raw_extract.contains(" is ");
    let is_dead = death_date.is_some()
        || (contains_was && !contains_is && raw_extract.contains("(") && raw_extract.contains(")"));

    // Create a brief description of the actor
    let mut actor_info = format!("**{page_title}**");

    if is_dead {
        if let Some(date) = death_date {
            actor_info.push_str(&format!(", who died on {date}"));
        } else {
            actor_info.push_str(", who has passed away");
        }
    } else {
        actor_info.push_str(", who is still alive");
    }

    Ok(Some(actor_info))
}

async fn search_celebrity(name: &str) -> Result<Option<(String, Option<String>)>> {
    const MAX_RETRIES: usize = 5;
    const INITIAL_DELAY_MS: u64 = 1000; // 1 second

    for attempt in 0..MAX_RETRIES {
        match search_celebrity_attempt(name).await {
            Ok(result) => return Ok(result),
            Err(e) => {
                if attempt == MAX_RETRIES - 1 {
                    // Last attempt failed, return the error
                    return Err(e);
                }

                // Calculate exponential backoff delay: 1s, 2s, 4s, 8s, 16s
                let delay_ms = INITIAL_DELAY_MS * (1 << attempt);
                info!(
                    "Wikipedia API attempt {} failed, retrying in {}ms: {:?}",
                    attempt + 1,
                    delay_ms,
                    e
                );

                tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
            }
        }
    }

    unreachable!()
}

async fn search_celebrity_attempt(name: &str) -> Result<Option<(String, Option<String>)>> {
    let client = Client::builder()
        .user_agent("CrowBot/1.0 (https://github.com/mwstowe/crowtdiscordbot)")
        .build()?;

    // First, search for the page - get multiple results to find the best match
    let search_url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&list=search&srsearch={}&format=json&srlimit=5",
        urlencoding::encode(name)
    );

    info!("Searching Wikipedia for: {}", name);
    let search_response = client.get(&search_url).send().await?;

    // Check if we got a successful HTTP response
    if !search_response.status().is_success() {
        error!(
            "Wikipedia API returned HTTP {}: {}",
            search_response.status(),
            search_response
                .status()
                .canonical_reason()
                .unwrap_or("Unknown")
        );
        return Err(anyhow::anyhow!(
            "Wikipedia API returned HTTP {}",
            search_response.status()
        ));
    }

    // Get response text first to log it if JSON parsing fails
    let response_text = search_response.text().await?;
    let search_json: Value = match serde_json::from_str(&response_text) {
        Ok(json) => json,
        Err(e) => {
            error!(
                "Failed to parse Wikipedia search response as JSON. Response was: {}",
                response_text.chars().take(200).collect::<String>()
            );
            return Err(anyhow::anyhow!("JSON parsing failed: {}", e));
        }
    };

    // Extract search results and find the best person match
    let search_results = match search_json
        .get("query")
        .and_then(|q| q.get("search"))
        .and_then(|s| s.as_array())
    {
        Some(results) => results,
        None => {
            info!("No search results found for: {}", name);
            return Ok(None);
        }
    };

    if search_results.is_empty() {
        info!("No search results found for: {}", name);
        return Ok(None);
    }

    // Try each result, preferring ones that look like real people
    // First: prefer a result whose title exactly matches the search query (case-insensitive)
    let mut best_title: Option<&str> = None;
    let name_lower = name.to_lowercase();
    for result in search_results {
        let title = result.get("title").and_then(|t| t.as_str()).unwrap_or("");
        if title.to_lowercase() == name_lower {
            info!("Found exact title match: {}", title);
            best_title = Some(title);
            break;
        }
    }

    // Second pass: look for a result whose snippet contains biographical indicators
    if best_title.is_none() {
        for result in search_results {
            let title = result.get("title").and_then(|t| t.as_str()).unwrap_or("");
            let snippet = result
                .get("snippet")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_lowercase();

            // Prefer results with biographical indicators in the snippet
            let has_bio_indicator = snippet.contains("born")
                || snippet.contains("died")
                || snippet.contains("was a")
                || snippet.contains("is a")
                || snippet.contains("writer")
                || snippet.contains("author")
                || snippet.contains("actor")
                || snippet.contains("musician")
                || snippet.contains("politician")
                || snippet.contains("scientist")
                || snippet.contains("professional")
                || snippet.contains("athlete")
                || snippet.contains("singer")
                || snippet.contains("comedian")
                || snippet.contains("director")
                || snippet.contains("engineer")
                || snippet.contains("businessman");

            // Skip results that look like fictional characters
            let looks_fictional = snippet.contains("fictional character")
                || snippet.contains("character in")
                || title.contains("(character)")
                || title.contains("(film)");

            if has_bio_indicator && !looks_fictional {
                info!("Found biographical result: {}", title);
                best_title = Some(title);
                break;
            }
        }
    }

    // Fall back to first result if no biographical match found
    let page_title = match best_title {
        Some(title) => title,
        None => match search_results
            .first()
            .and_then(|r| r.get("title"))
            .and_then(|t| t.as_str())
        {
            Some(title) => title,
            None => {
                info!("No search results found for: {}", name);
                return Ok(None);
            }
        },
    };

    info!("Found Wikipedia page: {}", page_title);

    // Now get the page content (including thumbnail image)
    let page_url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&prop=extracts|pageprops|pageimages&exintro&explaintext&redirects=1&pithumbsize=300&titles={}&format=json",
        urlencoding::encode(page_title)
    );

    let page_response = client.get(&page_url).send().await?;

    // Check if we got a successful HTTP response
    if !page_response.status().is_success() {
        error!(
            "Wikipedia page API returned HTTP {}: {}",
            page_response.status(),
            page_response
                .status()
                .canonical_reason()
                .unwrap_or("Unknown")
        );
        return Err(anyhow::anyhow!(
            "Wikipedia page API returned HTTP {}",
            page_response.status()
        ));
    }

    // Get response text first to log it if JSON parsing fails
    let page_response_text = page_response.text().await?;
    let page_json: Value = match serde_json::from_str(&page_response_text) {
        Ok(json) => json,
        Err(e) => {
            error!(
                "Failed to parse Wikipedia page response as JSON. Response was: {}",
                page_response_text.chars().take(200).collect::<String>()
            );
            return Err(anyhow::anyhow!("JSON parsing failed: {}", e));
        }
    };

    // Extract the page ID
    let pages = match page_json.get("query").and_then(|q| q.get("pages")) {
        Some(p) => p,
        None => {
            info!("No page data found for: {}", page_title);
            return Ok(None);
        }
    };

    // Get the first page (there should only be one)
    let page_id = match pages.as_object().and_then(|o| o.keys().next()) {
        Some(id) => id,
        None => {
            info!("No page ID found for: {}", page_title);
            return Ok(None);
        }
    };

    // Extract the extract (page content)
    let raw_extract = match pages
        .get(page_id)
        .and_then(|p| p.get("extract"))
        .and_then(|e| e.as_str())
    {
        Some(e) => e,
        None => {
            info!("No extract found for page: {}", page_title);
            return Ok(None);
        }
    };

    // Extract thumbnail URL if available
    let thumbnail_url = pages
        .get(page_id)
        .and_then(|p| p.get("thumbnail"))
        .and_then(|t| t.get("source"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());

    if let Some(ref url) = thumbnail_url {
        info!("Found thumbnail: {}", url);
    }

    // Log only the first 100 characters of the extract for debugging
    if raw_extract.len() > 100 {
        let preview_end = raw_extract
            .char_indices()
            .nth(100)
            .map_or(raw_extract.len(), |(i, _)| i);
        info!(
            "Extract preview (first 100 chars): {}...",
            &raw_extract[..preview_end]
        );
    } else {
        info!("Extract preview: {}", raw_extract);
    }

    // Check if this is a fictional character
    let is_fictional = is_fictional_character(raw_extract, page_title);

    if is_fictional {
        info!("Detected fictional character: {}", page_title);

        // Try to find the actor associated with this character
        if let Some(actor_info) = find_actor_for_character(raw_extract, page_title, &client).await?
        {
            return Ok(Some((
                format!("**{page_title}** is a fictional character. {actor_info}"),
                thumbnail_url.clone(),
            )));
        } else {
            return Ok(Some((
                format!("**{page_title}** is a fictional character, not a real person."),
                thumbnail_url.clone(),
            )));
        }
    }

    // Get Wikidata ID from pageprops
    let wikidata_id = pages
        .get(page_id)
        .and_then(|p| p.get("pageprops"))
        .and_then(|pp| pp.get("wikibase_item"))
        .and_then(|w| w.as_str());

    // Fetch structured data from Wikidata
    let wikidata = if let Some(qid) = wikidata_id {
        fetch_wikidata_person_info(&client, qid).await
    } else {
        None
    };

    // If we have Wikidata info with a birth date, it's definitely a person
    // Otherwise fall back to text heuristics
    let is_person = wikidata
        .as_ref()
        .is_some_and(|w| w.birth_date.is_some() || w.death_date.is_some())
        || raw_extract.contains(" born ")
        || raw_extract.contains(" died ")
        || Regex::new(r"\([^)]*\d{4}[^)]*\)")
            .ok()
            .is_some_and(|re| re.is_match(raw_extract));

    if !is_person {
        info!("Page doesn't appear to be about a person: {}", page_title);
        return Ok(Some((
            format!(
                "I found information about '{page_title}', but it doesn't appear to be a person."
            ),
            thumbnail_url.clone(),
        )));
    }

    // Determine gender for proper pronoun usage
    let (subject_pronoun, _object_pronoun, possessive_pronoun) = determine_gender(raw_extract);
    info!("Using pronouns: {}/{}", subject_pronoun, possessive_pronoun);

    // Get a short description (first two sentences) from Wikipedia extract
    let (_, _, cleaned_extract) = extract_dates_from_parentheses(raw_extract);
    let raw_parts: Vec<&str> = cleaned_extract.split('.').collect();
    let mut sentences: Vec<String> = Vec::new();
    let mut current = String::new();

    for part in &raw_parts {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        if current.is_empty() {
            current = trimmed.to_string();
        } else {
            let last_word = current.split_whitespace().last().unwrap_or("");
            let ends_with_initial = last_word.len() <= 2
                && last_word
                    .chars()
                    .all(|c| c.is_uppercase() || c.is_ascii_digit());

            if ends_with_initial {
                current.push_str(&format!(". {trimmed}"));
            } else {
                current.push('.');
                sentences.push(current);
                current = trimmed.to_string();
                if sentences.len() >= 2 {
                    break;
                }
            }
        }
    }
    if !current.is_empty() && sentences.len() < 2 {
        sentences.push(current);
    }
    let description = sentences.join(" ").trim().to_string();

    let mut response = format!("**{page_title}**: {description}");

    // Capitalize the subject pronoun for sentence starts
    let cap_pronoun = subject_pronoun
        .chars()
        .next()
        .unwrap()
        .to_uppercase()
        .to_string()
        + &subject_pronoun[1..];

    // Use Wikidata for alive/dead determination and dates
    // Trim trailing period to avoid double periods when appending
    let response_trimmed = response.trim_end_matches('.').trim_end().to_string();
    response = response_trimmed;

    if let Some(ref wd) = wikidata {
        if wd.is_dead {
            // Person is dead
            if let Some(ref death_date) = wd.death_date {
                if let Some(ref birth_date) = wd.birth_date {
                    // Calculate age at death
                    if let (Some(birth), Some(death)) =
                        (parse_date(birth_date), parse_date(death_date))
                    {
                        let age = calculate_age(birth, death);
                        response.push_str(&format!(
                            ". {cap_pronoun} died on {death_date} at the age of {age}."
                        ));
                    } else {
                        response.push_str(&format!(". {cap_pronoun} died on {death_date}."));
                    }
                } else {
                    response.push_str(&format!(". {cap_pronoun} died on {death_date}."));
                }
            } else {
                response.push_str(&format!(". {cap_pronoun} has died."));
            }

            if let Some(ref cause) = wd.cause_of_death {
                response.push_str(&format!(" Cause of death: {cause}."));
            } else if let Some(cause) = extract_cause_of_death(raw_extract) {
                response.push_str(&format!(" Cause of death: {cause}."));
            }
        } else {
            // Person is alive
            if let Some(ref birth_date) = wd.birth_date {
                if let Some(birth) = parse_date(birth_date) {
                    let today = chrono::Local::now().naive_local().date();
                    let age = calculate_age(birth, today);
                    response.push_str(&format!(
                        ". {cap_pronoun} is still alive at {age} years old."
                    ));
                } else {
                    response.push_str(&format!(". {cap_pronoun} is still alive."));
                }
            } else {
                response.push_str(&format!(". {cap_pronoun} is still alive."));
            }
        }
    } else {
        // No Wikidata — fall back to text-based heuristics
        let (birth_date, death_date, _) = extract_dates_from_parentheses(raw_extract);

        if death_date.is_some()
            || raw_extract.to_lowercase().contains(" died ")
            || (raw_extract.contains(" was ") && !raw_extract.contains(" is "))
        {
            if let Some(ref date) = death_date {
                response.push_str(&format!(". {cap_pronoun} died on {date}."));
            } else {
                response.push_str(&format!(". {cap_pronoun} has died."));
            }
            if let Some(cause) = extract_cause_of_death(raw_extract) {
                response.push_str(&format!(" Cause of death: {cause}."));
            }
        } else if let Some(ref date) = birth_date {
            if let Some(birth) = parse_date(date) {
                let today = chrono::Local::now().naive_local().date();
                let age = calculate_age(birth, today);
                response.push_str(&format!(
                    ". {cap_pronoun} is still alive at {age} years old."
                ));
            } else {
                response.push_str(&format!(". {cap_pronoun} is still alive."));
            }
        } else {
            response.push_str(&format!(
                ". I couldn't determine if {subject_pronoun} is alive or dead."
            ));
        }
    }

    Ok(Some((response, thumbnail_url.clone())))
}

pub fn extract_dates_from_parentheses(text: &str) -> (Option<String>, Option<String>, String) {
    // Check for sequential parentheses pattern like "(born Kenneth Donald Rogers) (August 21, 1938 – March 20, 2020)"
    // First, find all parenthetical sections in the text
    let mut parentheses_sections = Vec::new();
    let mut start_idx = 0;

    while let Some(open_idx) = text[start_idx..].find('(') {
        let open_pos = start_idx + open_idx;
        let mut depth = 1;
        let mut close_pos = None;

        for (i, c) in text[open_pos + 1..].char_indices() {
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                depth -= 1;
                if depth == 0 {
                    close_pos = Some(open_pos + 1 + i);
                    break;
                }
            }
        }

        if let Some(close_idx) = close_pos {
            parentheses_sections.push((open_pos, close_idx, &text[open_pos..=close_idx]));
            start_idx = close_idx + 1;
        } else {
            break; // No matching closing parenthesis found
        }
    }

    // Look for a section that contains birth-death dates (has a year range with dash)
    let year_regex = Regex::new(r"\d{4}").unwrap();
    for (_, _, section) in &parentheses_sections {
        // Remove the outer parentheses
        let content = &section[1..section.len() - 1];

        // Check if this contains a date range with a dash or en-dash
        if content.contains('–') || content.contains('-') {
            let separator = if content.contains('–') { '–' } else { '-' };
            let parts: Vec<&str> = content.split(separator).collect();

            if parts.len() == 2 {
                let birth_part = parts[0].trim();
                let death_part = parts[1].trim();

                // Check if both parts look like dates (contain years)
                if year_regex.is_match(birth_part) && year_regex.is_match(death_part) {
                    // Check if this is likely a birth-death range rather than a band membership or other date range
                    // Birth-death ranges typically:
                    // 1. Are near the beginning of the text (in the first 100 characters)
                    // 2. Are not preceded by words like "band", "group", "member", "career", etc.
                    // 3. Often have month/day information or are just years (1946-2021)

                    // Calculate position in text
                    let section_pos = section.as_ptr() as usize - text.as_ptr() as usize;
                    let is_near_beginning = section_pos < 100;

                    // Get preceding text to check for band/career indicators
                    let start_pos = section_pos.saturating_sub(30);
                    let preceding_text = &text[start_pos..section_pos];
                    let preceding_text_lower = preceding_text.to_lowercase();

                    // Define band/career indicators
                    let band_indicators = [
                        "band",
                        "group",
                        "member",
                        "career",
                        "tour",
                        "album",
                        "record",
                        "release",
                        "project",
                        "formed",
                        "founded",
                        "joined",
                        "left",
                        "played with",
                        "performed with",
                        "worked with",
                        "collaborated",
                        "session",
                        "studio",
                    ];

                    // Check if any indicators are present
                    let has_band_indicator = band_indicators
                        .iter()
                        .any(|&indicator| preceding_text_lower.contains(indicator));

                    // If it's near the beginning and doesn't have band indicators, it's likely birth-death
                    let is_likely_birth_death = is_near_beginning && !has_band_indicator;

                    if is_likely_birth_death {
                        // Create cleaned text without this parenthetical section
                        let mut cleaned_text = text.to_string();
                        for (start, end, sect) in &parentheses_sections {
                            if sect == section {
                                cleaned_text = format!("{}{}", &text[0..*start], &text[*end + 1..]);
                                break;
                            }
                        }
                        cleaned_text = cleaned_text.replace("  ", " ").trim().to_string();

                        info!(
                            "Found birth-death dates in parentheses: {} - {}",
                            birth_part, death_part
                        );
                        return (
                            Some(birth_part.to_string()),
                            Some(death_part.to_string()),
                            cleaned_text,
                        );
                    } else {
                        info!(
                            "Found date range but it appears to be for a band/career: {} - {}",
                            birth_part, death_part
                        );
                    }
                }
            }
        }
    }

    // If we didn't find a date range in any parenthetical section, try the original approach
    // Find the first opening parenthesis
    if let Some(open_paren_pos) = text.find('(') {
        // Find the matching closing parenthesis
        let mut depth = 1;
        let mut close_paren_pos = None;

        for (i, c) in text[open_paren_pos + 1..].char_indices() {
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                depth -= 1;
                if depth == 0 {
                    close_paren_pos = Some(open_paren_pos + 1 + i);
                    break;
                }
            }
        }

        if let Some(close_pos) = close_paren_pos {
            // Extract the entire parenthetical section
            let paren_section = &text[open_paren_pos..=close_pos];

            // Look for dates within this section
            let date_regex = Regex::new(r"(\w+ \d{1,2}, \d{4})").unwrap();
            let mut dates = Vec::new();

            for date_match in date_regex.find_iter(paren_section) {
                dates.push(date_match.as_str().to_string());
            }

            if dates.len() >= 2 {
                // Create cleaned text without the parentheses
                let mut cleaned_text =
                    format!("{}{}", &text[0..open_paren_pos], &text[close_pos + 1..]);
                cleaned_text = cleaned_text.replace("  ", " ").trim().to_string();

                return (Some(dates[0].clone()), Some(dates[1].clone()), cleaned_text);
            }
        }
    }

    // If the direct approach didn't work, fall back to the regex approach
    let re = Regex::new(r"^(.*?)\(([^)]+)\)(.*)$").unwrap();

    if let Some(captures) = re.captures(text) {
        let before = captures.get(1).map_or("", |m| m.as_str());
        let parentheses_content = captures.get(2).map_or("", |m| m.as_str());
        let after = captures.get(3).map_or("", |m| m.as_str());

        // Create cleaned text without the parentheses
        // Remove any double spaces that might be created when removing parentheses
        let mut cleaned_text = format!("{before}{after}");
        cleaned_text = cleaned_text.replace("  ", " ");

        // Direct check for birth-death date format
        if parentheses_content.contains('–') || parentheses_content.contains('-') {
            let separator = if parentheses_content.contains('–') {
                '–'
            } else {
                '-'
            };
            let parts: Vec<&str> = parentheses_content.split(separator).collect();

            if parts.len() == 2 {
                let birth_part = parts[0].trim();
                let death_part = parts[1].trim();

                // Check if both parts look like dates (contain years)
                let year_regex = Regex::new(r"\d{4}").unwrap();
                if year_regex.is_match(birth_part) && year_regex.is_match(death_part) {
                    return (
                        Some(birth_part.to_string()),
                        Some(death_part.to_string()),
                        cleaned_text,
                    );
                }
            }
        }

        // If direct extraction didn't work, try the more complex patterns
        let birth_date = extract_year_from_parentheses(parentheses_content, "born");
        let death_date = extract_year_from_parentheses(parentheses_content, "died");

        return (birth_date, death_date, cleaned_text);
    }

    // If no parentheses found, return the original text
    (None, None, text.to_string())
}

pub fn extract_year_from_parentheses(text: &str, date_type: &str) -> Option<String> {
    // Common patterns in Wikipedia parentheses
    // Examples: "born January 20, 1930", "20 January 1930 – 15 April 2023"

    info!("Extracting {} date from parentheses: '{}'", date_type, text);

    if date_type == "born" {
        // Look for birth date
        // Pattern: "born January 20, 1930" or just a date at the beginning
        let born_re = Regex::new(r"(?:born|b\.)\s+([A-Za-z]+\s+\d{1,2},?\s+\d{4})").unwrap();
        info!("Trying born regex pattern: {:?}", born_re.as_str());

        if let Some(captures) = born_re.captures(text) {
            let date = captures.get(1).map(|m| m.as_str().to_string());
            info!("Found birth date with born regex: {:?}", date);
            return date;
        } else {
            info!("Born regex did not match");
        }

        // If there's a dash, the birth date is likely before it
        if text.contains('–') || text.contains('-') {
            let parts: Vec<&str> = text.split(['–', '-']).collect();
            if !parts.is_empty() {
                let potential_date = parts[0].trim();
                // Check if it looks like a date (contains a year)
                if Regex::new(r"\d{4}").unwrap().is_match(potential_date) {
                    return Some(potential_date.to_string());
                }
            }
        }
    } else if date_type == "died" {
        // Look for death date
        // Pattern: "died April 15, 2023" or date after a dash
        let died_re = Regex::new(r"(?:died|d\.)\s+([A-Za-z]+\s+\d{1,2},?\s+\d{4})").unwrap();
        if let Some(captures) = died_re.captures(text) {
            let date = captures.get(1).map(|m| m.as_str().to_string());
            return date;
        }

        // If there's a dash, the death date is likely after it
        if text.contains('–') || text.contains('-') {
            let parts: Vec<&str> = text.split(['–', '-']).collect();
            if parts.len() > 1 {
                let potential_date = parts[1].trim();
                // Check if it looks like a date (contains a year)
                if Regex::new(r"\d{4}").unwrap().is_match(potential_date) {
                    return Some(potential_date.to_string());
                }
            }
        }

        // Special case for future dates - if the year is greater than current year
        let current_year = chrono::Local::now().year();
        let future_year_re =
            Regex::new(&format!(r"(\w+\s+\d{{1,2}},?\s+({current_year}-\d{{4}}))")).unwrap();
        if let Some(captures) = future_year_re.captures(text) {
            if let Some(date_match) = captures.get(1) {
                let date = date_match.as_str().to_string();
                return Some(date);
            }
        }
    }

    info!("No {} date found in parentheses", date_type);
    None
}

fn parse_date(date_str: &str) -> Option<NaiveDate> {
    info!("Attempting to parse date string: '{}'", date_str);

    // Try various date formats
    let formats = [
        "%d %B %Y",  // 20 April 2023
        "%B %d, %Y", // April 20, 2023
        "%Y-%m-%d",  // 2023-04-20
        "%B %Y",     // April 2023
        "%d %b %Y",  // 20 Apr 2023
        "%b %d, %Y", // Apr 20, 2023
    ];

    for format in &formats {
        match NaiveDate::parse_from_str(date_str, format) {
            Ok(date) => {
                info!(
                    "Successfully parsed '{}' with format '{}' as {}",
                    date_str, format, date
                );
                return Some(date);
            }
            Err(e) => {
                info!(
                    "Failed to parse '{}' with format '{}': {}",
                    date_str, format, e
                );
            }
        }
    }

    info!("Failed to parse date string '{}' with any format", date_str);
    None
}

// Function to extract cause of death from text
/// Structured person data from Wikidata
struct WikidataPersonInfo {
    birth_date: Option<String>,
    death_date: Option<String>,
    cause_of_death: Option<String>,
    is_dead: bool,
}

/// Fetch person info from Wikidata (P569=birth, P570=death, P509=cause of death)
async fn fetch_wikidata_person_info(client: &Client, qid: &str) -> Option<WikidataPersonInfo> {
    let url = format!(
        "https://www.wikidata.org/w/api.php?action=wbgetclaims&entity={}&format=json",
        qid
    );
    info!("Fetching Wikidata claims for {}", qid);
    let response = client.get(&url).send().await.ok()?;
    let json: Value = response.json().await.ok()?;

    let claims = json.get("claims")?;

    // Extract birth date (P569)
    let birth_date = claims
        .pointer("/P569/0/mainsnak/datavalue/value/time")
        .and_then(|v| v.as_str())
        .and_then(format_wikidata_date);

    // Extract death date (P570)
    let death_date = claims
        .pointer("/P570/0/mainsnak/datavalue/value/time")
        .and_then(|v| v.as_str())
        .and_then(format_wikidata_date);

    let is_dead = claims
        .get("P570")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty());

    // Extract cause of death (P509) - points to another entity
    let cause_of_death = if let Some(cause_id) = claims
        .pointer("/P509/0/mainsnak/datavalue/value/id")
        .and_then(|v| v.as_str())
    {
        fetch_wikidata_label(client, cause_id).await
    } else {
        None
    };

    info!(
        "Wikidata result: birth={:?}, death={:?}, cause={:?}, is_dead={}",
        birth_date, death_date, cause_of_death, is_dead
    );

    Some(WikidataPersonInfo {
        birth_date,
        death_date,
        cause_of_death,
        is_dead,
    })
}

/// Convert Wikidata time format (+1954-02-21T00:00:00Z) to readable date
fn format_wikidata_date(time_str: &str) -> Option<String> {
    // Format is like "+1954-02-21T00:00:00Z"
    let date_part = time_str.trim_start_matches('+').split('T').next()?;
    let date = NaiveDate::parse_from_str(date_part, "%Y-%m-%d").ok()?;
    Some(date.format("%B %-d, %Y").to_string())
}

/// Fetch the English label for a Wikidata entity
async fn fetch_wikidata_label(client: &Client, entity_id: &str) -> Option<String> {
    let url = format!(
        "https://www.wikidata.org/w/api.php?action=wbgetentities&ids={}&props=labels&languages=en&format=json",
        entity_id
    );
    let response = client.get(&url).send().await.ok()?;
    let json: Value = response.json().await.ok()?;

    let label = json
        .pointer(&format!("/entities/{}/labels/en/value", entity_id))
        .and_then(|v| v.as_str())?;

    let mut result = label.to_string();
    if let Some(first) = result.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    Some(result)
}

// Function to extract cause of death from text
fn extract_cause_of_death(text: &str) -> Option<String> {
    info!("Attempting to extract cause of death from text");

    // First, split the text into sentences for better context
    let sentences: Vec<&str> = text
        .split(['.', '!', '?'])
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    // First check for explicit "Cause of death:" format
    for sentence in &sentences {
        if sentence.to_lowercase().contains("cause of death:") {
            info!("Found explicit 'Cause of death:' statement");
            let parts: Vec<&str> = sentence.split("Cause of death:").collect();
            if parts.len() > 1 {
                let cause = parts[1].trim();
                // Validate that the cause makes sense (not just a name or nonsensical phrase)
                if !cause.is_empty() && cause.len() < 50 {
                    // Check against common invalid causes
                    let invalid_causes = [
                        "arrival",
                        "departure",
                        "birth",
                        "name",
                        "career",
                        "life",
                        "music",
                        "work",
                        "legacy",
                        "influence",
                        "style",
                        "technique",
                        "composition",
                        "performance",
                        "concert",
                        "tour",
                        "album",
                        "recording",
                        "release",
                        "publication",
                        "book",
                        "novel",
                        "story",
                        "film",
                        "movie",
                        "show",
                        "series",
                        "episode",
                        "season",
                        "chapter",
                        "part",
                        "volume",
                        "edition",
                    ];

                    let is_invalid = invalid_causes
                        .iter()
                        .any(|&invalid| cause.to_lowercase().contains(invalid));

                    if is_invalid {
                        info!("Skipping invalid cause of death: {}", cause);
                        continue;
                    }

                    // Check if the cause contains common medical terms or valid causes
                    let valid_cause_indicators = [
                        "disease",
                        "cancer",
                        "failure",
                        "attack",
                        "infection",
                        "cardiac",
                        "arrest",
                        "injury",
                        "wound",
                        "trauma",
                        "suicide",
                        "accident",
                        "complications",
                    ];
                    let is_likely_valid = valid_cause_indicators
                        .iter()
                        .any(|&valid| cause.to_lowercase().contains(valid));

                    if is_likely_valid {
                        info!("Found valid cause of death: {}", cause);
                        return Some(cause.to_string());
                    } else {
                        info!(
                            "Cause doesn't contain common medical terms, continuing search: {}",
                            cause
                        );
                    }
                }
            }
        }
    }

    // Look for sentences that mention death and might contain cause information
    let death_sentences: Vec<&str> = sentences
        .iter()
        .filter(|s| {
            let s_lower = s.to_lowercase();
            s_lower.contains("died")
                || s_lower.contains("death")
                || s_lower.contains("passed away")
                || s_lower.contains("was killed")
                || s_lower.contains("were killed")
                || s_lower.contains("succumbed")
                || s_lower.contains("lost his battle")
                || s_lower.contains("lost her battle")
        })
        .copied()
        .collect();

    if death_sentences.is_empty() {
        info!("No sentences mentioning death found");
        return None;
    }

    // Common patterns for cause of death - expanded with more variations
    let patterns = [
        // Direct cause patterns
        r"died (?:of|from|due to|after|following) ([^\.;:,]+)",
        r"death (?:was caused by|was due to|from|by) ([^\.;:,]+)",
        r"died .{0,30}? (?:of|from|due to|after|following) ([^\.;:,]+)",
        r"passed away (?:from|due to|after|following) ([^\.;:,]+)",
        r"succumbed to ([^\.;:,]+)",
        r"lost (?:his|her|their) (?:battle|fight|struggle) with ([^\.;:,]+)",
        r"died .{0,50}? complications (?:of|from) ([^\.;:,]+)",
        r"cause of death was ([^\.;:,]+)",
        r"death was attributed to ([^\.;:,]+)",
        r"died as a result of ([^\.;:,]+)",
        r"died because of ([^\.;:,]+)",
        r"death resulted from ([^\.;:,]+)",
        r"was killed in ([^\.;:,]+)",
        r"were killed in ([^\.;:,]+)",
        r"was killed by ([^\.;:,]+)",
        r"killed in ([^\.;:,]+?) (?:in|on|near|at)",
    ];

    // First try with death-related sentences for better context
    for sentence in &death_sentences {
        let sentence_lower_check = sentence.to_lowercase();

        // Skip sentences where the cause is explicitly unknown or disputed
        if sentence_lower_check.contains("remains unknown")
            || sentence_lower_check.contains("cause of death is unknown")
            || sentence_lower_check.contains("cause of death remains")
            || sentence_lower_check.contains("has been attributed to many")
            || sentence_lower_check.contains("never been determined")
            || sentence_lower_check.contains("subject of debate")
            || sentence_lower_check.contains("disputed")
        {
            info!(
                "Skipping sentence with unknown/disputed cause: {}",
                sentence
            );
            continue;
        }

        for pattern in &patterns {
            if let Ok(re) = Regex::new(pattern) {
                if let Some(captures) = re.captures(sentence) {
                    if let Some(cause_match) = captures.get(1) {
                        let mut cause = cause_match.as_str().trim().to_string();

                        // Skip if the cause is too long (likely not a real cause)
                        if cause.len() > 50 {
                            info!("Skipping cause that's too long: {}", cause);
                            continue;
                        }

                        // Skip if the cause contains phrases that indicate it's not actually a cause of death
                        let false_indicators = [
                            "until his death",
                            "until her death",
                            "until their death",
                            "before his death",
                            "before her death",
                            "before their death",
                            "prior to his death",
                            "prior to her death",
                            "prior to their death",
                            "at the time of",
                            "at the age of",
                            "career",
                            "professional",
                            "embarking",
                            "released",
                            "musician",
                            "album",
                            "single",
                            "art",
                            "painting",
                            "moving to",
                            "living in",
                            "working",
                            "studying",
                            "attending",
                            "graduating",
                            "known for",
                            "famous for",
                            "contributed",
                            "considered",
                            "influenced",
                        ];

                        let is_false_positive = false_indicators
                            .iter()
                            .any(|&indicator| cause.to_lowercase().contains(indicator));

                        if is_false_positive {
                            info!("Skipping false positive cause: {}", cause);
                            continue;
                        }

                        // Capitalize first letter
                        if !cause.is_empty() {
                            let first_char = cause
                                .chars()
                                .next()
                                .unwrap()
                                .to_uppercase()
                                .collect::<String>();
                            if cause.len() > 1 {
                                cause = first_char + &cause[1..];
                            } else {
                                cause = first_char;
                            }
                        }

                        info!("Found cause of death in death-related sentence: {}", cause);
                        return Some(cause);
                    }
                }
            }
        }
    }

    // If we still haven't found a cause, try to find specific diseases or conditions
    // that are commonly causes of death
    let common_causes = [
        "cancer",
        "heart attack",
        "stroke",
        "liver failure",
        "kidney failure",
        "respiratory failure",
        "pneumonia",
        "COVID-19",
        "coronavirus",
        "suicide",
        "accident",
        "complications",
        "heart failure",
        "cardiac arrest",
        "heart disease",
        "lung cancer",
        "brain cancer",
        "leukemia",
        "AIDS",
        "HIV",
        "overdose",
        "drug overdose",
        "alcohol",
        "cirrhosis",
        "alzheimer",
        "parkinson",
    ];

    for sentence in &death_sentences {
        let sentence_lower = sentence.to_lowercase();

        // Skip sentences where the cause is explicitly unknown or disputed
        if sentence_lower.contains("remains unknown")
            || sentence_lower.contains("cause of death is unknown")
            || sentence_lower.contains("cause of death remains")
            || sentence_lower.contains("has been attributed to many")
            || sentence_lower.contains("never been determined")
            || sentence_lower.contains("subject of debate")
            || sentence_lower.contains("disputed")
        {
            info!(
                "Skipping sentence with unknown/disputed cause: {}",
                sentence
            );
            continue;
        }

        for cause in &common_causes {
            if sentence_lower.contains(cause) {
                // Get the context around the cause
                if let Some(pos) = sentence_lower.find(cause) {
                    // Get a window of text around the cause
                    let start = pos.saturating_sub(10);
                    let end = (pos + cause.len() + 20).min(sentence.len());
                    let context = &sentence[start..end];

                    // Extract just the cause and nearby words
                    let cause_with_context = extract_cause_with_context(context, cause);

                    info!(
                        "Found common cause '{}' in death sentence",
                        cause_with_context
                    );
                    return Some(cause_with_context);
                }
            }
        }
    }

    info!("No cause of death found");
    None
}

// Helper function to extract a cause with some context
fn extract_cause_with_context(text: &str, cause: &str) -> String {
    let text_lower = text.to_lowercase();
    if let Some(pos) = text_lower.find(cause) {
        // Find the start of the phrase (after prepositions like "from", "of", etc.)
        let prepositions = ["from ", "of ", "with ", "due to ", "by ", "to "];
        let mut start = 0;
        for prep in &prepositions {
            if let Some(prep_pos) = text_lower[..pos].rfind(prep) {
                start = prep_pos + prep.len();
                break;
            }
        }

        // Find the end of the phrase (before punctuation or conjunctions)
        let mut end = text.len();
        let end_markers = [",", ";", ".", "and ", "but ", "which ", "when ", "while "];
        for marker in &end_markers {
            if let Some(marker_pos) = text_lower[pos..].find(marker) {
                end = pos + marker_pos;
                break;
            }
        }

        // Extract and clean up the phrase
        let mut result = text[start..end].trim().to_string();

        // Capitalize first letter
        if !result.is_empty() {
            let first_char = result
                .chars()
                .next()
                .unwrap()
                .to_uppercase()
                .collect::<String>();
            if result.len() > 1 {
                result = first_char + &result[1..];
            } else {
                result = first_char;
            }
        }

        return result;
    }

    // If we couldn't extract context, just return the cause capitalized
    cause
        .chars()
        .next()
        .unwrap()
        .to_uppercase()
        .collect::<String>()
        + &cause[1..]
}
fn calculate_age(birth_date: NaiveDate, today: NaiveDate) -> u32 {
    let mut age = today.year() - birth_date.year();

    info!(
        "Age calculation: birth_date={}, today={}, initial_age={}",
        birth_date, today, age
    );

    // Adjust age if birthday hasn't occurred yet this year
    if today.month() < birth_date.month()
        || (today.month() == birth_date.month() && today.day() < birth_date.day())
    {
        age -= 1;
        info!(
            "Birthday hasn't occurred yet this year, adjusted age to {}",
            age
        );
    }

    let final_age = age as u32;
    info!("Final calculated age: {}", final_age);
    final_age
}
