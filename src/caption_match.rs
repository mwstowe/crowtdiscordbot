//! Matching search queries against caption text, shared by the Frinkiac-family clients
//! (Frinkiac, Morbotron).
//!
//! The sites' search APIs are fuzzy: the top hit can share no words with the query
//! (Frinkiac returned "Testing." for "thrusting"). These helpers re-rank the hits on what
//! the caption actually says, so a client can pick the caption that matches best, or
//! decline when nothing does.

/// How good the best search candidate must be before it is accepted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MatchMode {
    /// A multi-word query needs the whole phrase; a single word needs a caption
    /// containing that word.
    Strict,
    /// The best candidate must contain at least `MIN_LOOSE_COVERAGE` of the query words.
    Loose,
    /// Accept the best-ranked candidate regardless of overlap (random search terms).
    Any,
}

/// Minimum fraction of a query's significant words a candidate must contain in `Loose` mode.
pub(crate) const MIN_LOOSE_COVERAGE: f64 = 0.5;

/// Words ignored when measuring overlap, so "the" or "is" can't carry a weak match.
const QUERY_STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "for", "in", "is", "it", "of", "on", "or",
    "the", "to", "was", "with",
];

/// A search result scored against the query.
pub(crate) struct Candidate<T> {
    pub item: T,
    /// The query has several words and the caption contains them as a whole-word run,
    /// ignoring case and punctuation.
    pub phrase_match: bool,
    /// Fraction (0.0-1.0) of the query's significant words found in the caption.
    pub coverage: f64,
    /// Tightness of the phrase match; 0.0 when `phrase_match` is false.
    pub phrase_score: f64,
    /// Whether the query counts as a multi-word phrase.
    pub is_phrase_query: bool,
}

/// Split text into lowercase words. Apostrophes stay inside words ("can't"); any other
/// non-alphanumeric character separates words ("hip-thrusting" is two words).
pub(crate) fn split_words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .replace('\u{2019}', "'")
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .map(|w| w.trim_matches('\''))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// The text as lowercase words separated by single spaces, with punctuation removed.
fn normalize(text: &str) -> String {
    split_words(text).join(" ")
}

/// Does the query count as a multi-word phrase? Decided on words, not raw spaces, so
/// "hip-thrusting" is a phrase and "  thrusting " is not.
pub(crate) fn is_phrase_query(query_lower: &str) -> bool {
    split_words(query_lower).len() > 1
}

/// Whole-word containment on normalized text ("the cat" is not inside "bathe cat").
fn contains_word_run(content_norm: &str, query_norm: &str) -> bool {
    !query_norm.is_empty() && format!(" {content_norm} ").contains(&format!(" {query_norm} "))
}

/// The query's meaningful words. Falls back to every word when the query is only stopwords.
fn significant_words(query_lower: &str) -> Vec<String> {
    let all = split_words(query_lower);
    let significant: Vec<String> = all
        .iter()
        .filter(|w| !QUERY_STOPWORDS.contains(&w.as_str()))
        .cloned()
        .collect();
    if significant.is_empty() {
        all
    } else {
        significant
    }
}

/// Credit for matching one query word against one caption word: 1.0 for the same word,
/// 0.75 for the same stem ("thrust" vs "thrusting"), otherwise 0.0.
fn word_credit(query_word: &str, caption_word: &str) -> f64 {
    if query_word == caption_word {
        return 1.0;
    }
    let (short, long) = if query_word.len() <= caption_word.len() {
        (query_word, caption_word)
    } else {
        (caption_word, query_word)
    };
    if short.chars().count() >= 4 && long.starts_with(short) && long.len() - short.len() <= 4 {
        0.75
    } else {
        0.0
    }
}

/// Fraction (0.0-1.0) of the query's significant words that appear in the caption.
pub(crate) fn word_coverage(caption: &str, query_words: &[String]) -> f64 {
    if query_words.is_empty() {
        return 0.0;
    }
    let caption_words = split_words(caption);
    let total: f64 = query_words
        .iter()
        .map(|q| {
            caption_words
                .iter()
                .map(|c| word_credit(q, c))
                .fold(0.0, f64::max)
        })
        .sum();
    total / query_words.len() as f64
}

