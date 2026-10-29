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
/// The claims must share an opening run of at least two words, and with any shared ending it
/// must be at least half of the shorter claim. What differs between them must be a value: up to
/// [`VALUE_WORDS`] words on each side, or words added on one side only.
#[must_use]
pub fn same_claim_different_value(before: &str, after: &str) -> bool {
    let a = clause(before);
    let b = clause(after);
    if a.len() < 2 || b.len() < 2 {
        return false;
    }

    let shared = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    if shared < 2 {
        return false;
    }
    let (rest_a, rest_b) = (
        a.get(shared..).unwrap_or(&[]),
        b.get(shared..).unwrap_or(&[]),
    );
    let ending = rest_a
        .iter()
        .rev()
        .zip(rest_b.iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let shorter = a.len().min(b.len());
    if (shared + ending) * 2 < shorter {
        return false;
    }
    let middle_a = rest_a.get(..rest_a.len() - ending).unwrap_or(&[]);
    let middle_b = rest_b.get(..rest_b.len() - ending).unwrap_or(&[]);
    if middle_a == middle_b {
        return false;
    }
    if middle_a.is_empty() || middle_b.is_empty() {
        return middle_a.len().max(middle_b.len()) <= ADDED_WORDS;
    }
    middle_a.len() <= VALUE_WORDS && middle_b.len() <= VALUE_WORDS
}

/// How many words a value may take on each side of a revision.
const VALUE_WORDS: usize = 2;

/// How many words a revision may add when it replaces nothing.
const ADDED_WORDS: usize = 3;

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

/// [`words`], up to the end of the first clause: a word ending in `,` or `;` past the claim's
/// first two words closes it, and an aside after it is not part of the value.
fn clause(text: &str) -> Vec<String> {
    let mut kept = Vec::new();
    for raw in text.split_whitespace() {
        let word = raw
            .trim_matches(|c: char| !c.is_alphanumeric() && c != '/' && c != '.' && c != '_')
            .to_lowercase();
        if word.is_empty() {
            continue;
        }
        if kept.is_empty() && OPENERS.contains(&word.as_str()) {
            continue;
        }
        kept.push(word);
        if kept.len() > 2 && raw.ends_with([',', ';']) {
            break;
        }
    }
    kept
}

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
    if denies(a, b) || same_claim_different_value(a, b) || substituted(a, b) {
        return false;
    }
    claim_overlap(a, b) >= SAME_CLAIM
}

/// Words that make a claim say the opposite thing, rather than say it differently.
///
/// They stay in [`GRAMMAR`], because two claims that are *both* negative share them for free;
/// what cannot be free is one claim having one and the other not.
const NEGATORS: &[&str] = &["not", "no", "never", "none", "nor", "without", "cannot"];

/// Whether one of these claims denies what the other asserts.
///
/// Checked before overlap, and on purpose: negation is the one word that can leave two claims
/// reading almost identically while they cannot both be true. Dropping it as phrasing filed a
/// person's correction as agreement with the thing they were correcting.
#[must_use]
fn denies(a: &str, b: &str) -> bool {
    negative(a) != negative(b)
}

/// Contractions written without their apostrophe.
const CONTRACTED: &[&str] = &[
    "dont", "doesnt", "didnt", "isnt", "arent", "wasnt", "werent", "wont", "wouldnt", "cant",
    "couldnt", "shouldnt", "mustnt", "havent", "hasnt", "hadnt", "neednt",
];

/// Whether a claim is negated anywhere: a negating word, or a contraction ending in `n't`.
#[must_use]
pub fn negative(text: &str) -> bool {
    crate::normalised(text).split_whitespace().any(negates)
}

/// Whether one word negates.
fn negates(word: &str) -> bool {
    let word = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '\u{2019}');
    NEGATORS.contains(&word)
        || CONTRACTED.contains(&word)
        || word.ends_with("n't")
        || word.ends_with("n\u{2019}t")
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

/// What a claim is about, as its distinct content words.
#[must_use]
pub fn content_words(text: &str) -> Vec<String> {
    content(text)
}

/// What a claim is about: its words, less grammar, stemmed, without repeats.
fn content(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in crate::normalised(text).split_whitespace() {
        if GRAMMAR.contains(&word) || negates(word) {
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
    if let [_, .., before, last] = letters.as_slice()
        && before == last
    {
        held.pop();
    }
    held
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worst thing a memory can do with a correction is file it as agreement.
    #[test]
    fn a_claim_and_its_denial_are_not_one_claim() {
        for (said, denied) in [
            (
                "Deploys go out on Fridays.",
                "Deploys never go out on Fridays.",
            ),
            (
                "The cache is cleared on deploy.",
                "The cache is not cleared on deploy.",
            ),
            (
                "We run the linter in CI.",
                "We do not run the linter in CI.",
            ),
            (
                "Migrations are reversible.",
                "No migrations are reversible.",
            ),
        ] {
            assert!(
                !same_claim(said, denied),
                "`{denied}` was taken as another way of saying `{said}`"
            );
        }
    }

    #[test]
    fn a_contracted_denial_is_not_corroboration() {
        for (said, denied) in [
            ("we deploy on fridays", "we don't deploy on fridays"),
            ("we deploy on fridays", "we dont deploy on fridays"),
            ("we deploy on fridays", "we don\u{2019}t deploy on fridays"),
            ("the job retries", "the job won't retry"),
            ("the cache is shared", "the cache isn't shared"),
            ("you can force push to main", "you can't force push to main"),
            ("we keep the old schema", "we no longer keep the old schema"),
        ] {
            assert!(
                !same_claim(said, denied),
                "`{denied}` corroborated `{said}`"
            );
        }
        assert!(same_claim(
            "we don't deploy on fridays",
            "we do not deploy on fridays"
        ));
    }

    /// And the words that do that must still be free where they are not the difference.
    #[test]
    fn phrasing_words_are_still_shared_for_free() {
        assert!(same_claim(
            "we always run the linter in CI",
            "we always run the linter in ci"
        ));
        assert!(same_claim(
            "the release never ships without a changelog",
            "a release never ships without the changelog"
        ));
    }

    #[test]
    fn two_steps_of_one_process_are_two_claims() {
        assert!(!same_claim_different_value(
            "The build runs the linter first.",
            "The build runs the tests after the linter."
        ));
    }

    #[test]
    fn a_sentence_that_only_opens_the_same_way_revises_nothing() {
        for (held, said) in [
            ("we deploy with fly", "we deploy the worker separately"),
            (
                "the build is run with make",
                "the build is failing on CI again",
            ),
        ] {
            assert!(
                !same_claim_different_value(held, said),
                "{said} revised {held}"
            );
        }
    }

    #[test]
    fn a_value_inside_a_shared_ending_is_still_a_revision() {
        assert!(same_claim_different_value(
            "we run the tests with cargo test",
            "we run the tests with make test"
        ));
        assert!(same_claim_different_value(
            "the deploy target is heroku in staging",
            "the deploy target is fly.io in staging"
        ));
    }

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
