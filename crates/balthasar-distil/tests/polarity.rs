//! F2: the polarity corpus. Directives that must keep their negation, and sentences that use the
//! same words without instructing anything.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_distil::read_instruction;

/// Standing rules, each with the word its claim must keep.
const DIRECTIVES: &[(&str, &str)] = &[
    ("never commit straight to main", "never"),
    ("Never use sudo in the install script.", "never"),
    ("we never vendor node_modules", "never"),
    ("you should never edit generated files", "never"),
    ("please never print the token", "never"),
    ("ok so we always run make verify before a release", "always"),
    ("always pin the toolchain", "always"),
    ("Always use uv for python deps", "always"),
    ("from now on never push on fridays", "never"),
    ("remember: we never squash merge", "never"),
    ("don't forget we never rebase develop", "never"),
    ("and always write the changelog by hand", "always"),
];

/// The same words, narrating, reporting or asking. None of these is an instruction.
const NARRATIVE: &[&str] = &[
    "we never got round to it",
    "the tests always fail on CI",
    "it always failed on the old box",
    "never mind, it works now",
    "I can never remember which flag it is",
    "does it always fail like that?",
    "the build always takes forever on this laptop",
    "this never happened before the upgrade",
    "they never answered the issue",
    "the docs always lag behind the code",
    "I never liked that library",
    "it was always slow",
    "nobody ever reads it, it never gets updated",
    "the flaky test always passes on a rerun",
    "we always wondered why that was",
    "honestly it never worked well",
    "the cache never warmed up",
    "it always seemed odd to me",
    "the old server never had swap",
    "did it never occur to anyone?",
];

#[test]
fn a_directive_keeps_its_polarity() {
    for (said, word) in DIRECTIVES {
        let read = read_instruction(said, &[]).unwrap_or_else(|| panic!("missed: {said}"));
        let claim = read.claim.unwrap_or_else(|| panic!("no claim: {said}"));
        assert!(
            claim.to_lowercase().contains(word),
            "`{said}` lost `{word}`: {claim}"
        );
    }
}

#[test]
fn narrative_is_not_taken_for_an_instruction() {
    let taken: Vec<&str> = NARRATIVE
        .iter()
        .copied()
        .filter(|said| read_instruction(said, &[]).is_some())
        .collect();
    let precision = 1.0 - taken.len() as f64 / NARRATIVE.len() as f64;
    assert!(precision >= 0.95, "taken as instructions: {taken:?}");
}
