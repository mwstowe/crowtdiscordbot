//! Choosing which caption cards go into a Frinkiac GIF.
//!
//! Frinkiac returns a fixed window of cards (usually 1-4, 3-9 seconds) around the
//! matched frame. That window often carries unrelated lines before or after the joke,
//! and it doesn't know which line the search matched. This module picks a tighter clip
//! made of three parts:
//!
//! - the matched sentence, found from the frame Frinkiac matched
//! - one sentence of setup before it, when the window has one that is whole and fits
//! - the tail: nothing more (the baseline), or up to wherever the joke ends
//!
//! The tail is where the search term's position matters: the term may sit at the start
//! of a joke (the payoff follows) or at the end (nothing follows). Gemini decides where
//! the clip ends; code decides everything else and checks Gemini's answer, falling back
//! to the baseline whenever Gemini is unavailable or its answer doesn't hold up.
//!
//! Cuts only ever fall on card boundaries, and never in the middle of a sentence whose
//! remainder is inside the window.

use crate::frinkiac::{ends_mid_sentence, starts_mid_sentence, TimedSubtitle};
use crate::gemini_api::GeminiClient;
use std::time::Duration;
use tracing::{info, warn};

/// Longest clip built from a selection, in milliseconds. A single sentence longer than
/// this is still kept whole (cards are never cut), but nothing else is added to it.
pub(crate) const MAX_CLIP_MS: u64 = 8_000;

/// Complete sentences of setup kept before the matched sentence.
pub(crate) const SETUP_UNITS: usize = 1;

/// Most sentences kept after the matched sentence (an answer or payoff).
const MAX_TRAILING_UNITS: usize = 2;

/// How long to wait for Gemini before using the baseline instead.
const GEMINI_TIMEOUT: Duration = Duration::from_secs(8);

/// A run of consecutive cards forming one sentence, or a sentence fragment cut off by
/// the edge of the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Unit {
    /// Index of the first card (inclusive).
    pub first: usize,
    /// Index of the last card (inclusive).
    pub last: usize,
    /// Opens with a lowercase continuation, so its beginning lies outside the window.
    pub starts_mid: bool,
    /// Has no sentence ending, so its end lies outside the window.
    pub ends_open: bool,
}

/// Group cards into sentence units. A card joins the previous unit when that unit is
/// unfinished (no sentence ending, or a trailing comma) or the card starts in lowercase.
pub(crate) fn build_units(cards: &[TimedSubtitle]) -> Vec<Unit> {
    let mut units: Vec<Unit> = Vec::new();
    for (i, card) in cards.iter().enumerate() {
        let continues = units.last().is_some_and(|u| {
            ends_mid_sentence(&cards[u.last].text) || starts_mid_sentence(&card.text)
        });
        match units.last_mut() {
            Some(u) if continues => u.last = i,
            _ => units.push(Unit {
                first: i,
                last: i,
                starts_mid: false,
                ends_open: false,
            }),
        }
    }
    for u in &mut units {
        u.starts_mid = starts_mid_sentence(&cards[u.first].text);
        u.ends_open = ends_mid_sentence(&cards[u.last].text);
    }
    units
}

/// Index of the unit containing the given card.
fn unit_index_of(units: &[Unit], card: usize) -> usize {
    units
        .iter()
        .position(|u| u.first <= card && card <= u.last)
        .unwrap_or(0)
}

/// Milliseconds from the start of `first` to the end of `last`.
fn span_ms(cards: &[TimedSubtitle], first: usize, last: usize) -> u64 {
    cards[last].end.saturating_sub(cards[first].start)
}

/// The card Frinkiac matched: the one whose time span contains the frame timestamp,
/// otherwise the one whose representative frame is nearest to it.
pub(crate) fn anchor_card_index(cards: &[TimedSubtitle], frame_ts: u64) -> Option<usize> {
    if cards.is_empty() {
        return None;
    }
    cards
        .iter()
        .position(|c| c.start <= frame_ts && frame_ts <= c.end)
        .or_else(|| (0..cards.len()).min_by_key(|&i| cards[i].representative_ts.abs_diff(frame_ts)))
}

