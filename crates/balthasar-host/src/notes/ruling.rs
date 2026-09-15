//! What an extraction may keep: a pinned note only where a person laid a rule down, and never a
//! note that only restates one already kept.

use serde_json::Value;

/// Words a person lays a rule down with, rather than asking for one piece of work.
const RULE_WORDS: &[&str] = &[
    "rule", "rules", "always", "never", "must", "prefer", "not", "avoid", "remember", "dont",
];

/// Whether the person said anything rule-shaped in what an extraction read fresh. Only then may it
/// pin: a model asked to keep rules found one in every chapter request.
pub(super) fn laid_down(input: &str) -> bool {
    let fresh = input
        .split_once("New since then")
        .map_or(input, |(_, after)| after);
    fresh
        .lines()
        .filter(|line| line.contains("] person: "))
        .any(|line| {
            let line = line.to_lowercase().replace('\'', "");
            line.contains("from now on")
                || line.contains("we use")
                || line
                    .split(|c: char| !c.is_alphanumeric())
                    .any(|word| RULE_WORDS.contains(&word))
        })
}

/// The same ops for an extraction whose span laid no rule down: an add is kept unpinned, and an
/// update leaves the pinning as it was, since forcing it off unpinned a rule said long before.
pub(super) fn unpinned(ops: Vec<Value>) -> Vec<Value> {
    ops.into_iter()
        .map(|mut op| {
            if op["op"] == "add" {
                if op["pinned"] == Value::Bool(true) {
                    op["pinned"] = Value::Bool(false);
                }
            } else if let Some(fields) = op.as_object_mut() {
                fields.remove("pinned");
            }
            op
        })
        .collect()
}

/// Whether an added note only restates one already kept: either text holds the other, compared by
/// their words alone. Skipped rather than merged, since a merge would carry its pinning over.
pub(super) fn echoes(op: &Value, kept: &[balthasar_store::Note]) -> bool {
    let words = |text: &str| {
        text.to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { ' ' })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let Some(new) = op["text"].as_str().map(words).filter(|t| t.len() >= 16) else {
        return false;
    };
    kept.iter().any(|note| {
        let old = words(&note.text);
        old.len() >= 16 && (new.contains(&old) || old.contains(&new))
    })
}
