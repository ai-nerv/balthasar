//! Whether a sentence announces that something changed: the difference between restating a
//! claim and replacing one.

/// Phrases that announce a replacement. Verb forms only: "the migrations folder" is a noun.
const MARKERS: &[&str] = &[
    "switch to",
    "switched to",
    "switching to",
    "switch over",
    "switched over",
    "move to",
    "moved to",
    "moving to",
    "moved off",
    "moving off",
    "moved away from",
    "migrate to",
    "migrated to",
    "migrating to",
    "instead of",
    "no longer",
    "from now on",
    "going forward",
    "replaced by",
    "replaced with",
    "replace it with",
    "revert to",
    "reverted to",
    "roll back to",
    "rolled back to",
    "changed to",
    "change of plan",
    "is now",
    "are now",
    "now uses",
    "now use",
];

/// Words that, shortly before a marker, say the change did not happen.
const DENIED: &[&str] = &[
    "not", "no", "never", "didn't", "don't", "won't", "didnt", "dont",
];

/// Whether `text` says that something was replaced.
///
/// A standalone "now" counts too ("we deploy with fly.io now"), unless it opens a clause
/// ("now that"). A marker with a negation in the three words before it does not.
#[must_use]
pub fn announces_change(text: &str) -> bool {
    let words: Vec<String> = text
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'')
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect();
    let joined = format!(" {} ", words.join(" "));
    for marker in MARKERS {
        let needle = format!(" {marker} ");
        for (at, _) in joined.match_indices(&needle) {
            if !denied_before(&joined, at) {
                return true;
            }
        }
    }
    words.iter().enumerate().any(|(i, word)| {
        let before = words.get(i.saturating_sub(3)..i).unwrap_or_default();
        word == "now"
            && words.get(i + 1).is_none_or(|next| next != "that")
            && !before.iter().any(|w| DENIED.contains(&w.as_str()))
    })
}

/// Whether one of the three words before byte `at` negates.
fn denied_before(joined: &str, at: usize) -> bool {
    joined
        .get(..at)
        .unwrap_or_default()
        .split_whitespace()
        .rev()
        .take(3)
        .any(|word| DENIED.contains(&word))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_replacement_is_announced() {
        for said in [
            "we switched to zstd last week",
            "we moved off heroku, it's fly.io",
            "use make instead of cargo test",
            "we no longer vendor the sdk",
            "the default branch is now develop",
            "we deploy with fly.io now, since the migration",
        ] {
            assert!(announces_change(said), "{said}");
        }
    }

    #[test]
    fn a_noun_or_a_denial_is_not_a_change() {
        for said in [
            "the migrations folder is generated",
            "a replacement part arrived",
            "we did not switch to zstd",
            "we never moved to fly.io",
            "now that the tests pass, ship it",
            "the build is run with make",
        ] {
            assert!(!announces_change(said), "{said}");
        }
    }
}