/// Add up to `SETUP_UNITS` whole sentences before unit `a`, staying within `MAX_CLIP_MS`
/// of the clip's end (`hi`). A fragment cut off by the window start is never used as setup.
/// Returns the index of the first unit of the clip.
fn add_setup(cards: &[TimedSubtitle], units: &[Unit], a: usize, hi: usize) -> usize {
    let mut lo = a;
    let mut taken = 0;
    while taken < SETUP_UNITS && lo > 0 {
        let prev = &units[lo - 1];
        if prev.starts_mid || span_ms(cards, prev.first, units[hi].last) > MAX_CLIP_MS {
            break;
        }
        lo -= 1;
        taken += 1;
    }
    lo
}

/// Deterministic clip: the matched sentence, the answer if it is a question, and
/// `SETUP_UNITS` sentence(s) of setup, all within `MAX_CLIP_MS`. Returns inclusive card
/// indices.
pub(crate) fn baseline_selection(cards: &[TimedSubtitle], anchor_card: usize) -> (usize, usize) {
    let units = build_units(cards);
    let a = unit_index_of(&units, anchor_card);

    // A question's payoff is usually the next line, so keep it when it is a complete
    // sentence and fits.
    let mut hi = a;
    if cards[units[a].last].text.trim_end().ends_with('?') {
        if let Some(next) = units.get(a + 1) {
            if !next.ends_open && span_ms(cards, units[a].first, next.last) <= MAX_CLIP_MS {
                hi = a + 1;
            }
        }
    }

    let lo = add_setup(cards, &units, a, hi);
    (units[lo].first, units[hi].last)
}

/// A clip that ends at card `last` (inclusive), for when something else (Gemini) has
/// decided where the joke ends. Returns `None` if `last` is out of range or falls before
/// the matched sentence.
///
/// The end is corrected rather than trusted: it is rounded out to a whole sentence, an
/// unfinished trailing sentence is dropped, at most `MAX_TRAILING_UNITS` sentences follow
/// the matched one, and trailing sentences are shed until the clip fits `MAX_CLIP_MS`
/// (the matched sentence is never cut). Setup is then added as in the baseline.
pub(crate) fn selection_ending_at(
    cards: &[TimedSubtitle],
    anchor_card: usize,
    last: usize,
) -> Option<(usize, usize)> {
    if cards.is_empty() || anchor_card >= cards.len() || last >= cards.len() || last < anchor_card {
        return None;
    }

    let units = build_units(cards);
    let a = unit_index_of(&units, anchor_card);
    let mut hi = unit_index_of(&units, last);

    while hi > a && units[hi].ends_open {
        hi -= 1;
    }
    if hi - a > MAX_TRAILING_UNITS {
        hi = a + MAX_TRAILING_UNITS;
    }
    while hi > a && span_ms(cards, units[a].first, units[hi].last) > MAX_CLIP_MS {
        hi -= 1;
    }

    let lo = add_setup(cards, &units, a, hi);
    Some((units[lo].first, units[hi].last))
}

/// JSON schema for Gemini's answer.
pub(crate) fn selection_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "OBJECT",
        "properties": {
            "last": { "type": "INTEGER" },
            "reason": { "type": "STRING" }
        },
        "required": ["last", "reason"]
    })
}

