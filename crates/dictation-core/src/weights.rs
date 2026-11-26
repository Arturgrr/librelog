//! How much each Dictionary term weighs: how often dictations said it, how often a correction
//! the person accepted produced it, and when it was added. Engines take flat term lists, so
//! weight decides which terms they hear and in what order. Only counts and timestamps are kept,
//! under a hash of each term; never text.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::replace::Replacement;
use crate::terms::{MAX_TERMS, rank_sources};
use crate::text::{flatten, says};

const DAY_MS: u64 = 24 * 60 * 60 * 1000;
/// Use counts halve every two weeks.
pub const WEIGHT_HALF_LIFE_MS: u64 = 14 * DAY_MS;
// A term is core while it was said at least twice lately, was corrected lately, or is new.
const CORE_USES: f64 = 1.5;
const CORE_CORRECTED_MS: u64 = 14 * DAY_MS;
const CORE_ADDED_MS: u64 = 7 * DAY_MS;
// Long-tail terms the moment doesn't point to are heard only while the list is shorter than this.
const HEARING_FILL: usize = 40;
const MAX_WEIGHED_TERMS: usize = 2_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TermStats {
    /// Uses, decayed to `used_at`.
    uses: f64,
    used_at: u64,
    /// Corrections that produced it, decayed to `corrected_at`.
    corrections: f64,
    corrected_at: u64,
    /// When weights first saw it in the Dictionary; 0 for a term from before weights existed.
    added_at: u64,
}

const VERSION: u32 = 1;

/// The weights blob the caller keeps between dictations.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    version: u32,
    updated_at: u64,
    terms: BTreeMap<String, TermStats>,
}

impl Stored {
    /// The stored weights, or `None` when there are none or they can't be read.
    fn parse(blob: Option<&str>) -> Option<Self> {
        blob.and_then(|text| serde_json::from_str::<Stored>(text).ok())
            .filter(|stored| stored.version == VERSION)
    }
}

fn decayed(score: f64, at: u64, now: u64) -> f64 {
    score * 0.5f64.powf(now.saturating_sub(at) as f64 / WEIGHT_HALF_LIFE_MS as f64)
}

