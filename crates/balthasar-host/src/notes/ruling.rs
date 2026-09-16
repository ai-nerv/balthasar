//! What an extraction may keep: a pinned note only where a person laid a rule down, and never a
//! note that only restates one already kept.

use serde_json::Value;

/// Words a person lays a rule down with, rather than asking for one piece of work.
const RULE_WORDS: &[&str] = &[
    "always", "never", "must", "prefer", "not", "avoid", "remember", "dont",
];

/// Phrases that lay one down. "Rule" alone is not one: a request can be about the rules.
const RULE_PHRASES: &[&str] = &[
    "from now on",
    "we use",
    "a rule",
    "firm rule",
    "hard rule",
    "house rule",
    "golden rule",
    "rule:",
    "rules:",
    "rule is",
    "rules are",
    "as a rule",
];

/// Whether the person said anything rule-shaped in what an extraction read fresh: only then may it pin.
pub(super) fn laid_down(input: &str) -> bool {
    let fresh = input
        .split_once("New since then")
        .map_or(input, |(_, after)| after);
    fresh
        .lines()
        .filter(|line| line.contains("] person: "))
        .any(|line| {
            let line = line.to_lowercase().replace('\'', "");
            RULE_PHRASES.iter().any(|phrase| line.contains(phrase))
                || line
                    .split(|c: char| !c.is_alphanumeric())
                    .any(|word| RULE_WORDS.contains(&word))
        })
}

/// The same ops with no say over pinning: a new note is unpinned, one on a kept note left as it was.
pub(super) fn unpinned(ops: Vec<Value>) -> Vec<Value> {
    ops.into_iter()
        .map(|mut op| {
            if let Some(fields) = op.as_object_mut() {
                fields.remove("pinned");
            }
            op
        })
        .collect()
}

/// Words a note uses to say how to work, rather than how things are.
const DEONTIC: &[&str] = &["must", "should", "shall"];

/// Verbs a note opens with when it is telling the assistant what to do: a fact opens with its
/// subject instead.
const TOLD: &[&str] = &[
    "read", "write", "use", "run", "keep", "make", "add", "fix", "check", "ensure", "follow",
    "include", "name", "list", "put", "start", "end", "do", "ask", "tell", "verify", "report",
    "describe", "split", "send", "set", "call", "treat",
];

/// The ops a span with no rule in it may keep: a new note saying how the work must be done, or
/// opening by telling the assistant to do it, is a rule no person stated. One that lands on a note
/// already kept is an update to it, and goes through.
pub(super) fn factual(ops: Vec<Value>, kept: &[balthasar_store::Note]) -> Vec<Value> {
    ops.into_iter()
        .filter(|op| {
            let known = kept.iter().any(|n| {
                op["id"].as_str() == Some(n.id.as_str())
                    || op["title"]
                        .as_str()
                        .is_some_and(|t| n.title.eq_ignore_ascii_case(t.trim()))
            });
            op["op"] != "add"
                || known
                || !op["text"].as_str().is_some_and(|text| {
                    let said = words(text);
                    said.iter().any(|word| DEONTIC.contains(&word.as_str()))
                        || said
                            .first()
                            .is_some_and(|word| TOLD.contains(&word.as_str()))
                })
        })
        .collect()
}

/// Whether an added note only restates one already kept: nearly every word of the shorter text
/// comes, in order, in the longer one, and it is all of it or most of the longer. Skipped rather
/// than merged, since a merge would carry its pinning over.
pub(super) fn echoes(op: &Value, kept: &[balthasar_store::Note]) -> bool {
    let Some(new) = op["text"].as_str().map(words) else {
        return false;
    };
    kept.iter().any(|note| {
        let old = words(&note.text);
        let (short, long) = (new.len().min(old.len()), new.len().max(old.len()));
        let shared = in_order(&new, &old);
        short >= 4 && shared * 10 >= short * 9 && (shared == short || shared * 10 >= long * 6)
    })
}

/// Ops with each pinned rule left in its own words, by id or by title, unless something is retired
/// into it or it comes out much shorter: new words for the same rule only make every request miss
/// the cache.
pub(super) fn kept_wording(ops: Vec<Value>, kept: &[balthasar_store::Note]) -> Vec<Value> {
    if ops.iter().any(|op| op["op"] == "retire") {
        return ops;
    }
    ops.into_iter()
        .map(|mut op| {
            let rule = kept.iter().find(|n| {
                n.pinned
                    && (op["id"].as_str() == Some(n.id.as_str())
                        || op["title"]
                            .as_str()
                            .is_some_and(|t| n.title.eq_ignore_ascii_case(t.trim())))
            });
            let shorter = rule.is_some_and(|n| {
                op["text"]
                    .as_str()
                    .is_some_and(|text| words(text).len() * 3 <= words(&n.text).len() * 2)
            });
            if let (Some(_), false, Some(fields)) = (rule, shorter, op.as_object_mut()) {
                for field in ["title", "text", "description"] {
                    fields.remove(field);
                }
            }
            op
        })
        .collect()
}

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

/// How many words the two share in the same order: the longest common subsequence.
fn in_order(a: &[String], b: &[String]) -> usize {
    let mut row = vec![0; b.len() + 1];
    for x in a {
        let mut diagonal = 0;
        for (j, y) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = if x == y {
                diagonal + 1
            } else {
                above.max(row[j])
            };
            diagonal = above;
        }
    }
    row[b.len()]
}
