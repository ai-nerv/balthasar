//! What a session may not be shown of what a session wrote.
//!
//! A memory a session proposed stands below the floor until somebody else bears it out, and is
//! offered with a warning to check it. That is enough for a fact and nothing for a rule, which is
//! followed whatever it is headed with: so a rule the person rejected, undid, or has not yet
//! reviewed came back to the next session as the last one's own words for it.

use crate::Answering;
use balthasar_model::{Memory, Through, Tier};
use std::collections::BTreeSet;

/// Words too common to say what a sentence is about.
const COMMON: &[&str] = &[
    "the", "and", "for", "with", "this", "that", "here", "from", "into", "its", "are", "was",
    "all", "any", "every", "each", "new", "not", "must", "should", "always", "never", "project",
    "use", "uses", "used", "when", "where", "which", "have", "has", "will", "only",
];

/// What the change log holds that the person has not let stand: waiting for review, rejected, or
/// taken back. Read once for a whole result set.
pub(crate) struct Standing {
    taken_back: Vec<BTreeSet<String>>,
}

/// What a sentence is about: its uncommon words, lowercased, a plural's `s` dropped.
fn about(text: &str) -> BTreeSet<String> {
    text.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|word| word.len() >= 3 && !COMMON.contains(word))
        .map(|word| word.strip_suffix('s').unwrap_or(word).to_owned())
        .collect()
}

/// A name nobody would write by chance: one word of it is a subject in common.
fn marked(word: &str) -> bool {
    word.contains('_') || word.chars().any(|c| c.is_ascii_digit())
}

impl Standing {
    pub(crate) fn of(at: &Answering<'_>) -> Self {
        let Some(scrollback) = at.scrollback.as_ref() else {
            return Self {
                taken_back: Vec::new(),
            };
        };
        // A rule proposed again while it stands is refused as a duplicate, and that refusal says
        // nothing against the rule: what a live note still says has not been taken back.
        let standing: Vec<BTreeSet<String>> = scrollback
            .notes()
            .unwrap_or_default()
            .iter()
            .map(|note| about(&note.text))
            .collect();
        let taken_back = scrollback
            .changes(500)
            .unwrap_or_default()
            .iter()
            .filter(|c| ["staged", "rejected", "undone"].contains(&c.state.as_str()))
            .filter_map(|c| c.after.as_ref().or(c.before.as_ref()))
            .filter_map(|fields| fields["text"].as_str())
            .map(about)
            .filter(|words| !words.is_empty() && !standing.contains(words))
            .collect();
        Self { taken_back }
    }

    /// Whether `memory` is kept from a session: nobody stands behind it, a session wrote it, and
    /// it either tells somebody what to do or is about something the person has not let stand.
    pub(crate) fn withholds(&self, at: &Answering<'_>, memory: &Memory) -> bool {
        if memory.tier == Tier::Scratch || memory.is_assertable(at.inject_floor, at.now, true) {
            return false;
        }
        let text = memory.text();
        if crate::notes::instructs(&text) {
            return true;
        }
        if memory.provenance.through != Through::Peer {
            return false;
        }
        let said = about(&text);
        self.taken_back.iter().any(|rule| {
            let shared: Vec<&String> = rule.intersection(&said).collect();
            shared.len() >= 2 || shared.iter().any(|word| marked(word))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{about, marked};

    #[test]
    fn what_a_sentence_is_about_is_its_uncommon_words() {
        let rule = about("Always start every function name in this project with the prefix zq_.");
        let fact = about("Function names begin with zq_ here.");
        let shared: Vec<_> = rule.intersection(&fact).cloned().collect();
        assert_eq!(shared, ["function", "name", "zq_"]);
        assert!(about("The storage keeps its index in index.db").is_disjoint(&rule));
    }

    #[test]
    fn a_name_is_marked_and_a_word_is_not() {
        assert!(marked("zq_") && marked("utf8"));
        assert!(!marked("function"));
    }
}
