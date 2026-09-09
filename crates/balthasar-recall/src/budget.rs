//! Fitting what is worth saying into the room there is.

/// Roughly how many characters a token is.
pub const CHARS_PER_TOKEN: usize = 4;

/// Tokens, roughly, for a piece of text.
#[must_use]
pub fn tokens(text: &str) -> usize {
    text.len().div_ceil(CHARS_PER_TOKEN)
}

/// How many characters each weight is worth, given a total.
#[must_use]
pub fn share(total_chars: usize, weights: &[f64]) -> f64 {
    let sum: f64 = weights.iter().filter(|w| **w > 0.0).sum();
    if sum <= 0.0 {
        return 0.0;
    }
    total_chars as f64 / sum
}

/// Whether two rendered lines are near enough to be the same line.
///
/// Overlap alone is not enough: lines that disagree about an identity token are different claims.
#[must_use]
pub fn near_duplicate(a: &str, b: &str, threshold: f64) -> bool {
    let left = words(a);
    let right = words(b);
    if left.is_empty() || right.is_empty() {
        return false;
    }

    let shared = left.iter().filter(|w| right.contains(*w)).count();
    let smaller = left.len().min(right.len()) as f64;
    if shared as f64 / smaller < threshold {
        return false;
    }

    let only_in = |these: &[String], those: &[String]| -> bool {
        these
            .iter()
            .filter(|word| !those.contains(word))
            .any(|word| distinguishing(word))
    };
    !only_in(&left, &right) && !only_in(&right, &left)
}

/// Whether a word is the kind that tells two otherwise-identical claims apart.
fn distinguishing(word: &str) -> bool {
    word.chars()
        .any(|c| c.is_ascii_digit() || c == '/' || c == '.' || c == '_')
}

/// A line's words, lowercased, with punctuation and duplicates gone.
fn words(text: &str) -> Vec<String> {
    let mut out: Vec<String> = text
        .split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '/')
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Cut a line to fit, on a word boundary where there is one.
#[must_use]
pub fn fit(text: &str, chars: usize) -> Option<String> {
    if text.len() <= chars {
        return Some(text.to_owned());
    }
    // Below this there is no room for anything a reader could use.
    if chars < 24 {
        return None;
    }
    let cut = text[..chars - 1]
        .rfind(char::is_whitespace)
        .unwrap_or(chars - 1);
    Some(format!("{}…", text[..cut].trim_end()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_share_is_proportional_to_weight() {
        let per = share(1000, &[1.0, 4.0]);
        assert!((per * 1.0 - 200.0).abs() < 1.0);
        assert!((per * 4.0 - 800.0).abs() < 1.0);
    }

    #[test]
    fn a_configuration_that_declared_nothing_gets_nothing() {
        assert_eq!(share(1000, &[]), 0.0);
        assert_eq!(share(1000, &[0.0]), 0.0);
    }

    #[test]
    fn one_claim_restated_is_caught() {
        assert!(near_duplicate(
            "we run the tests with make test",
            "the tests are run with make test",
            0.8
        ));
    }

    #[test]
    fn two_claims_that_differ_only_in_a_number_are_not() {
        assert!(!near_duplicate(
            "situation number 0: do thing 0",
            "situation number 1: do thing 1",
            0.8
        ));
        assert!(!near_duplicate(
            "the build takes 40 seconds",
            "the build takes 90 seconds",
            0.8
        ));
    }

    #[test]
    fn two_claims_that_differ_only_in_a_path_are_not() {
        assert!(!near_duplicate(
            "central file src/a.rs",
            "central file src/b.rs",
            0.8
        ));
    }

    #[test]
    fn a_restatement_with_a_shared_number_is_still_a_restatement() {
        assert!(near_duplicate(
            "we pin rust 1.94 for the build",
            "the build pins rust 1.94",
            0.8
        ));
    }

    #[test]
    fn two_different_claims_are_not() {
        assert!(!near_duplicate("we use make", "we use cargo", 0.8));
        assert!(!near_duplicate(
            "the build takes 40 seconds",
            "the tests take 3 seconds",
            0.8
        ));
    }

    #[test]
    fn punctuation_does_not_make_two_lines_different() {
        assert!(near_duplicate("we use make.", "We use make!", 0.8));
    }

    #[test]
    fn nothing_is_a_duplicate_of_nothing() {
        assert!(!near_duplicate("", "anything", 0.8));
    }

    #[test]
    fn a_line_that_fits_is_left_alone() {
        assert_eq!(fit("short", 100).as_deref(), Some("short"));
    }

    #[test]
    fn a_line_is_cut_on_a_word_boundary() {
        let cut = fit(
            "the quick brown fox jumps over the lazy dog and keeps going",
            30,
        )
        .expect("something");
        assert!(cut.ends_with('…'), "{cut}");
        assert!(cut.len() <= 30, "{} is {}", cut, cut.len());
        assert!(!cut.contains("quic…"), "{cut}");
    }

    #[test]
    fn there_is_a_width_below_which_nothing_is_better_than_something() {
        assert_eq!(fit("a rather long line of prose", 10), None);
    }

    #[test]
    fn a_token_estimate_never_answers_zero_for_real_text() {
        assert_eq!(tokens(""), 0);
        assert_eq!(tokens("a"), 1);
        assert_eq!(tokens("abcd"), 1);
        assert_eq!(tokens("abcde"), 2);
    }
}
