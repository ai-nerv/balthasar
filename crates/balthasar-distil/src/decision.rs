//! DECISION: the person stating a choice or changing one, a quantity that replaces a quantity
//! the project holds, and a bare value the next reply links to what it replaced.
//!
//! ```text
//!   let's go with zstd for the transport          a choice
//!   bump the retries to 7                         replaces "the job makes 3 retries"
//!   person: zstd   assistant: zstd replaces gzip  replaces "we compress with gzip"
//! ```

use crate::{Candidate, Observation, Role};
use balthasar_model::{Body, MemoryId, NoteKind, ScopeId, Tier, Timestamp, WitnessKind};
use balthasar_store::{Recall, Store};

/// Phrases that state a choice.
const CHOICES: &[&str] = &[
    "let's use",
    "lets use",
    "let's go with",
    "lets go with",
    "let's stick with",
    "lets stick with",
    "we'll use",
    "we will use",
    "we'll go with",
    "we will go with",
    "we're using",
    "we are using",
    "we're going with",
    "we decided to",
    "we decided on",
    "we chose",
    "we picked",
    "we settled on",
];

/// Verbs that set a quantity to a new value.
const SETS: &[&str] = &[
    "bump", "raise", "lower", "increase", "decrease", "set", "change", "make",
];

/// Words that sit between a replacement and what it replaced.
const FILLER: &[&str] = &[
    "the", "earlier", "previous", "old", "former", "of", "a", "an",
];

/// The shortest and longest sentence read as a decision.
const CLAIM: std::ops::RangeInclusive<usize> = 8..=200;

/// How many words a reply may be and still be a bare value.
const BARE: usize = 3;

/// How many memories are looked at for one quantity or value.
const LOOKED_AT: usize = 8;

/// Every decision in these turns, and what each replaces when the turn said so.
#[must_use]
pub fn decisions(
    store: &Store,
    scope: &ScopeId,
    turns: &[Observation],
    now: Timestamp,
    extra: &[String],
) -> Vec<Candidate> {
    let mut out = Vec::new();
    for (index, turn) in turns.iter().enumerate() {
        if turn.role != Role::User || !turn.kind.can_instruct() {
            continue;
        }
        let Some(spoken) = crate::extract::spoken(&turn.text) else {
            continue;
        };
        let bare = spoken.split_whitespace().count() <= BARE && !spoken.contains('?');
        if bare {
            out.extend(bare_value(
                store,
                &spoken,
                turns.get(index + 1..).unwrap_or(&[]),
                now,
            ));
            continue;
        }
        for said in sentences(&spoken) {
            if matches!(store.what_this_revises(scope, &said), Ok(Some(_))) {
                continue;
            }
            if let Some(found) = quantity_change(store, &said, now) {
                out.push(found.at(turn.cursor));
            } else if chosen(&said, extra) || balthasar_model::announces_change(&said) {
                out.push(choice(&said, "decision").at(turn.cursor));
            }
        }
    }
    out
}

fn choice(text: &str, from: &str) -> Candidate {
    Candidate::new(
        Body::note(text, NoteKind::Claim),
        Tier::Fact,
        WitnessKind::Choice,
        from,
    )
}

/// A sentence that sets a quantity the project holds to another number: replaced when it says
/// so ("bump", "set", a change marker), disputed otherwise.
fn quantity_change(store: &Store, said: &str, now: Timestamp) -> Option<Candidate> {
    let (number, unit) = quantity(said)?;
    let held = holding(store, &unit, now, |text| {
        quantity(text).is_some_and(|(n, u)| u == unit && n != number)
    })?;
    let candidate = choice(said, "decision about a quantity");
    let sets = words(said).iter().any(|w| SETS.contains(&w.as_str()))
        || balthasar_model::announces_change(said);
    Some(if sets {
        candidate.replacing(held.0)
    } else {
        candidate.disputing(held.0)
    })
}

/// A short reply whose answer names what it replaced: the held claim, with the new value.
fn bare_value(
    store: &Store,
    value: &str,
    after: &[Observation],
    now: Timestamp,
) -> Option<Candidate> {
    let reply = after
        .iter()
        .take_while(|turn| turn.role != Role::User)
        .find(|turn| turn.role == Role::Assistant)?;
    let value = words(value).into_iter().last()?;
    let old = replaced_in(&reply.text, &value)?;
    let (id, text) = holding(store, &old, now, |text| words(text).contains(&old))?;
    let revised = words_swapped(&text, &old, &value);
    Some(choice(&revised, "decision linked by the reply").replacing(id))
}

