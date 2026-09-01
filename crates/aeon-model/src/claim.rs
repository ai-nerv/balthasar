//! Telling a correction from a second fact, when nobody named a slot.
//!
//! A slotted fact contradicts by construction: `(project, deploy_target)` holds one live
//! answer and the database refuses a second. Most of what people actually say carries no slot —
//! *"remember: we deploy to heroku"* is prose, and naming its subject and predicate takes a
//! person or a model.
//!
//! So the suite found aeon asserting both *"we deploy to heroku"* and *"we deploy to fly.io"*
//! at once, a month apart, with nothing to tell them apart. The rule here is what fixes that:
//! **same lead-in, different tail** is one claim revised, not two claims held.

/// A claim's opening words, lowercased.
///
/// Two, because it is the shortest run that means anything and it makes an index key. It is a
/// bucket, not the decision — [`same_claim_different_value`] settles it.
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
/// Word-wise: the claims must share an opening run of at least two words, that run must be at
/// least half of the shorter claim, and what follows it must differ.
///
/// The half rule is what keeps *"the staging box is at 10.0.0.7"* and *"the production box is
/// at 10.0.0.8"* apart — they share one word before diverging, and they are two boxes rather
/// than one box that moved.
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
/// A person correcting themselves says *"no, we deploy with fly.io now"*. The claim is the same
/// claim; the opening word is about the speech act. Without stripping these the shared-prefix
/// rule starts comparing at word zero, finds nothing in common, and treats a correction as a
/// second fact — which is how the suite found aeon asserting both answers at once.
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

    // Only from the front, and only while they keep coming. A claim that *contains* one of
    // these words in the middle is a claim about that word.
    while held.first().is_some_and(|w| OPENERS.contains(&w.as_str())) {
        held.remove(0);
    }
    held
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_correction_that_opens_with_no_still_revises() {
        // Found by the extended suite: a person correcting themselves says "no, we deploy with
        // fly.io now". The shared-prefix rule started at word zero, found nothing in common,
        // and treated it as a second fact — so both answers were asserted at once.
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
        // Only the opening is stripped. "the no-cache header is required" is a claim about a
        // header, and eating its first word would make it match anything.
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
        // The half rule still applies afterwards, so two different subjects stay apart even
        // when both open with a marker.
        assert!(!same_claim_different_value(
            "no, the staging box is at 10.0.0.7",
            "actually the production box is at 10.0.0.8"
        ));
    }

    #[test]
    fn the_same_claim_with_a_new_value_is_a_revision() {
        // What the suite caught: both of these were asserted at once, a month apart.
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
        // One word in common before diverging. Two boxes, not one box that moved.
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
        // Saying the same thing twice is corroboration, and the content hash already has it.
        assert!(!same_claim_different_value(
            "we deploy to fly.io",
            "we deploy to fly.io"
        ));
    }

    #[test]
    fn a_longer_claim_that_merely_adds_is_still_a_revision() {
        // "we deploy to fly" then "we deploy to fly from main" — the tail changed, and the
        // later one is the fuller answer.
        assert!(same_claim_different_value(
            "we deploy to fly",
            "we deploy to fly from main"
        ));
    }

    #[test]
    fn two_words_are_not_enough_to_revise_a_long_claim() {
        // The half rule. A long claim sharing only its opening two words is a different
        // claim that happens to start the same way.
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
