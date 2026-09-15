//! Whether what a person said laid a rule down: an extraction may pin a note only then.

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

/// The same ops with nothing pinned, for an extraction whose span laid no rule down.
pub(super) fn unpinned(ops: Vec<Value>) -> Vec<Value> {
    ops.into_iter()
        .map(|mut op| {
            if op["pinned"] == Value::Bool(true) {
                op["pinned"] = Value::Bool(false);
            }
            op
        })
        .collect()
}
