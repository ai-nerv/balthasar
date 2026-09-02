//! What kind of question is being asked.
//!
//! Classification changes which candidates are *generated*, never what relevance means. A
//! temporal query and an entity query walk different edges and then score the results the same
//! way — because if classification could also change the scoring, a misclassified query would
//! be wrong twice and there would be no way to tell which half failed.
//!
//! Rules, and deliberately transparent ones. A learned classifier would be more accurate and
//! would make `--explain` say "the model thought so", which is not an explanation. The rules
//! are a short list of markers anybody can read, extend, and argue with.

use memo_model::Family;

/// What a query is reaching for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// When, and in what order. "what did we do before the release"
    Temporal,
    /// Why it broke and what fixed it. "why did the build fail"
    Causal,
    /// What is known about a thing. "what do we know about the deploy script"
    Entity,
    /// Whether something like this has been solved. "have we hit this before"
    Semantic,
    /// What is true now. "what is the deploy target"
    Current,
    /// How something should be done. "how do we run the tests"
    Procedural,
    /// Nothing in particular.
    Plain,
}

impl Shape {
    /// Which relationship families are worth traversing for this shape.
    ///
    /// Empty means none — a plain lexical query gets the existing behaviour and pays nothing
    /// for the machinery it is not using.
    #[must_use]
    pub fn families(self) -> &'static [Family] {
        match self {
            Self::Temporal => &[Family::Temporal],
            Self::Causal => &[Family::Causal, Family::Temporal],
            Self::Entity => &[Family::Entity],
            Self::Semantic => &[Family::Semantic, Family::Entity],
            // What is true now is answered by slots and validity, not by walking outward.
            Self::Current => &[],
            // A procedure is reached through what it is about and what repaired what.
            Self::Procedural => &[Family::Entity, Family::Causal],
            Self::Plain => &[],
        }
    }

    /// The word this is spelled with.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Temporal => "temporal",
            Self::Causal => "causal",
            Self::Entity => "entity",
            Self::Semantic => "semantic",
            Self::Current => "current",
            Self::Procedural => "procedural",
            Self::Plain => "plain",
        }
    }

    /// Why this shape was chosen, for `--explain`.
    #[must_use]
    pub fn because(self) -> &'static str {
        match self {
            Self::Temporal => "the query asks about order or time",
            Self::Causal => "the query asks why something happened or what fixed it",
            Self::Entity => "the query asks what is known about a thing",
            Self::Semantic => "the query asks whether something like this was solved before",
            Self::Current => "the query asks what is true now",
            Self::Procedural => "the query asks how something is done",
            Self::Plain => "no marker matched, so nothing beyond search was walked",
        }
    }
}

/// Work out what a query is reaching for.
///
/// Checked in order of how specific each marker set is, so that "why did the build fail before
/// the release" reads as causal rather than temporal — the more specific reading wins, and the
/// order below *is* the specificity ranking rather than an accident of how it was written.
#[must_use]
pub fn shape_of(query: &str) -> Shape {
    let text = format!(" {} ", query.trim().to_lowercase());
    let has = |markers: &[&str]| markers.iter().any(|m| text.contains(m));

    if has(&[
        " why ",
        " what broke",
        " what fixed",
        " caused",
        " because",
        " failing",
        " failed",
        " root cause",
        " went wrong",
    ]) {
        return Shape::Causal;
    }
    if has(&[
        " how do ",
        " how does ",
        " how to ",
        " how should ",
        " steps ",
        " procedure ",
        " the way we ",
        " workflow ",
    ]) {
        return Shape::Procedural;
    }
    if has(&[
        " before ",
        " after ",
        " when did",
        " when we",
        " last time",
        " previously ",
        " yesterday ",
        " earlier ",
        " since ",
        " history ",
        " timeline ",
    ]) {
        return Shape::Temporal;
    }
    if has(&[
        " like this",
        " similar",
        " anything like",
        " have we hit",
        " seen this",
        " same problem",
        " again ",
    ]) {
        return Shape::Semantic;
    }
    if has(&[
        " what do we know",
        " everything about",
        " about the ",
        " related to ",
        " anything on ",
    ]) {
        return Shape::Entity;
    }
    if has(&[
        " what is ",
        " what's ",
        " current ",
        " right now",
        " these days",
        " nowadays ",
    ]) {
        return Shape::Current;
    }
    Shape::Plain
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn why_something_broke_is_a_causal_question() {
        assert_eq!(shape_of("why did the build fail"), Shape::Causal);
        assert_eq!(shape_of("what fixed the deploy"), Shape::Causal);
    }

    #[test]
    fn order_words_ask_about_time() {
        assert_eq!(
            shape_of("what did we do before the release"),
            Shape::Temporal
        );
        assert_eq!(shape_of("when did we last touch this"), Shape::Temporal);
    }

    #[test]
    fn a_specific_reading_beats_a_general_one() {
        // "why ... before ..." carries both a causal and a temporal marker. Causal is the more
        // specific reading and has to win, or every causal question with a date in it would be
        // answered by walking the wrong family.
        assert_eq!(
            shape_of("why did the build fail before the release"),
            Shape::Causal
        );
    }

    #[test]
    fn how_something_is_done_reaches_for_procedure() {
        assert_eq!(shape_of("how do we run the tests"), Shape::Procedural);
        assert_eq!(shape_of("how should this be deployed"), Shape::Procedural);
    }

    #[test]
    fn what_is_true_now_does_not_walk_outward() {
        // Slots and validity answer this. Traversing would add candidates that are related to
        // the current answer rather than being it.
        assert_eq!(shape_of("what is the deploy target"), Shape::Current);
        assert!(Shape::Current.families().is_empty());
    }

    #[test]
    fn an_ordinary_search_pays_nothing_for_machinery_it_does_not_use() {
        assert_eq!(shape_of("deploy target"), Shape::Plain);
        assert!(Shape::Plain.families().is_empty());
    }

    #[test]
    fn a_causal_question_also_walks_time() {
        // What fixed a failure is usually near it. Causal edges are sparse, so the temporal
        // family is carried along as the fallback rather than as the answer.
        assert!(Shape::Causal.families().contains(&Family::Causal));
        assert!(Shape::Causal.families().contains(&Family::Temporal));
    }

    #[test]
    fn every_shape_can_say_why_it_was_chosen() {
        for shape in [
            Shape::Temporal,
            Shape::Causal,
            Shape::Entity,
            Shape::Semantic,
            Shape::Current,
            Shape::Procedural,
            Shape::Plain,
        ] {
            assert!(!shape.because().is_empty(), "{shape:?}");
            assert!(!shape.as_str().is_empty());
        }
    }

    #[test]
    fn classification_is_a_pure_function_of_the_query() {
        // No clock, no store, no configuration. Two identical queries classify identically on
        // every machine, which is what makes a per-query-type benchmark comparable.
        for query in ["why did it fail", "before the release", "deploy target"] {
            assert_eq!(shape_of(query), shape_of(query));
        }
    }

    #[test]
    fn a_marker_inside_a_word_does_not_count() {
        // " why " is bounded by spaces, so "somewhy" and "anywhere" cannot classify a query.
        assert_eq!(shape_of("anywhere the deploy goes"), Shape::Plain);
    }
}
