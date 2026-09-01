//! How a result is ranked.
//!
//! Finding is [`super::read`]'s job; deciding what a hit is worth is this one's. They came
//! apart when the file that did both reached 985 lines, which is the gate saying the state
//! inside it wanted a second module.
//!
//! Seven signals. Three are about the question — is this an answer. Four are about the memory
//! — is it worth giving. Keeping them apart is what lets a store say *this matches well and I
//! should not be stating it as fact*, which is the whole of the two floors.

use aeon_model::{Memory, Timestamp};

/// One result, and why it scored what it did.
///
/// Every term is kept rather than only the sum, because `--explain` exists so a ranking can be
/// argued with and a total nobody can decompose is not an argument.
#[derive(Debug, Clone)]
pub struct Scored {
    /// The memory.
    pub memory: Memory,
    /// What it scored overall.
    pub score: f64,
    /// Cosine similarity, when both sides were embedded.
    pub semantic: Option<f64>,
    /// The lexical component, from full-text ranking.
    pub lexical: f64,
    /// What the query and the memory are both about, rarity-weighted.
    pub entity: f64,
    /// How often and how recently it has been needed.
    pub frecency: f64,
    /// Confidence, as held.
    pub confidence: f64,
    /// Strength at the moment of the search.
    pub strength: f64,
    /// Whether it came from the nearer store.
    pub near: bool,
}

/// How much of the question a result actually answered.
///
/// Only the query-relative signals. Confidence and strength say how much the memory is worth in
/// general; they say nothing about whether it answers what was asked, and folding them in is
/// how a certain, well-used, irrelevant fact outranks a hesitant, faded, correct one.
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

/// How much each signal counts.
///
/// Configuration overrides these; they are what aeon does when nothing has been said. Written
/// as a struct rather than as constants in the sum so the weighting can be read without
/// reading the arithmetic, and so a caller can change one without restating the rest.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weights {
    /// Cosine similarity, when an embedding exists on both sides.
    pub semantic: f64,
    /// Full-text ranking. The floor, always present.
    pub lexical: f64,
    /// How often and how recently a memory has actually been needed.
    pub frecency: f64,
    /// How sure.
    pub confidence: f64,
    /// How faded.
    pub strength: f64,
    /// Whether the project store outranks the global one.
    pub scope: f64,
    /// What the query and the memory are both *about*.
    ///
    /// Words and things come apart exactly where it matters: `deployment` shares no token with
    /// `we deploy with fly`, and both are about *fly*.
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
    /// The same weighting with the semantic share given to the lexical one.
    ///
    /// What aeon uses when nothing is embedded. Redistributing rather than dropping keeps the
    /// remaining signals summing to one, so a score means the same thing either way — without
    /// it, every result on a store with no vectors would score 30% lower for no reason.
    #[must_use]
    pub fn without_vectors(self) -> Self {
        Self {
            lexical: self.lexical + self.semantic,
            semantic: 0.0,
            ..self
        }
    }

    /// Every weight summed, for the tests that keep the set honest.
    #[must_use]
    pub fn total(self) -> f64 {
        self.semantic + self.lexical + self.entity + self.frecency + self.confidence + self.strength
    }
}

/// Access count at which the frequency half of frecency is most of the way to full.
///
/// Saturating rather than linear: the difference between being needed once and ten times is
/// large, and between a hundred and a thousand it is not.
const FREQUENT: f64 = 8.0;

/// How long an access stays fresh, in days.
const RECENT_DAYS: f64 = 7.0;

/// How often and how recently a memory has actually been needed, in `0..1`.
///
/// Frecency as editors and browsers rank with it: what somebody keeps returning to outranks
/// what merely matches, independently of the words in it. A memory created and never recalled
/// scores 0.5 — full access-recency, no frequency — so fresh candidates start level rather
/// than at the bottom.
#[must_use]
pub fn frecency(access_count: u32, last_accessed: Timestamp, now: Timestamp) -> f64 {
    let frequency = 1.0 - (-f64::from(access_count) / FREQUENT).exp();
    let days = ((now - last_accessed).max(0)) as f64 / 86_400.0;
    let recency = (-days / RECENT_DAYS).exp();
    (frequency + recency) / 2.0
}

