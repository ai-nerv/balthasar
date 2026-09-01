//! When two claims are the same claim said differently.
//!
//! CALLUS used to corroborate on an exact digest, so "we use make test" in one run and "run make
//! test instead" in another were two claims seen once each — neither reaching the two distinct
//! sessions corroboration needs, so neither crossed. This is what closes that.
//!
//! # Why not the embedder
//!
//! The obvious answer is cosine similarity on the vectors memo already computes. It was measured
//! and it does not work. On the local hashed embedder, two claims that *contradict* each other
//! score higher than most true paraphrases:
//!
//! ```text
//!   0.537  "we use make test"               / "run make test instead"      same claim
//!   0.692  "we deploy with fly.io"          / "we deploy with heroku"      contradiction
//!   0.727  "the staging box is at 10.0.0.7" / "the production box is at 10.0.0.8"
//! ```
//!
//! There is no threshold in that. Character n-grams measure how a sentence is *spelled*, and two
//! claims about deployment are spelled alike whichever host they name — the words carrying the
//! meaning are a few characters out of hundreds. A merge here manufactures corroboration, which
//! is the exact failure the per-source evidence rule exists to prevent, so a signal that cannot
//! separate a claim from its own contradiction may not be the one used.
//!
//! # What is used instead
//!
//! The content words, stemmed, compared as sets. Grammar carries the phrasing and content words
//! carry the claim, so dropping the first is what makes rewording invisible and changing a value
//! loud: `fly.io` and `heroku` are both content, so swapping them halves the overlap.
//!
//! Measured on the same corpus, the two populations separate with room to spare — every
//! rewording at or above 0.667, every different claim at or below 0.500 — and `calibration.rs`
//! holds that gap open.

use memo_store::Cluster;

/// How much of what two claims are about must be shared to call them one claim.
///
/// Set between the measured populations rather than at a round number, and nearer the safe side
/// of the gap: a missed corroboration costs one delayed promotion, and a false one puts a claim
/// in the project's memory that no run actually made.
const SAME_CLAIM: f32 = 0.6;

/// How many clusters one pass will compare against each other.
///
/// The comparison is quadratic, so this is a wall-clock budget rather than a correctness one —
/// newest first, and anything missed is found by the next pass, exactly as with the run cap.
const COMPARED: usize = 300;

/// Words that carry phrasing rather than claim.
///
/// Deliberately short. Every word here is a word two different claims are allowed to share for
/// free, so a long list makes unrelated things look alike — which is the direction that hurts.
const GRAMMAR: &[&str] = &[
    "a", "an", "the", "is", "are", "was", "were", "be", "been", "to", "of", "in", "on", "at",
    "for", "with", "and", "or", "we", "you", "i", "it", "this", "that", "these", "those", "our",
    "your", "instead", "before", "after", "then", "when", "always", "never", "should", "must",
    "do", "does", "did", "run", "use", "uses", "using", "by", "from", "not", "no", "its", "if",
    "so", "just", "please", "keep", "keeps",
];

/// A cluster, and whether it took a near match to assemble.
#[derive(Debug, Clone, PartialEq)]
pub struct Akin {
    /// The claim and the runs that made it.
    pub cluster: Cluster,
    /// Whether two differently-worded claims were treated as one.
    ///
    /// Carried into the witness note, so `memo why` can say the corroboration was a rewording
    /// rather than a repeat and a person can disagree with it.
    pub near: bool,
}

/// Fold clusters that say the same thing into one another.
///
/// Greedy and order-dependent by design: clusters arrive with the most-witnessed first, so a
/// rewording joins the best-established statement of the claim rather than founding a rival one.
#[must_use]
pub fn merge(clusters: Vec<Cluster>) -> Vec<Akin> {
    let mut out: Vec<Akin> = Vec::new();

    for cluster in clusters {
        let joined = out
            .iter_mut()
            .take(COMPARED)
            .find(|held| same_claim(&held.cluster.text, &cluster.text));

        match joined {
            Some(held) => {
                // Only a different digest is a rewording. Identical text folding together is an
                // exact repeat, and a witness note claiming otherwise would be a lie about the
                // one thing this flag exists to report.
                let reworded = held.cluster.hash != cluster.hash;
                for session in cluster.sessions {
                    if !held.cluster.sessions.contains(&session) {
                        held.cluster.sessions.push(session);
                    }
                }
                held.cluster.first_seen = held.cluster.first_seen.min(cluster.first_seen);
                held.near |= reworded;
            }
            None => out.push(Akin {
                cluster,
                near: false,
            }),
        }
    }
    out
}

/// Whether two claims are one claim worded differently.
///
/// Two conditions, and the second is the one that makes this safe to act on. Overlap alone puts
/// "the api key is in .env.local" and "…in .env.production" at 0.5, which is close enough to the
/// threshold to be uncomfortable — so anything the revision rule reads as *same subject, changed
/// value* is refused outright, however similar it looks. A claim and its own replacement must
/// never corroborate each other.
#[must_use]
pub fn same_claim(a: &str, b: &str) -> bool {
    if memo_model::same_claim_different_value(a, b) {
        return false;
    }
    claim_overlap(a, b) >= SAME_CLAIM
}