fn term_key(term: &str) -> String {
    let digest = Sha256::digest(term.trim().to_lowercase().as_bytes());
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_core(stats: Option<&TermStats>, now: u64) -> bool {
    let Some(stats) = stats else { return true };
    decayed(stats.uses, stats.used_at, now) >= CORE_USES
        || (stats.corrections > 0.0 && now.saturating_sub(stats.corrected_at) < CORE_CORRECTED_MS)
        || now.saturating_sub(stats.added_at) < CORE_ADDED_MS
}

fn score(stats: Option<&TermStats>, now: u64) -> f64 {
    stats.map_or(0.0, |stats| {
        decayed(stats.uses, stats.used_at, now)
            + 3.0 * decayed(stats.corrections, stats.corrected_at, now)
    })
}

/// The Dictionary split into core and long-tail terms, each heaviest first and otherwise in the
/// Dictionary's own order. Without weights, every term is core.
fn weigh<'a>(
    dictionary: &'a [String],
    stored: Option<&Stored>,
    now: u64,
) -> (Vec<&'a str>, Vec<&'a str>) {
    let Some(stored) = stored else {
        return (dictionary.iter().map(String::as_str).collect(), Vec::new());
    };
    let mut rows: Vec<(usize, &str, bool, f64)> = dictionary
        .iter()
        .enumerate()
        .map(|(order, term)| {
            let stats = stored.terms.get(&term_key(term));
            (order, term.as_str(), is_core(stats, now), score(stats, now))
        })
        .collect();
    rows.sort_by(|a, b| b.3.total_cmp(&a.3).then(a.0.cmp(&b.0)));
    let core = rows.iter().filter(|row| row.2).map(|row| row.1).collect();
    let tail = rows.iter().filter(|row| !row.2).map(|row| row.1).collect();
    (core, tail)
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RankRequest {
    pub now_ms: u64,
    pub dictionary: Vec<String>,
    pub replacements: Vec<Replacement>,
    /// Distinctive terms from this dictation's context snapshot, most relevant first.
    pub context_terms: Vec<String>,
    /// The stored weights blob, or `None` when there are none.
    pub weights: Option<String>,
    pub limit: Option<u32>,
}

/// The terms speech-to-text hears for one dictation, highest first: the phrases replacements
/// listen for, the Dictionary's core terms, long-tail terms the context points to, the
/// context's own terms, then other long-tail terms, heaviest first, while the list is short.
pub fn rank_terms(request: &RankRequest) -> Vec<String> {
    let limit = request
        .limit
        .map_or(MAX_TERMS, |limit| (limit as usize).min(MAX_TERMS));
    let stored = Stored::parse(request.weights.as_deref());
    let (core, tail) = weigh(&request.dictionary, stored.as_ref(), request.now_ms);
    let moment: Vec<String> = request
        .context_terms
        .iter()
        .map(|t| t.to_lowercase())
        .collect();
    let pointed: Vec<&str> = tail
        .iter()
        .copied()
        .filter(|term| points_to(term, &moment))
        .collect();
    let spoken: Vec<&str> = request
        .replacements
        .iter()
        .map(|r| r.from.as_str())
        .collect();
    let context: Vec<&str> = request.context_terms.iter().map(String::as_str).collect();
    let heard = rank_sources([spoken, core, pointed, context], limit);
    let fill = heard.len().max(HEARING_FILL).min(limit);
    rank_sources(
        [heard.iter().map(String::as_str).collect::<Vec<_>>(), tail],
        fill,
    )
}

/// Whether the moment points to a long-tail term: one of the context's terms is it, contains it,
/// or is one of its words.
fn points_to(term: &str, moment: &[String]) -> bool {
    let lower = term.to_lowercase();
    let flat = flatten(term);
    moment.iter().any(|seen| {
        *seen == lower
            || (flat.chars().count() >= 3 && flatten(seen) == flat)
            || lower
                .split(|c: char| !c.is_alphanumeric())
                .any(|word| word.chars().count() >= 3 && word == seen)
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TermEvent {
    /// A dictation came out as `text`: each Dictionary term in it was used.
    Used { text: String },
    /// The person accepted these corrections: the engine mishears them.
    Corrected { terms: Vec<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RecordRequest {
    pub now_ms: u64,
    pub dictionary: Vec<String>,
    pub weights: Option<String>,
    pub event: TermEvent,
}

/// The weights blob after `event`. Terms no longer in the Dictionary are forgotten.
pub fn record_terms(request: RecordRequest) -> String {
    let now = request.now_ms;
    let parsed = Stored::parse(request.weights.as_deref());
    let fresh = parsed.is_none();
    let mut stored = parsed.unwrap_or(Stored {
        version: VERSION,
        ..Stored::default()
    });
    let dictionary: Vec<(&String, String)> = request
        .dictionary
        .iter()
        .take(MAX_WEIGHED_TERMS)
        .map(|term| (term, term_key(term)))
        .collect();
    stored
        .terms
        .retain(|key, _| dictionary.iter().any(|(_, listed)| listed == key));
    // A term weights haven't seen is new, unless the weights start now: then the whole
    // Dictionary was there before, and nothing in it is "just added".
    for (_, key) in &dictionary {
        stored.terms.entry(key.clone()).or_insert(TermStats {
            added_at: if fresh { 0 } else { now },
            ..TermStats::default()
        });
    }
    let bump = |stats: &mut TermStats, corrected: bool| {
        if corrected {
            stats.corrections = decayed(stats.corrections, stats.corrected_at, now) + 1.0;
            stats.corrected_at = now;
        } else {
            stats.uses = decayed(stats.uses, stats.used_at, now) + 1.0;
            stats.used_at = now;
        }
    };
    match &request.event {
        TermEvent::Used { text } => {
            for (term, key) in &dictionary {
                if says(text, term)
                    && let Some(stats) = stored.terms.get_mut(key)
                {
                    bump(stats, false);
                }
            }
        }
        TermEvent::Corrected { terms } => {
            for term in terms {
                let key = term_key(term);
                if let Some(stats) = stored.terms.get_mut(&key) {
                    bump(stats, true);
                }
            }
        }
    }
    stored.updated_at = now;
    serde_json::to_string(&stored).expect("weights serialize")
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_000 * DAY_MS;
    const DICTIONARY: [&str; 4] = ["Granola", "Fastrepl", "Wispr Flow", "Supabase"];

    fn list(terms: &[&str]) -> Vec<String> {
        terms.iter().map(|t| (*t).to_owned()).collect()
    }

    fn record(weights: Option<String>, now: u64, dictionary: &[&str], event: TermEvent) -> String {
        record_terms(RecordRequest {
            now_ms: now,
            dictionary: list(dictionary),
            weights,
            event,
        })
    }

    fn used(text: &str) -> TermEvent {
        TermEvent::Used { text: text.into() }
    }

    fn rank(
        weights: Option<&str>,
        now: u64,
        dictionary: &[&str],
        context: &[&str],
        limit: Option<u32>,
    ) -> Vec<String> {
        rank_terms(&RankRequest {
            now_ms: now,
            dictionary: list(dictionary),
            replacements: vec![Replacement {
                from: "work mail".into(),
                to: "a@b.c".into(),
            }],
            context_terms: list(context),
            weights: weights.map(str::to_owned),
            limit,
        })
    }

    #[test]
    fn ranks_terms_by_recent_use_and_accepted_corrections() {
        let listed = ["work mail", "Granola", "Fastrepl", "Wispr Flow", "Supabase"];
        // The Dictionary as weights first see it predates them: nothing in it is new.
        let blob = record(None, NOW, &DICTIONARY, used("nothing listed"));
        assert_eq!(rank(Some(&blob), NOW, &DICTIONARY, &[], None), listed);

        let blob = record(Some(blob), NOW, &DICTIONARY, used("Fastrepl ships"));
        let blob = record(Some(blob), NOW, &DICTIONARY, used("fastrepl again"));
        let blob = record(
            Some(blob),
            NOW,
            &DICTIONARY,
            TermEvent::Corrected {
                terms: list(&["wispr flow"]),
            },
        );
        assert_eq!(
            rank(Some(&blob), NOW, &DICTIONARY, &[], None),
            ["work mail", "Wispr Flow", "Fastrepl", "Granola", "Supabase"]
        );
        // A long-tail term the dictation's context points to comes before the rest of the tail.
        assert_eq!(
            rank(Some(&blob), NOW, &DICTIONARY, &["supabase"], None),
            ["work mail", "Wispr Flow", "Fastrepl", "Supabase", "Granola"]
        );
        assert_eq!(
            rank(Some(&blob), NOW, &DICTIONARY, &[], Some(3)),
            ["work mail", "Wispr Flow", "Fastrepl"]
        );
        // No weights, or an unreadable blob, hears the Dictionary as listed.
        assert_eq!(rank(None, NOW, &DICTIONARY, &[], None), listed);
        assert_eq!(rank(Some("not json"), NOW, &DICTIONARY, &[], None), listed);
        // Two months on, one fresh use outweighs the faded ones.
        let later = NOW + 60 * DAY_MS;
        let blob = record(Some(blob), later, &DICTIONARY, used("Granola notes"));
        assert_eq!(
            rank(Some(&blob), later, &DICTIONARY, &[], None),
            ["work mail", "Granola", "Wispr Flow", "Fastrepl", "Supabase"]
        );
        // Only hashes, counts and timestamps are stored.
        let lower = blob.to_lowercase();
        assert!(
            DICTIONARY
                .iter()
                .all(|term| !lower.contains(&term.to_lowercase()))
        );
    }

    #[test]
    fn hears_new_terms_ahead_of_a_long_tail_and_forgets_removed_ones() {
        let many: Vec<String> = (0..50).map(|n| format!("Term{n}x")).collect();
        let mut dictionary: Vec<&str> = many.iter().map(String::as_str).collect();
        let blob = record(None, NOW, &dictionary, used(""));
        dictionary.push("Fastrepl");
        let blob = record(Some(blob), NOW + DAY_MS, &dictionary, used(""));
        let heard = rank(Some(&blob), NOW + DAY_MS, &dictionary, &[], None);
        // The new term is core; the 50 old ones only fill the list up to 40.
        assert_eq!(heard[..2], ["work mail", "Fastrepl"]);
        assert_eq!(heard.len(), HEARING_FILL);
        let blob = record(Some(blob), NOW + DAY_MS, &["Fastrepl"], used(""));
        assert_eq!(blob.matches("addedAt").count(), 1);
    }
}
