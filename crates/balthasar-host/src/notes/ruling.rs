//! Literal rule evidence and duplicate-note matching.

use serde_json::Value;

const STANDING: &[&str] = &[
    "always ",
    "never ",
    "must ",
    "prefer ",
    "avoid ",
    "remember ",
    "don't ",
    "dont ",
    "do not ",
    "from now on ",
];
const MARKERS: &[&str] = &[
    "a firm rule:",
    "firm rule:",
    "hard rule:",
    "house rule:",
    "rule:",
    "rules:",
];
const TOLD: &[&str] = &[
    "read", "write", "use", "run", "keep", "make", "add", "fix", "check", "ensure", "follow",
    "include", "name", "list", "put", "start", "end", "do", "ask", "tell", "verify", "report",
    "describe", "split", "send", "set", "call", "treat", "plan", "spawn", "wait", "create",
    "explain", "prefer", "avoid",
];

pub(super) fn canonical(text: &str) -> String {
    text.trim()
        .trim_end_matches('.')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub(super) fn same_rule(a: &str, b: &str) -> bool {
    let wording = |text: &str| {
        let text = text.trim();
        let text = text
            .strip_suffix('.')
            .unwrap_or(text)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let mut letters = text.chars();
        let first = letters
            .next()
            .map(|c| c.to_lowercase().to_string())
            .unwrap_or_default();
        first + letters.as_str()
    };
    wording(a) == wording(b)
}

pub(super) fn unquoted(input: &str) -> Vec<&str> {
    let mut fence = None;
    let mut quoted = false;
    let mut out = Vec::new();
    for raw in input.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let start = raw.trim_start_matches(' ');
        let marker = start
            .chars()
            .next()
            .filter(|c| raw.len() - start.len() <= 3 && (*c == '`' || *c == '~'));
        if let Some(marker) = marker {
            let size = start.chars().take_while(|c| *c == marker).count();
            if size >= 3 {
                match fence {
                    None => fence = Some((marker, size)),
                    Some((opened, count))
                        if opened == marker
                            && size >= count
                            && start[size..].trim_matches([' ', '\t']).is_empty() =>
                    {
                        fence = None;
                    }
                    _ => {}
                }
                continue;
            }
        }
        if fence.is_some() {
            continue;
        }
        let markup = line.split('<').skip(1).any(|part| {
            part.chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '/' | '!' | '?'))
        });
        if line.starts_with('>')
            || line.ends_with(':')
            || line.starts_with('\'')
            || line.contains([
                '"', '“', '”', '‘', '’', '`', '«', '»', '‹', '›', '„', '‟', '‚', '‛',
            ])
            || line.contains(" '")
            || markup
        {
            quoted = true;
        }
        if quoted || raw.starts_with("    ") || raw.starts_with('\t') {
            continue;
        }
        out.push(line.strip_prefix("- ").unwrap_or(line));
    }
    out
}

pub(super) fn rule_text(line: &str) -> Option<&str> {
    // A question lays nothing down, whatever word it opens with.
    if line.trim_end().ends_with('?') {
        return None;
    }
    let lower = line.to_lowercase();
    for prefix in MARKERS {
        if lower.starts_with(prefix) {
            return Some(
                line[prefix.len()..]
                    .trim()
                    .strip_prefix("we ")
                    .unwrap_or(line[prefix.len()..].trim()),
            );
        }
    }
    let body = line
        .strip_prefix("we ")
        .or_else(|| line.strip_prefix("We "))
        .unwrap_or(line);
    let lower = body.to_lowercase();
    let words = words(body);
    let standing = STANDING.iter().any(|prefix| lower.starts_with(prefix))
        || (TOLD.contains(&words.first()?.as_str())
            && words
                .iter()
                .any(|w| w == "not" || w == "never" || w == "always"))
        || line.to_lowercase().starts_with("we use ")
        || obliges(&lower, &words);
    standing.then_some(body)
}

/// Whether a line lays something down wherever in it the word comes: "In this project every
/// function name must start with zq_" is as much a rule as "Always start …", and a person does
/// not open every rule with the word that makes it one. The whole line is still what is kept, and
/// whose line it is, and that it is not a quotation, are settled before this is asked.
fn obliges(lower: &str, words: &[String]) -> bool {
    const WORDS: &[&str] = &["must", "shall", "always", "never"];
    const PHRASES: &[&str] = &["from now on", "do not ", "don't ", "standing rule"];
    words.iter().any(|w| WORDS.contains(&w.as_str()))
        || PHRASES.iter().any(|phrase| lower.contains(phrase))
}

pub(super) fn laid_down(input: &str) -> bool {
    unquoted(input)
        .into_iter()
        .any(|line| rule_text(line).is_some())
}

pub(super) fn instruction(text: &str) -> bool {
    let said = words(text);
    laid_down(text)
        || said
            .iter()
            .any(|w| ["must", "should", "shall"].contains(&w.as_str()))
        || said.first().is_some_and(|w| TOLD.contains(&w.as_str()))
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