/// Cosine similarity of two vectors, or `None` when they cannot be compared.
///
/// Different lengths mean different models, and comparing across them produces a number that
/// means nothing. Answering `None` is what lets the caller fall back to lexical rather than
/// rank on noise.
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
    // Mapped from `-1..1` into `0..1`, because every other signal lives there and a term that
    // could go negative would let one axis veto all the others.
    Some(((dot / (left.sqrt() * right.sqrt())) + 1.0) / 2.0)
}

/// Words that match everything and therefore mean nothing.
///
/// FTS5's `unicode61` tokenizer has no stopword list, so `the` is a term like any other — and
/// a question containing it matched every memory containing it, which is most of them. That is
/// how "what is the production database password" came back with the test command.
const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "been", "but", "by", "can", "did", "do", "does",
    "for", "from", "had", "has", "have", "how", "i", "if", "in", "is", "it", "its", "me", "my",
    "no", "not", "of", "on", "or", "our", "should", "so", "than", "that", "the", "their", "them",
    "then", "there", "these", "this", "those", "to", "was", "we", "were", "what", "when", "where",
    "which", "who", "why", "will", "with", "would", "you", "your",
];

/// A person's words as something FTS5 will accept.
///
/// Every term quoted and joined with `OR`. Unquoted input is a syntax the user did not ask to
/// be writing: a bare `-` or `*` is an operator to FTS5 and a typo to everyone else, and a
/// search that errors on an apostrophe is a search nobody trusts.
///
/// Stopwords are dropped. A query made of nothing else asks nothing, and answers nothing —
/// which is the correct behaviour and not an empty result to apologise for.
pub(crate) fn fts_query(query: &str) -> String {
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
        // A term nothing can match, rather than a syntax error or a match on everything.
        return "\"\u{0}nothing\"".to_owned();
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

/// What share of the query's words the memory actually contains.
///
/// The absolute half of the lexical signal. Without it a memory matching one word out of two
/// is indistinguishable from one matching both, which is exactly how a question about the
/// production box was answered with the staging box's address.
///
/// A query with nothing to ask about is neutral rather than zero: it did not fail to match,
/// there was nothing to match.
#[must_use]
pub fn coverage(wanted: &[String], text: &str) -> f64 {
    if wanted.is_empty() {
        return 1.0;
    }
    // Stemmed on both sides, because the layer that found the row stems too. Raw containment
    // scored a memory about `make test` at zero for a question about `tests` — the retrieval
    // stage matched it and the scoring stage said it had matched nothing.
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

/// A crude stem, applied to both sides so they agree.
///
/// Not a linguistic claim — a way to make the scoring stage agree with the retrieval stage,
/// which uses FTS5's porter tokenizer. Being consistently wrong about `running` costs nothing;
/// being inconsistent about `tests` cost a whole category.
fn stem(word: &str) -> String {
    for suffix in ["ing", "ed", "es", "s"] {
        if word.len() > suffix.len() + 2 && word.ends_with(suffix) && !word.ends_with("ss") {
            return word[..word.len() - suffix.len()].to_owned();
        }
    }
    word.to_owned()
}

/// One bm25 rank against the best in its result set, as a `0..1` where more is better.
///
/// Both numbers are negative and more-negative is better, so the ratio is already the right way
/// round. A set in which nothing scored at all is neutral rather than zero: letting an absent
/// signal push every candidate to the bottom of one axis lets the other axes decide by default.
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
        // A fresh memory has full access-recency and no frequency. Scoring it at zero would
        // bury everything new under everything old.
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
        // Every other signal lives in 0..1. A term that could go negative would let one axis
        // veto all the others.
        let value = cosine(&[1.0, 2.0], &[-3.0, -1.0]).expect("comparable");
        assert!((0.0..=1.0).contains(&value), "{value}");
    }

    #[test]
    fn vectors_from_different_models_are_not_compared() {
        // A number produced from mismatched dimensions means nothing, and nothing downstream
        // would notice it was nonsense.
        assert_eq!(cosine(&[1.0, 0.0], &[1.0, 0.0, 0.0]), None);
        assert_eq!(cosine(&[], &[]), None);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), None);
    }

    #[test]
    fn dropping_vectors_keeps_the_weighting_summing_to_one() {
        // Without redistributing, every result on an unembedded store would score 30% lower
        // for no reason, and a threshold tuned on one store would be wrong on the other.
        let with = Weights::default();
        let without = with.without_vectors();
        assert!((with.total() - without.total()).abs() < 1e-9);
        assert_eq!(without.semantic, 0.0);
    }
}