/// The share of content words two claims have in common.
///
/// Named for claims to keep it apart from the entity overlap relations are built on: that one
/// asks what two memories are *about*, this one asks whether they say the same thing.
#[must_use]
pub fn claim_overlap(a: &str, b: &str) -> f32 {
    let (x, y) = (content(a), content(b));
    if x.is_empty() || y.is_empty() {
        return 0.0;
    }
    let shared = x.iter().filter(|word| y.contains(*word)).count() as f32;
    let union = x.len() + y.len() - shared as usize;
    if union == 0 {
        return 0.0;
    }
    shared / union as f32
}

/// What a claim is about: its words, less grammar, stemmed, without repeats.
fn content(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in memo_model::normalised(text).split_whitespace() {
        if GRAMMAR.contains(&word) {
            continue;
        }
        let stemmed = stem(word);
        if !stemmed.is_empty() && !out.contains(&stemmed) {
            out.push(stemmed);
        }
    }
    out
}

/// A crude stem: enough that `commit` and `committing` are one word.
///
/// Not a linguist's stemmer and not trying to be. What it has to fix is the case the corpus
/// actually produced — an English suffix making two spellings of one verb miss each other — and
/// a longer rule set would start merging words that mean different things.
fn stem(word: &str) -> String {
    let mut held = word.to_owned();
    for suffix in ["ing", "ed", "es", "s"] {
        if held.len() > suffix.len() + 2 && held.ends_with(suffix) {
            held.truncate(held.len() - suffix.len());
            break;
        }
    }
    // `committ` -> `commit`. A consonant doubled before a suffix is English spelling, not a
    // different word.
    let letters: Vec<char> = held.chars().collect();
    if letters.len() > 2 && letters[letters.len() - 1] == letters[letters.len() - 2] {
        held.pop();
    }
    held
}

#[cfg(test)]
mod tests {
    use super::*;
    use memo_model::SessionId;

    fn cluster(text: &str, sessions: &[&str]) -> Cluster {
        Cluster {
            text: text.to_owned(),
            hash: memo_model::content_hash(text),
            sessions: sessions.iter().map(|s| SessionId::new(*s)).collect(),
            first_seen: 0,
        }
    }

    #[test]
    fn two_runs_wording_it_differently_corroborate() {
        // The whole point. Before this each of these was one claim seen once, so neither
        // reached the two distinct sessions CALLUS needs and neither crossed.
        let merged = merge(vec![
            cluster("we use make test", &["01A"]),
            cluster("run make test instead", &["01B"]),
        ]);
        assert_eq!(merged.len(), 1, "one claim: {merged:?}");
        assert_eq!(merged[0].cluster.sessions.len(), 2);
        assert!(merged[0].near, "and it says the match was a rewording");
    }

    #[test]
    fn a_claim_never_corroborates_its_own_replacement() {
        // The failure that matters. These are the same subject with a different value, which is
        // a revision — and a store that took it as agreement would hold a fact no run stated.
        for (a, b) in [
            ("we deploy with fly.io", "we deploy with heroku"),
            (
                "the api key is in .env.local",
                "the api key is in .env.production",
            ),
            (
                "never commit to main directly",
                "never commit to develop directly",
            ),
            (
                "run make test before pushing",
                "run make test after pulling",
            ),
        ] {
            assert!(!same_claim(a, b), "{a:?} must not corroborate {b:?}");
            let merged = merge(vec![cluster(a, &["01A"]), cluster(b, &["01B"])]);
            assert_eq!(merged.len(), 2, "{a:?} / {b:?}");
        }
    }

    #[test]
    fn different_subjects_stay_apart() {
        for (a, b) in [
            ("we use make test", "we use cargo build"),
            ("the database is postgres", "the cache is redis"),
            (
                "migrations live in db/migrate",
                "fixtures live in db/fixtures",
            ),
            (
                "the staging box is at 10.0.0.7",
                "the production box is at 10.0.0.8",
            ),
            (
                "run make fmt before committing",
                "run make lint after committing",
            ),
        ] {
            assert!(!same_claim(a, b), "{a:?} must not merge with {b:?}");
        }
    }

    #[test]
    fn an_exact_repeat_is_not_reported_as_a_near_match() {
        // `near` drives what the witness note says, so it has to mean something. An identical
        // claim from two runs is a repeat and must read as one.
        let merged = merge(vec![
            cluster("we use make test", &["01A"]),
            cluster("we use make test", &["01B"]),
        ]);
        assert_eq!(merged.len(), 1);
        assert!(!merged[0].near);
    }

    #[test]
    fn a_run_saying_it_twice_is_still_one_run() {
        // Merging must not manufacture the diversity it is being counted for.
        let merged = merge(vec![
            cluster("we use make test", &["01A"]),
            cluster("run make test instead", &["01A"]),
        ]);
        assert_eq!(merged.len(), 1);
        assert_eq!(
            merged[0].cluster.sessions.len(),
            1,
            "one session, however many ways it said it"
        );
    }

    #[test]
    fn a_claim_with_nothing_in_it_matches_nothing() {
        assert!(!same_claim("", ""));
        assert!(!same_claim("the it is", "of and to"));
        assert_eq!(claim_overlap("", "we use make"), 0.0);
    }
}
