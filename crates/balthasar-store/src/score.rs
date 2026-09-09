//! How a result is ranked.
//!
//! Seven signals: three about the question, four about the memory.

use balthasar_model::{Memory, Timestamp};

/// One result, and why it scored what it did; every term is kept for `--explain`.
#[derive(Debug, Clone)]
pub struct Scored {
    pub memory: Memory,
    pub score: f64,
    /// Cosine similarity, when both sides were embedded.
    pub semantic: Option<f64>,
    /// The lexical component, from full-text ranking.
    pub lexical: f64,
    /// What the query and the memory are both about, rarity-weighted.
    pub entity: f64,
    /// How often and how recently it has been needed.
    pub frecency: f64,
    pub confidence: f64,
    /// Strength at the moment of the search.
    pub strength: f64,
    /// Whether it came from the nearer store.
    pub near: bool,
}

/// Only the query-relative signals. Folding confidence and strength in is how a certain,
/// well-used, irrelevant fact outranks a hesitant, faded, correct one.
impl Scored {
    /// The query-relative share of the score, in `0..1`.
    #[must_use]
    pub fn relevance(&self) -> f64 {
        self.semantic
            .unwrap_or(self.lexical)
            .max(self.lexical)
            .max(self.entity)
    }
}

/// How much each signal counts when nothing has been configured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weights {
    /// Cosine similarity, when an embedding exists on both sides.
    pub semantic: f64,
    /// Full-text ranking. The floor, always present.
    pub lexical: f64,
    /// How often and how recently a memory has actually been needed.
    pub frecency: f64,
    pub confidence: f64,
    pub strength: f64,
    /// Whether the project store outranks the global one.
    pub scope: f64,
    /// What the query and the memory are both *about*, which words alone do not capture.
    pub entity: f64,
}

impl Default for Weights {
    fn default() -> Self {
        Self {
            semantic: 0.26,
            lexical: 0.22,
            entity: 0.14,
            frecency: 0.13,
            confidence: 0.13,
            strength: 0.08,
            scope: 0.04,
        }
    }
}

impl Weights {
    /// The same weighting with the semantic share given to the lexical one, for a store with
    /// nothing embedded. Redistributing keeps the remaining signals summing to one.
    #[must_use]
    pub fn without_vectors(self) -> Self {
        Self {
            lexical: self.lexical + self.semantic,
            semantic: 0.0,
            ..self
        }
    }

    #[must_use]
    pub fn total(self) -> f64 {
        self.semantic + self.lexical + self.entity + self.frecency + self.confidence + self.strength
    }
}

/// Access count at which the frequency half of frecency is most of the way to full.
const FREQUENT: f64 = 8.0;

/// How long an access stays fresh, in days.
const RECENT_DAYS: f64 = 7.0;

/// How often and how recently a memory has actually been needed, in `0..1`. A memory created
/// and never recalled scores 0.5 — full access-recency, no frequency.
#[must_use]
pub fn frecency(access_count: u32, last_accessed: Timestamp, now: Timestamp) -> f64 {
    let frequency = 1.0 - (-f64::from(access_count) / FREQUENT).exp();
    let days = ((now - last_accessed).max(0)) as f64 / 86_400.0;
    let recency = (-days / RECENT_DAYS).exp();
    (frequency + recency) / 2.0
}

/// Cosine similarity of two vectors, or `None` when they cannot be compared. Different lengths
/// mean different models, and comparing across them produces a number that means nothing.
#[must_use]
pub fn cosine(a: &[f32], b: &[f32]) -> Option<f64> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let (mut dot, mut left, mut right) = (0.0_f64, 0.0_f64, 0.0_f64);
    for (x, y) in a.iter().zip(b) {
        dot += f64::from(*x) * f64::from(*y);
        left += f64::from(*x) * f64::from(*x);
        right += f64::from(*y) * f64::from(*y);
    }
    if left == 0.0 || right == 0.0 {
        return None;
    }
    // Mapped from `-1..1` into `0..1`: a term that could go negative would veto every other axis.
    Some(((dot / (left.sqrt() * right.sqrt())) + 1.0) / 2.0)
}

/// Words that match everything and therefore mean nothing. FTS5's `unicode61` tokenizer has no
/// stopword list, so `the` is a term like any other and matches most memories.
const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "been", "but", "by", "can", "did", "do", "does",
    "for", "from", "had", "has", "have", "how", "i", "if", "in", "is", "it", "its", "me", "my",
    "no", "not", "of", "on", "or", "our", "should", "so", "than", "that", "the", "their", "them",
    "then", "there", "these", "this", "those", "to", "was", "we", "were", "what", "when", "where",
    "which", "who", "why", "will", "with", "would", "you", "your",
];

