//! How many answers a predicate may hold at once.
//!
//! `project deploy_target` holds one: a later answer replaces the earlier one, and the store
//! refuses to say both. `you likes` holds many: sushi and pizza are both true, and treating the
//! second as a correction of the first would quietly delete something nobody retracted.
//!
//! R-Mem's distinction, and it exposed a real defect here — the unique index was applied to
//! every fact, so aeon could not record that somebody liked two things.

/// Predicates that describe an accumulating set.
///
/// Matched on the whole word rather than by substring, so `disliked` is not read as `liked`.
const MANY: &[&str] = &[
    "likes",
    "liked",
    "loves",
    "enjoys",
    "prefers",
    "wants",
    "avoids",
    "dislikes",
    "uses",
    "used",
    "knows",
    "speaks",
    "has",
    "owns",
    "plays",
    "reads",
    "watches",
    "depends_on",
    "imports",
    "requires",
    "provides",
    "exports",
    "touches",
    "central_file",
    "slow_command",
    "friends_with",
    "colleague_of",
    "member_of",
    "tagged",
    "mentions",
];

/// Predicates that name one current answer, stated because guessing them wrong is expensive.
///
/// Anything not listed either way falls to [`is_single_valued`]'s default, which is *single* —
/// the conservative choice, because a wrongly-single predicate raises a visible constraint
/// error and a wrongly-many one silently accumulates contradictions nobody notices.
const ONE: &[&str] = &[
    "name",
    "pronouns",
    "timezone",
    "editor",
    "shell",
    "email",
    "deploy_target",
    "test_command",
    "build_command",
    "lint_command",
    "package_manager",
    "language",
    "runtime",
    "database",
    "works_at",
    "lives_in",
    "born_in",
    "role",
    "version",
];

/// Whether a predicate names one current answer.
///
/// The default is *single*, and deliberately so. A predicate wrongly marked single raises a
/// constraint error the moment a second answer arrives — loud, and fixed by adding a word to a
/// list. A predicate wrongly marked many accumulates contradictions in silence, and nothing
/// downstream ever notices.
#[must_use]
pub fn is_single_valued(predicate: &str) -> bool {
    let word = predicate.trim().to_lowercase();
    if MANY.iter().any(|m| *m == word) {
        return false;
    }
    if ONE.iter().any(|o| *o == word) {
        return true;
    }
    // A plural is usually an accumulating set: `tags`, `imports`, `follows`. Anything in ONE
    // has already returned above, so reaching here with a plural settles it.
    if word.ends_with('s') && !word.ends_with("ss") && word.len() > 3 {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_current_answer_holds_one() {
        for p in ["deploy_target", "test_command", "timezone", "works_at"] {
            assert!(is_single_valued(p), "{p}");
        }
    }

    #[test]
    fn an_accumulating_set_holds_many() {
        for p in [
            "likes",
            "uses",
            "depends_on",
            "central_file",
            "slow_command",
        ] {
            assert!(!is_single_valued(p), "{p}");
        }
    }

    #[test]
    fn a_plural_is_read_as_a_set() {
        assert!(!is_single_valued("tags"));
        assert!(!is_single_valued("follows"));
    }

    #[test]
    fn a_listed_singular_beats_the_plural_rule() {
        // `pronouns` is plural and names one answer.
        assert!(is_single_valued("pronouns"));
    }

    #[test]
    fn a_double_s_is_not_a_plural() {
        assert!(is_single_valued("address"));
    }

    #[test]
    fn an_unknown_predicate_is_single_by_default() {
        // The conservative direction: wrongly-single raises a visible constraint error;
        // wrongly-many accumulates contradictions nobody notices.
        assert!(is_single_valued("some_predicate_nobody_listed"));
    }

    #[test]
    fn a_negation_is_not_read_as_its_opposite() {
        // Substring matching would read `dislikes` as `likes`. Both are sets here, but the
        // rule has to be word-wise for the cases where they differ.
        assert!(!is_single_valued("dislikes"));
    }
}
