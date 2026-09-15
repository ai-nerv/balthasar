//! Telling a correction from a second fact, when nobody named a slot.
//!
//! Same lead-in, different tail is one claim revised, not two claims held.

/// A claim's opening words, lowercased.
///
/// Two words. A bucket, not the decision — [`same_claim_different_value`] settles it.
#[must_use]
pub fn lead(text: &str) -> String {
    words(text)
        .into_iter()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether the second claim revises the first rather than joining it.
///
/// The claims must share an opening run of at least two words, that run must be at least half
/// of the shorter claim, and what follows it must differ.
#[must_use]
pub fn same_claim_different_value(before: &str, after: &str) -> bool {
    let a = words(before);
    let b = words(after);
    if a.len() < 2 || b.len() < 2 {
        return false;
    }

    let shared = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    if shared < 2 {
        return false;
    }
    let shorter = a.len().min(b.len());
    if shared * 2 < shorter {
        return false;
    }
    a[shared..] != b[shared..]
}

/// Words that open a turn without being part of the claim it carries.
///
/// Stripped only from the front, so the shared-prefix rule starts at the claim itself.
const OPENERS: &[&str] = &[
    "no",
    "nope",
    "actually",
    "wait",
    "sorry",
    "correction",
    "remember",
    "note",
    "fyi",
    "also",
    "and",
    "but",
    "well",
    "hmm",
    "oh",
    "hey",
];

/// A claim's words, lowercased, with punctuation and opening markers removed.
fn words(text: &str) -> Vec<String> {
    let mut held: Vec<String> = text
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|c: char| !c.is_alphanumeric() && c != '/' && c != '.' && c != '_')
                .to_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect();

    while held.first().is_some_and(|w| OPENERS.contains(&w.as_str())) {
        held.remove(0);
    }
    held
}

/// How much of what two claims are about must be shared to call them one claim.
///
/// Set between the measured populations, nearer the safe side of the gap.
const SAME_CLAIM: f32 = 0.6;

/// Words that carry phrasing rather than claim.
///
/// Deliberately short: every word here is one two different claims may share for free.
const GRAMMAR: &[&str] = &[
    "a", "an", "the", "is", "are", "was", "were", "be", "been", "to", "of", "in", "on", "at",
    "for", "with", "and", "or", "we", "you", "i", "it", "this", "that", "these", "those", "our",
    "your", "instead", "before", "after", "then", "when", "always", "never", "should", "must",
    "do", "does", "did", "run", "use", "uses", "using", "by", "from", "not", "no", "its", "if",
    "so", "just", "please", "keep", "keeps",
];

/// Whether two claims are one claim worded differently.
///
/// Anything the revision rule reads as *same subject, changed value* is refused outright: a
/// claim and its own replacement must never corroborate each other.
///
/// A dense embedder cannot make this distinction — `bge-small-en-v1.5` scores a true rewording
/// at 0.813 and a claim beside its own replacement at 0.801.
#[must_use]
pub fn same_claim(a: &str, b: &str) -> bool {
    if same_claim_different_value(a, b) || substituted(a, b) {
        return false;
    }
    claim_overlap(a, b) >= SAME_CLAIM
}

/// Whether each claim says something the other does not.
///
/// No true rewording substitutes: restating adds, drops or reorders words, so one claim's
/// content ends up a subset of the other's. Saying a *different* thing swaps one content word
/// for another, and both sides are then left holding something the other lacks.
#[must_use]
fn substituted(a: &str, b: &str) -> bool {
    let (x, y) = (content(a), content(b));
    let only_in_a = x.iter().any(|word| !y.contains(word));
    let only_in_b = y.iter().any(|word| !x.contains(word));
    only_in_a && only_in_b
}

/// The share of content words two claims have in common.
///
/// Named for claims to keep it apart from the entity overlap relations are built on.
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
    for word in crate::normalised(text).split_whitespace() {
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
fn stem(word: &str) -> String {
    let mut held = word.to_owned();
    for suffix in ["ing", "ed", "es", "s"] {
        if held.len() > suffix.len() + 2 && held.ends_with(suffix) {
            held.truncate(held.len() - suffix.len());
            break;
        }
    }
    // `committ` -> `commit`: a doubled consonant before a suffix is spelling, not a new word.
    let letters: Vec<char> = held.chars().collect();
    if letters.len() > 2 && letters[letters.len() - 1] == letters[letters.len() - 2] {
        held.pop();
    }
    held
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_correction_that_opens_with_no_still_revises() {
        assert!(same_claim_different_value(
            "remember: we deploy with heroku",
            "no, we deploy with fly.io now"
        ));
        assert!(same_claim_different_value(
            "we deploy with heroku",
            "actually we deploy with fly.io"
        ));
    }

    #[test]
    fn a_marker_in_the_middle_of_a_claim_is_part_of_it() {
        assert_eq!(
            words("the no cache header"),
            vec!["the", "no", "cache", "header"]
        );
        assert_eq!(
            words("no, the cache header"),
            vec!["the", "cache", "header"]
        );
    }

    #[test]
    fn stripping_markers_does_not_merge_unrelated_claims() {
        assert!(!same_claim_different_value(
            "no, the staging box is at 10.0.0.7",
            "actually the production box is at 10.0.0.8"
        ));
    }

    #[test]
    fn the_same_claim_with_a_new_value_is_a_revision() {
        assert!(same_claim_different_value(
            "we deploy to heroku",
            "we deploy to fly.io"
        ));
        assert!(same_claim_different_value(
            "the version is 1.2",
            "the version is 2.0"
        ));
        assert!(same_claim_different_value("we use make", "we use cargo"));
    }

    #[test]
    fn two_claims_about_different_things_are_two_claims() {
        assert!(!same_claim_different_value(
            "the staging box is at 10.0.0.7",
            "the production box is at 10.0.0.8"
        ));
    }

    #[test]
    fn unrelated_claims_are_unrelated() {
        assert!(!same_claim_different_value(
            "we deploy to fly.io",
            "the tests run with make"
        ));
    }

    #[test]
    fn a_claim_does_not_revise_itself() {
        assert!(!same_claim_different_value(
            "we deploy to fly.io",
            "we deploy to fly.io"
        ));
    }

    #[test]
    fn a_longer_claim_that_merely_adds_is_still_a_revision() {
        assert!(same_claim_different_value(
            "we deploy to fly",
            "we deploy to fly from main"
        ));
    }

    #[test]
    fn two_words_are_not_enough_to_revise_a_long_claim() {
        assert!(!same_claim_different_value(
            "the build takes forty seconds on this machine",
            "the build system was replaced last year with bazel"
        ));
    }

    #[test]
    fn a_claim_too_short_to_have_a_lead_in_revises_nothing() {
        assert!(!same_claim_different_value("yes", "no"));
    }

    #[test]
    fn a_lead_is_the_first_two_words() {
        assert_eq!(lead("We Deploy to fly.io"), "we deploy");
        assert_eq!(lead("hi"), "hi");
        assert_eq!(lead(""), "");
    }
}