/// A person's words as something FTS5 will accept: every term quoted and joined with `OR`, and
/// stopwords dropped. Unquoted input is a syntax the user did not ask to be writing.
pub fn fts_query(query: &str) -> String {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|term| term.replace('"', ""))
        .filter(|term| !term.is_empty())
        .filter(|term| {
            let bare: String = term
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
                .to_lowercase();
            !STOPWORDS.contains(&bare.as_str())
        })
        .map(|term| format!("\"{term}\""))
        .collect();
    if terms.is_empty() {
        // A term nothing can match, rather than a syntax error or a match on everything. Not a
        // NUL: FTS5 reads one inside a quoted string as the end of it and reports "unterminated".
        return "\"zznomatchzz\"".to_owned();
    }
    terms.join(" OR ")
}

/// The words a query is actually asking about.
pub(crate) fn terms_of(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|term| {
            term.chars()
                .filter(|c| c.is_alphanumeric() || *c == '.' || *c == '/' || *c == '_' || *c == '-')
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|term| !term.is_empty() && !STOPWORDS.contains(&term.as_str()))
        .collect()
}

/// What share of the query's words the memory actually contains, the absolute half of the
/// lexical signal. A query with nothing to ask about is neutral rather than zero.
#[must_use]
pub fn coverage(wanted: &[String], text: &str) -> f64 {
    if wanted.is_empty() {
        return 1.0;
    }
    // Stemmed on both sides, because the layer that found the row stems too: raw containment
    // scored a memory about `make test` at zero for a question about `tests`.
    let held: Vec<String> = text
        .split_whitespace()
        .map(|word| {
            stem(
                &word
                    .chars()
                    .filter(|c| {
                        c.is_alphanumeric() || *c == '.' || *c == '/' || *c == '_' || *c == '-'
                    })
                    .collect::<String>()
                    .to_lowercase(),
            )
        })
        .filter(|word| !word.is_empty())
        .collect();

    let hit = wanted
        .iter()
        .filter(|term| {
            let want = stem(term);
            held.iter()
                .any(|word| word == &want || word.contains(&want))
        })
        .count();
    hit as f64 / wanted.len() as f64
}

/// A crude stem, applied to both sides so they agree with FTS5's porter tokenizer.
fn stem(word: &str) -> String {
    for suffix in ["ing", "ed", "es", "s"] {
        if word.len() > suffix.len() + 2 && word.ends_with(suffix) && !word.ends_with("ss") {
            return word[..word.len() - suffix.len()].to_owned();
        }
    }
    word.to_owned()
}

/// One bm25 rank against the best in its result set, as a `0..1` where more is better. Both
/// numbers are negative and more-negative is better; a set in which nothing scored is neutral.
pub(crate) fn relative(rank: f64, best: f64) -> f64 {
    if best >= 0.0 {
        return 0.5;
    }
    (rank / best).clamp(0.0, 1.0)
}

#[cfg(test)]
mod scoring {
    use super::*;

    const NOW: Timestamp = 1_756_000_000;
    const DAY: Timestamp = 86_400;

    #[test]
    fn something_never_recalled_starts_level_rather_than_last() {
        assert!((frecency(0, NOW, NOW) - 0.5).abs() < 0.01);
    }

    #[test]
    fn what_is_returned_to_outranks_what_is_not() {
        let often = frecency(20, NOW, NOW);
        let once = frecency(1, NOW, NOW);
        assert!(often > once);
    }

    #[test]
    fn an_old_access_counts_for_less_than_a_recent_one() {
        assert!(frecency(5, NOW, NOW) > frecency(5, NOW - 30 * DAY, NOW));
    }

    #[test]
    fn frecency_stays_in_range() {
        for (count, days) in [(0, 0), (1000, 0), (0, 10_000), (1000, 10_000)] {
            let value = frecency(count, NOW - days * DAY, NOW);
            assert!((0.0..=1.0).contains(&value), "{count}/{days} gave {value}");
        }
    }

    #[test]
    fn identical_vectors_are_as_similar_as_it_gets() {
        let v = [1.0_f32, 0.0, 0.5];
        assert!((cosine(&v, &v).expect("comparable") - 1.0).abs() < 1e-6);
    }

    #[test]
    fn opposite_vectors_are_as_dissimilar_as_it_gets() {
        assert!(cosine(&[1.0, 0.0], &[-1.0, 0.0]).expect("comparable") < 1e-6);
    }

    #[test]
    fn similarity_never_goes_negative() {
        let value = cosine(&[1.0, 2.0], &[-3.0, -1.0]).expect("comparable");
        assert!((0.0..=1.0).contains(&value), "{value}");
    }

    #[test]
    fn vectors_from_different_models_are_not_compared() {
        assert_eq!(cosine(&[1.0, 0.0], &[1.0, 0.0, 0.0]), None);
        assert_eq!(cosine(&[], &[]), None);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), None);
    }

    #[test]
    fn dropping_vectors_keeps_the_weighting_summing_to_one() {
        let with = Weights::default();
        let without = with.without_vectors();
        assert!((with.total() - without.total()).abs() < 1e-9);
        assert_eq!(without.semantic, 0.0);
    }
}