/// What a reply says `value` replaced: "X replaces the earlier Y", "from Y to X",
/// "replace Y with X", "X instead of Y".
fn replaced_in(reply: &str, value: &str) -> Option<String> {
    let tokens = words(reply);
    let at = tokens.iter().position(|t| t == value)?;
    let after = |from: usize| {
        tokens
            .get(from..)
            .unwrap_or_default()
            .iter()
            .find(|t| !FILLER.contains(&t.as_str()))
            .cloned()
    };
    let before = |back: usize| at.checked_sub(back).and_then(|i| tokens.get(i)).cloned();
    let old = match tokens.get(at + 1).map(String::as_str) {
        Some("replaces" | "replacing" | "instead" | "over") => after(at + 2),
        _ => match (before(1).as_deref(), before(3).as_deref()) {
            (Some("to"), Some("from")) => before(2),
            (Some("with"), Some(verb)) if verb.starts_with("replac") => before(2),
            _ => None,
        },
    }?;
    (old != value).then_some(old)
}

/// The first open fact or habit about `topic` whose text passes `wanted`.
fn holding(
    store: &Store,
    topic: &str,
    now: Timestamp,
    wanted: impl Fn(&str) -> bool,
) -> Option<(MemoryId, String)> {
    let mut ask = Recall::of(topic, now);
    ask.limit = LOOKED_AT;
    ask.tiers = vec![Tier::Fact, Tier::Habit];
    store
        .recall_current(&ask)
        .ok()?
        .into_iter()
        .map(|hit| (hit.memory().id.clone(), hit.memory().text()))
        .find(|(_, text)| wanted(text))
}

/// The number a sentence gives a unit: "7 retries", or "retries to 7".
fn quantity(text: &str) -> Option<(u64, String)> {
    let tokens = words(text);
    for (i, token) in tokens.iter().enumerate() {
        let Ok(number) = token.parse::<u64>() else {
            continue;
        };
        if let Some(unit) = tokens.get(i + 1).filter(|u| unit_like(u)) {
            return Some((number, singular(unit)));
        }
        let linked = i
            .checked_sub(1)
            .and_then(|k| tokens.get(k))
            .is_some_and(|w| matches!(w.as_str(), "to" | "is" | "of" | "at"));
        if let Some(unit) = i
            .checked_sub(2)
            .and_then(|k| tokens.get(k))
            .filter(|u| unit_like(u))
            && linked
        {
            return Some((number, singular(unit)));
        }
    }
    None
}

fn unit_like(word: &str) -> bool {
    word.len() >= 3 && word.chars().all(char::is_alphabetic) && !FILLER.contains(&word)
}

/// A unit without its plural ending, so "retry" and "retries" are one unit.
fn singular(word: &str) -> String {
    if let Some(stem) = word.strip_suffix("ies") {
        return format!("{stem}y");
    }
    word.strip_suffix('s')
        .filter(|stem| !stem.ends_with('s'))
        .unwrap_or(word)
        .to_owned()
}

/// Whether a sentence states a choice that no negation shortly before it takes back.
fn chosen(said: &str, extra: &[String]) -> bool {
    let lower = said.to_lowercase().replace('\u{2019}', "'");
    let phrases = CHOICES
        .iter()
        .copied()
        .chain(extra.iter().map(String::as_str));
    phrases.into_iter().any(|phrase| {
        lower.match_indices(phrase).any(|(at, _)| {
            let before = lower.get(at.saturating_sub(12)..at).unwrap_or_default();
            !["not", "n't", "never", "no "]
                .iter()
                .any(|n| before.contains(n))
        })
    })
}

/// A message's sentences that may be decisions: no questions, nothing too short or too long.
fn sentences(text: &str) -> Vec<String> {
    text.split(['\n', '!', ';'])
        .flat_map(|part| part.split(". "))
        .map(|s| s.trim().trim_end_matches('.').to_owned())
        .filter(|s| !s.contains('?') && CLAIM.contains(&s.len()))
        .collect()
}

/// Lowercased words, punctuation trimmed, `fly.io` and `make-test` kept whole.
fn words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '-' && c != '_')
                .trim_end_matches('.')
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// `text` with every whole word `old` written as `new`.
fn words_swapped(text: &str, old: &str, new: &str) -> String {
    text.split(' ')
        .map(|word| {
            let bare = word
                .trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '-' && c != '_');
            if bare.eq_ignore_ascii_case(old) {
                word.replacen(bare, new, 1)
            } else {
                word.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