/// Score how tightly a caption matches a query phrase. Higher is better.
/// Returns 0.0 if the phrase isn't present as a substring.
///
/// Rewards the phrase being a large fraction of the caption (so a caption that
/// *is* the quote beats one where the phrase is buried in a long multi-sentence
/// run), with a bonus when the caption is (nearly) exactly the phrase.
/// `query_lower` must already be lowercased; `caption` is lowercased internally.
pub(crate) fn phrase_score_str(caption: &str, query_lower: &str) -> f64 {
    let content = caption.to_lowercase();
    if content.is_empty() || !content.contains(query_lower) {
        return 0.0;
    }

    let query_len = query_lower.chars().count() as f64;
    let content_len = content.chars().count().max(1) as f64;

    // Fraction of the caption occupied by the query (0.0-1.0).
    let coverage = (query_len / content_len).min(1.0);

    // Bonus if the trimmed caption equals the phrase (ignoring surrounding
    // punctuation/whitespace) - the ideal "closest match".
    let trimmed = content.trim_matches(|c: char| !c.is_alphanumeric());
    let exact_bonus = if trimmed == query_lower { 1.0 } else { 0.0 };

    coverage + exact_bonus
}

/// Score every item against the query and order them best-first: whole phrase matches,
/// then higher word coverage, then tighter phrase matches. The sort is stable, so ties
/// keep the input order (the caller's preference, e.g. API relevance or classic seasons
/// first). `content` returns the caption text of an item.
pub(crate) fn rank_candidates<T>(
    items: Vec<T>,
    content: impl Fn(&T) -> &str,
    query_lower: &str,
) -> Vec<Candidate<T>> {
    let query_norm = normalize(query_lower);
    let is_phrase = is_phrase_query(query_lower);
    let query_words = significant_words(query_lower);

    let mut ranked: Vec<Candidate<T>> = items
        .into_iter()
        .map(|item| {
            let text = content(&item);
            let content_norm = normalize(text);
            let phrase_match = is_phrase && contains_word_run(&content_norm, &query_norm);
            let coverage = if phrase_match {
                1.0
            } else {
                word_coverage(text, &query_words)
            };
            let phrase_score = if phrase_match {
                phrase_score_str(&content_norm, &query_norm)
            } else {
                0.0
            };
            Candidate {
                item,
                phrase_match,
                coverage,
                phrase_score,
                is_phrase_query: is_phrase,
            }
        })
        .collect();

    ranked.sort_by(|a, b| {
        b.phrase_match
            .cmp(&a.phrase_match)
            .then(
                b.coverage
                    .partial_cmp(&a.coverage)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(
                b.phrase_score
                    .partial_cmp(&a.phrase_score)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
    ranked
}

/// Is this candidate good enough for the given mode?
pub(crate) fn is_acceptable<T>(candidate: &Candidate<T>, mode: MatchMode) -> bool {
    match mode {
        MatchMode::Any => true,
        MatchMode::Strict => {
            if candidate.is_phrase_query {
                candidate.phrase_match
            } else {
                candidate.coverage >= 1.0
            }
        }
        MatchMode::Loose => candidate.coverage >= MIN_LOOSE_COVERAGE,
    }
}

/// How many leading candidates tie with the best on match quality. Repeating a query
/// cycles through this many results for variety without drifting to worse matches.
pub(crate) fn top_tier_len<T>(ranked: &[Candidate<T>]) -> usize {
    let Some(best) = ranked.first() else {
        return 0;
    };
    ranked
        .iter()
        .take_while(|c| {
            c.phrase_match == best.phrase_match && (c.coverage - best.coverage).abs() < 1e-9
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Fixtures mirror real Frinkiac responses for the "thrusting" / "duffman thrusting"
    // incident and real Morbotron captions.

    fn hit(episode: &str, ts: u64, content: &str) -> serde_json::Value {
        json!({ "Episode": episode, "Timestamp": ts, "Content": content })
    }

    fn content(v: &serde_json::Value) -> &str {
        v.get("Content").and_then(|c| c.as_str()).unwrap_or("")
    }

    fn thrusting_pool() -> Vec<serde_json::Value> {
        vec![
            hit("S08E25", 245829, "Testing."),
            hit("S03E13", 428094, "Testing."),
            hit("S11E12", 1000541, "Thrust! Parry!"),
            hit("S12E15", 968634, "Duffman is thrusting"),
            hit(
                "S34E07",
                178052,
                "You're gonna mop the floor with that hip-thrusting has-been.",
            ),
        ]
    }

    fn best_episode(pool: &[serde_json::Value], query: &str, mode: MatchMode) -> Option<String> {
        let ranked = rank_candidates(pool.iter().collect(), |v| content(v), query);
        let best = ranked.first()?;
        if !is_acceptable(best, mode) {
            return None;
        }
        best.item["Episode"].as_str().map(str::to_string)
    }

    #[test]
    fn single_word_query_prefers_caption_containing_the_word() {
        // "Testing." (the API's top fuzzy hit) must lose to a caption with "thrusting".
        assert_eq!(
            best_episode(&thrusting_pool(), "thrusting", MatchMode::Strict).as_deref(),
            Some("S12E15")
        );
    }

    #[test]
    fn single_word_strict_rejects_when_no_caption_has_the_word() {
        let pool = vec![hit("S08E25", 1, "Testing."), hit("S03E13", 2, "Resting.")];
        assert_eq!(best_episode(&pool, "thrusting", MatchMode::Strict), None);
        // Loose needs at least half the words too, so the fuzzy junk is rejected.
        assert_eq!(best_episode(&pool, "thrusting", MatchMode::Loose), None);
    }

    #[test]
    fn stem_match_is_accepted_loosely_but_not_strictly() {
        let pool = vec![
            hit("S08E25", 1, "Testing."),
            hit("S11E12", 2, "Thrust! Parry!"),
        ];
        assert_eq!(best_episode(&pool, "thrusting", MatchMode::Strict), None);
        assert_eq!(
            best_episode(&pool, "thrusting", MatchMode::Loose).as_deref(),
            Some("S11E12")
        );
    }

    #[test]
    fn multi_word_loose_prefers_caption_with_most_query_words() {
        let pool = vec![
            hit("S11E16", 229146, "Duffman!"),
            hit("S09E01", 131757, "Hey, it's Duffman--"),
            hit("S12E15", 968634, "Duffman is thrusting"),
        ];
        // No caption holds the whole phrase, so strict fails...
        assert_eq!(
            best_episode(&pool, "duffman thrusting", MatchMode::Strict),
            None
        );
        // ...and loose picks the caption containing both words, not the API's first hit.
        assert_eq!(
            best_episode(&pool, "duffman thrusting", MatchMode::Loose).as_deref(),
            Some("S12E15")
        );
    }

    #[test]
    fn whole_phrase_beats_higher_ranked_partial_matches() {
        let pool = vec![
            hit("S01E01", 1, "Duffman says a lot of things."),
            hit("S02E02", 2, "Oh, yeah! Duffman thrusting forward!"),
        ];
        assert_eq!(
            best_episode(&pool, "duffman thrusting", MatchMode::Strict).as_deref(),
            Some("S02E02")
        );
    }

    #[test]
    fn phrase_matching_ignores_punctuation_and_case() {
        // The real caption has a comma and an exclamation mark; the query has neither.
        let pool = vec![
            hit("S07E16", 1, "I'm in terrible pain."),
            hit("S07E16", 2, "Good news, everyone!"),
        ];
        assert_eq!(
            best_episode(&pool, "good news everyone", MatchMode::Strict).as_deref(),
            Some("S07E16")
        );
        let ranked = rank_candidates(pool.iter().collect(), |v| content(v), "good news everyone");
        assert!(ranked[0].phrase_match);
        assert_eq!(ranked[0].item["Timestamp"], 2);
        // Hyphenated captions match a spaced query too.
        let pool = vec![hit("S34E07", 3, "that hip-thrusting has-been")];
        assert_eq!(
            best_episode(&pool, "hip thrusting", MatchMode::Strict).as_deref(),
            Some("S34E07")
        );
    }

    #[test]
    fn phrase_matching_respects_word_boundaries() {
        let pool = vec![hit("S01E01", 1, "Put on your bathe cat costume")];
        // "the cat" is inside "bathe cat" as raw text but not as whole words.
        assert_eq!(best_episode(&pool, "the cat", MatchMode::Strict), None);
    }

    #[test]
    fn tighter_phrase_match_ranks_first() {
        let pool = [
            hit(
                "S01E01",
                1,
                "Oh, good news everyone, we have a long delivery today.",
            ),
            hit("S02E02", 2, "Good news, everyone!"),
        ];
        let ranked = rank_candidates(pool.iter().collect(), |v| content(v), "good news everyone");
        assert_eq!(ranked[0].item["Timestamp"], 2);
    }

    #[test]
    fn season_order_only_breaks_ties() {
        // Equal match quality: the earlier entry stays first.
        let pool = vec![
            hit("S05E05", 1, "Everything's coming up Milhouse!"),
            hit("S20E01", 2, "Everything's coming up Milhouse!"),
        ];
        assert_eq!(
            best_episode(&pool, "milhouse", MatchMode::Strict).as_deref(),
            Some("S05E05")
        );
        // A better later match outranks a weaker earlier one.
        let pool = vec![
            hit("S05E05", 1, "Testing."),
            hit("S20E01", 2, "Everything's coming up Milhouse!"),
        ];
        assert_eq!(
            best_episode(&pool, "milhouse", MatchMode::Strict).as_deref(),
            Some("S20E01")
        );
    }

    #[test]
    fn any_mode_always_accepts_best_ranked() {
        let pool = vec![hit("S08E25", 1, "Testing.")];
        assert_eq!(
            best_episode(&pool, "thrusting", MatchMode::Any).as_deref(),
            Some("S08E25")
        );
    }

    #[test]
    fn stopwords_cannot_carry_a_loose_match() {
        // "the" alone is half of the query; it must not make an unrelated caption acceptable.
        let pool = vec![hit("S01E01", 1, "The weather is nice today.")];
        assert_eq!(best_episode(&pool, "the thrusting", MatchMode::Loose), None);
    }

    #[test]
    fn top_tier_len_counts_ties_with_the_best() {
        let pool = [
            hit("S01E01", 1, "Milhouse!"),
            hit("S02E02", 2, "Milhouse?"),
            hit("S03E03", 3, "Testing."),
        ];
        let ranked = rank_candidates(pool.iter().collect(), |v| content(v), "milhouse");
        assert_eq!(top_tier_len(&ranked), 2);
        assert_eq!(top_tier_len::<()>(&[]), 0);
    }

    #[test]
    fn is_phrase_query_counts_words_not_spaces() {
        assert!(is_phrase_query("good news everyone"));
        assert!(is_phrase_query("hip-thrusting"));
        assert!(!is_phrase_query("  thrusting "));
        assert!(!is_phrase_query("d'oh!"));
    }

    #[test]
    fn split_words_handles_hyphens_apostrophes_and_curly_quotes() {
        assert_eq!(split_words("Hip-thrusting!"), vec!["hip", "thrusting"]);
        assert_eq!(split_words("Can't stop"), vec!["can't", "stop"]);
        assert_eq!(split_words("Can\u{2019}t"), vec!["can't"]);
    }

    #[test]
    fn word_credit_rules() {
        assert_eq!(word_credit("thrusting", "thrusting"), 1.0);
        assert_eq!(word_credit("thrusting", "thrust"), 0.75);
        assert_eq!(word_credit("thrusting", "testing"), 0.0);
        // Short words never stem-match ("the" vs "then").
        assert_eq!(word_credit("the", "then"), 0.0);
    }

    #[test]
    fn phrase_score_prefers_tighter_match() {
        let q = "false advertising";

        // Caption that is essentially the phrase should outscore one where the
        // phrase is buried in a long multi-sentence run.
        let tight = "Man, that is flagrant false advertising!";
        let buried = "just like the one on the sign. D'oh! Nuts. \
                      That's false advertising. Sorry, sir. No refunds.";

        let s_tight = phrase_score_str(tight, q);
        let s_buried = phrase_score_str(buried, q);
        assert!(
            s_tight > s_buried,
            "tight={s_tight} should beat buried={s_buried}"
        );
    }

    #[test]
    fn phrase_score_exact_caption_gets_bonus() {
        let q = "false advertising";
        assert!(
            phrase_score_str("False advertising!", q)
                > phrase_score_str("that is false advertising for sure right", q)
        );
    }

    #[test]
    fn phrase_score_zero_when_absent() {
        assert_eq!(phrase_score_str("d'oh!", "false advertising"), 0.0);
    }
}