/// The prompt asking Gemini where the clip should end. Cards are numbered from 1.
pub(crate) fn build_selection_prompt(
    query: &str,
    cards: &[TimedSubtitle],
    anchor_card: usize,
) -> String {
    let t0 = cards.first().map(|c| c.start).unwrap_or(0);
    let listing = cards
        .iter()
        .enumerate()
        .map(|(i, c)| {
            format!(
                "{} [{:.1}-{:.1}s] {}{}",
                i + 1,
                c.start.saturating_sub(t0) as f64 / 1000.0,
                c.end.saturating_sub(t0) as f64 / 1000.0,
                c.text,
                if i == anchor_card {
                    "   <-- matched line"
                } else {
                    ""
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "You help trim a looping GIF clip from The Simpsons that a chat bot posts after a \
         search. Below are consecutive subtitle cards around one moment, numbered from 1, with \
         start and end times. The user searched for: \"{query}\". The card marked as the matched \
         line is why this moment was chosen.\n\n\
         The clip starts at the matched line, plus one sentence of setup before it that is added \
         for you. Your only job is to say where the clip ENDS: the number of its LAST card, which \
         must be the matched line's card or a later one.\n\
         - the search term may fall at the start, middle or end of the joke or quote\n\
         - if the matched line is the punchline or the whole quote, the last card is the matched \
         line itself\n\
         - if the matched line is the setup or the start of the joke, the last card is where the \
         payoff lands\n\
         - if the matched line is a question and the answer follows, include the answer\n\
         - end right after the punchline or payoff. Do not include lines after it, especially \
         ones unrelated to the joke\n\
         - never end in the middle of a sentence\n\
         - the whole clip, setup included, should stay under about 7 seconds\n\n\
         Cards:\n{listing}"
    )
}

/// Read Gemini's `last` (1-based) as a 0-based card index for `n` cards.
pub(crate) fn parse_last_card(value: &serde_json::Value, n: usize) -> Option<usize> {
    let last = value.get("last")?.as_u64()? as usize;
    if last == 0 || last > n {
        return None;
    }
    Some(last - 1)
}

/// Ask Gemini where the clip should end and return the resulting clip (inclusive card
/// indices). `None` means the caller should use the baseline.
pub(crate) async fn gemini_selection(
    gemini: &GeminiClient,
    query: &str,
    cards: &[TimedSubtitle],
    anchor_card: usize,
) -> Option<(usize, usize)> {
    // With a single sentence in the window there is nothing to choose between.
    if build_units(cards).len() <= 1 {
        return None;
    }

    let prompt = build_selection_prompt(query, cards, anchor_card);
    let answer = match gemini
        .generate_structured_json(&prompt, selection_schema(), GEMINI_TIMEOUT)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            warn!("Clip selection: Gemini unavailable ({e}); using baseline cut");
            return None;
        }
    };

    let Some(last) = parse_last_card(&answer, cards.len()) else {
        warn!("Clip selection: unusable Gemini answer {answer}; using baseline cut");
        return None;
    };
    let selection = selection_ending_at(cards, anchor_card, last);
    info!(
        "Clip selection: Gemini ended the clip at card {}, giving cards {:?} ({})",
        last + 1,
        selection.map(|(a, b)| (a + 1, b + 1)),
        answer.get("reason").and_then(|r| r.as_str()).unwrap_or("")
    );
    selection
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// (text, start_ms, end_ms) cards taken from real Frinkiac caption windows.
    fn cards(spec: &[(&str, u64, u64)]) -> Vec<TimedSubtitle> {
        spec.iter()
            .map(|(text, start, end)| TimedSubtitle {
                text: text.to_string(),
                start: *start,
                end: *end,
                representative_ts: (start + end) / 2,
            })
            .collect()
    }

    fn mmm_donuts() -> Vec<TimedSubtitle> {
        cards(&[
            ("Mmm... donuts.", 0, 2461),
            (
                "I am happy to report we've succeeded in blocking",
                2878,
                5214,
            ),
        ])
    }

    fn insect_overlords() -> Vec<TimedSubtitle> {
        cards(&[
            ("The ants will soon be here.", 0, 2294),
            (
                "And I, for one, welcome our new insect overlords.",
                2377,
                6048,
            ),
            (
                "I'd like to remind them that as a trusted TV personality,",
                6131,
                8509,
            ),
        ])
    }

    fn thrusting() -> Vec<TimedSubtitle> {
        cards(&[
            ("It's such a beautiful day.", 0, 1877),
            ("Where are the crowds?", 1960, 3253),
            ("Duffman is thrusting", 3336, 4880),
            ("in the direction of the problem.", 4963, 6631),
        ])
    }

    fn danger() -> Vec<TimedSubtitle> {
        cards(&[
            ("and work nights at the plant.", 0, 1376),
            ("Isn't that dangerous to your health?", 1459, 3294),
            ("You think I'm in danger?", 3377, 4712),
            ("I am the danger. I'm the one who knocks.", 4795, 7507),
        ])
    }

    fn steamed_hams() -> Vec<TimedSubtitle> {
        cards(&[
            ("Patented Skinner burgers, old family recipe.", 0, 2878),
            ("- For steamed hams. - Yes.", 2961, 6256),
            ("Yes, and you call them steamed hams", 6339, 8049),
        ])
    }

    #[test]
    fn units_merge_continuations_and_flag_window_edges() {
        let t = thrusting();
        let u = build_units(&t);
        assert_eq!(u.len(), 3);
        assert_eq!(
            (u[2].first, u[2].last),
            (2, 3),
            "split sentence is one unit"
        );
        assert!(!u[2].ends_open);

        let d = danger();
        let u = build_units(&d);
        assert!(u[0].starts_mid, "window opens mid-sentence");

        let s = steamed_hams();
        let u = build_units(&s);
        assert!(u.last().unwrap().ends_open, "window closes mid-sentence");
    }

    #[test]
    fn anchor_is_the_card_holding_the_frame() {
        let t = thrusting();
        assert_eq!(anchor_card_index(&t, 4000), Some(2));
        // Outside every card: nearest representative frame wins.
        assert_eq!(anchor_card_index(&t, 7000), Some(3));
        assert_eq!(anchor_card_index(&[], 0), None);
    }

    #[test]
    fn baseline_drops_unrelated_trailing_line() {
        // The joke is the first card; the second is an unfinished, unrelated sentence.
        assert_eq!(baseline_selection(&mmm_donuts(), 0), (0, 0));
    }

    #[test]
    fn baseline_stops_before_dangling_fragment_and_keeps_one_setup() {
        // Setup + punchline; the unfinished third card is left out.
        assert_eq!(baseline_selection(&insect_overlords(), 1), (0, 1));
    }

    #[test]
    fn baseline_keeps_one_setup_sentence_and_whole_split_sentence() {
        // "Where are the crowds?" + "Duffman is thrusting in the direction of the problem."
        assert_eq!(baseline_selection(&thrusting(), 2), (1, 3));
    }

    #[test]
    fn baseline_includes_answer_to_a_question() {
        // Setup question, the matched question, then its answer.
        assert_eq!(baseline_selection(&danger(), 2), (1, 3));
    }

    #[test]
    fn baseline_never_uses_a_window_edge_fragment_as_setup() {
        // Card 0 starts mid-sentence (its beginning is outside the window), so it can't
        // serve as setup for the matched line.
        let c = cards(&[
            ("and work nights at the plant.", 0, 1376),
            ("I am the danger.", 1459, 3294),
        ]);
        assert_eq!(baseline_selection(&c, 1), (1, 1));
    }

    #[test]
    fn baseline_question_pulls_in_the_next_complete_line() {
        // "Isn't that dangerous to your health?" is a question, so the next line follows.
        assert_eq!(baseline_selection(&danger(), 1), (1, 2));
    }

    #[test]
    fn baseline_with_unfinished_following_line() {
        // One setup sentence before the matched card; the unfinished tail is excluded.
        assert_eq!(baseline_selection(&steamed_hams(), 1), (0, 1));
    }

    #[test]
    fn baseline_respects_the_time_budget() {
        let c = cards(&[
            ("A long setup that runs a while.", 0, 5000),
            ("And then a very long punchline.", 5100, 12_800),
        ]);
        // Adding the setup would exceed MAX_CLIP_MS, so only the anchor is kept.
        assert_eq!(baseline_selection(&c, 1), (1, 1));
    }

    #[test]
    fn ending_at_drops_unfinished_trailing_sentence_and_still_adds_setup() {
        // Gemini ended on the dangling third card: it is dropped, and the setup sentence
        // is added even though Gemini's pick didn't mention it.
        assert_eq!(selection_ending_at(&insect_overlords(), 1, 2), Some((0, 1)));
        assert_eq!(selection_ending_at(&insect_overlords(), 1, 1), Some((0, 1)));
    }

    #[test]
    fn ending_at_rejects_bad_ends() {
        let t = thrusting();
        assert_eq!(
            selection_ending_at(&t, 2, 1),
            None,
            "ends before the matched line"
        );
        assert_eq!(selection_ending_at(&t, 2, 9), None, "past the last card");
        assert_eq!(selection_ending_at(&t, 9, 9), None, "anchor out of range");
        assert_eq!(selection_ending_at(&[], 0, 0), None);
    }

    #[test]
    fn ending_at_rounds_out_to_whole_sentences() {
        // Ending on the first half of a split sentence takes the whole sentence.
        assert_eq!(selection_ending_at(&thrusting(), 2, 2), Some((1, 3)));
    }

    #[test]
    fn ending_at_keeps_the_payoff_after_the_matched_line() {
        // The term is at the start of the joke: the payoff that follows is kept.
        let c = cards(&[
            ("I'm going to tell you a secret.", 0, 2000),
            ("The secret is this.", 2100, 3500),
            ("There is no secret.", 3600, 5000),
        ]);
        assert_eq!(selection_ending_at(&c, 1, 2), Some((0, 2)));
        // The term is the punchline: nothing after it is added.
        assert_eq!(selection_ending_at(&c, 1, 1), Some((0, 1)));
    }

    #[test]
    fn ending_at_caps_trailing_sentences() {
        let c = cards(&[
            ("One.", 0, 1000),
            ("Two.", 1100, 2000),
            ("Three.", 2100, 3000),
            ("Four.", 3100, 4000),
            ("Five.", 4100, 5000),
        ]);
        // Matched sentence 0 may be followed by at most MAX_TRAILING_UNITS (2) sentences.
        assert_eq!(selection_ending_at(&c, 0, 4), Some((0, 2)));
    }

    #[test]
    fn ending_at_respects_budget_and_never_cuts_the_matched_line() {
        let long = |trailing_end: u64| {
            cards(&[
                ("Setup line here.", 0, 3000),
                ("The matched line is quite long indeed.", 3100, 9200),
                ("And a trailing payoff.", 9300, trailing_end),
            ])
        };
        // Matched + payoff is 7.9s: fits, but the setup would push it over, so no setup.
        assert_eq!(selection_ending_at(&long(11_000), 1, 2), Some((1, 2)));
        // Matched + payoff is 8.9s: the payoff is shed; the matched line stays whole.
        assert_eq!(selection_ending_at(&long(12_000), 1, 2), Some((1, 1)));
    }

    #[test]
    fn prompt_numbers_cards_marks_match_and_states_the_rules() {
        let p = build_selection_prompt("thrusting", &thrusting(), 2);
        assert!(p.contains("\"thrusting\""));
        assert!(p.contains("1 [0.0-1.9s] It's such a beautiful day."));
        assert!(p.contains("3 [3.3-4.9s] Duffman is thrusting   <-- matched line"));
        assert!(p.contains("one sentence of setup before it that is added for you"));
        assert!(p.contains("LAST card"));
        assert!(p.contains("start, middle or end"));
        assert!(!p.contains("2 [1.9-3.3s] Where are the crowds?   <--"));
    }

    #[test]
    fn parse_last_card_converts_to_zero_based_and_rejects_bad_values() {
        assert_eq!(
            parse_last_card(&json!({"last": 3, "reason": "x"}), 4),
            Some(2)
        );
        assert_eq!(parse_last_card(&json!({"last": 4}), 4), Some(3));
        assert_eq!(parse_last_card(&json!({"last": 0}), 4), None);
        assert_eq!(parse_last_card(&json!({"last": 5}), 4), None);
        assert_eq!(parse_last_card(&json!({"last": "3"}), 4), None);
        assert_eq!(parse_last_card(&json!({}), 4), None);
    }
}
