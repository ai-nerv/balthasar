//! Telling a correction from a second fact, when nobody named a slot.
//!
//! A slotted fact contradicts by construction: `(project, deploy_target)` holds one live
//! answer and the database refuses a second. Most of what people actually say carries no slot —
//! *"remember: we deploy to heroku"* is prose, and naming its subject and predicate takes a
//! person or a model.
//!
//! So the suite found balthasar asserting both *"we deploy to heroku"* and *"we deploy to fly.io"*
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
/// second fact — which is how the suite found balthasar asserting both answers at once.
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

/// Whether two claims are one claim worded differently.
///
/// Below, and beside [`same_claim_different_value`] on purpose: one asks whether a claim was
/// restated, the other whether it was replaced, and they are the two halves of the same
/// question. Being wrong in either direction is expensive — a missed restatement costs one
/// delayed promotion, a missed replacement asserts a fact nobody stated.
///
/// # Why not an embedder
///
/// Measured, on real weights, and rejected. `bge-small-en-v1.5` scores a true rewording at
/// 0.813 and a claim beside its own replacement at 0.801 — twelve thousandths apart. Dense
/// vectors move very little when one entity is swapped for another, which is exactly the
/// distinction this has to make, so no threshold on them can make it. The hashed embedder is
/// worse still: it puts the contradiction *above* the rewording.
///
/// What works is the content words: grammar carries phrasing and content carries the claim, so
/// dropping the first makes rewording invisible and changing a value loud. `fly.io` and
/// `heroku` are both content, so swapping them halves the overlap.
/// How much of what two claims are about must be shared to call them one claim.
///
/// Set between the measured populations rather than at a round number, and nearer the safe side
/// of the gap: a missed corroboration costs one delayed promotion, and a false one puts a claim
/// in the project's memory that no run actually made.
const SAME_CLAIM: f32 = 0.6;

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

/// Fold clusters that say the same thing into one another.
///
/// Greedy and order-dependent by design: clusters arrive with the most-witnessed first, so a
/// Whether two claims are one claim worded differently.
///
/// Two conditions, and the second is the one that makes this safe to act on. Overlap alone puts
/// "the api key is in .env.local" and "…in .env.production" at 0.5, which is close enough to the
/// threshold to be uncomfortable — so anything the revision rule reads as *same subject, changed
/// value* is refused outright, however similar it looks. A claim and its own replacement must
/// never corroborate each other.
#[must_use]
pub fn same_claim(a: &str, b: &str) -> bool {
    if same_claim_different_value(a, b) || substituted(a, b) {
        return false;
    }
    claim_overlap(a, b) >= SAME_CLAIM
}

/// Whether each claim says something the other does not.
///
/// The rule that does most of the work, and it fell out of looking at what every true rewording
/// in the corpus has in common: none of them substitutes. Restating something adds words,
/// drops words, or reorders them — one claim's content ends up a subset of the other's. Saying
/// a *different* thing swaps one content word for another, and both sides are then left holding
/// something the other lacks.
///
/// It is what overlap alone cannot see. `loud says the same thing` and `spread says the same
/// thing` share three content words out of five, which reads as a rewording by any threshold
/// and is two claims about two subjects. So is `situation number 0` beside `situation number 1`,
/// and `we deploy with fly.io` beside `we deploy with heroku`.
#[must_use]
fn substituted(a: &str, b: &str) -> bool {
    let (x, y) = (content(a), content(b));
    let only_in_a = x.iter().any(|word| !y.contains(word));
    let only_in_b = y.iter().any(|word| !x.contains(word));
    only_in_a && only_in_b
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
