//! Verbatim user-turn verification for peer `remember` proposals.

use balthasar_model::{Quoting, SessionId};
use balthasar_store::Transcript;

/// Returns the newest cursor of an eligible user turn in `session` that contains
/// `statement` word-for-word as one or more contiguous whole sentences.
pub(crate) fn find_in_user_turns(
    scrollback: &Transcript,
    session: &SessionId,
    statement: &str,
) -> Option<u64> {
    if balthasar_model::looks_like_injection(statement) {
        return None;
    }
    let target = canon(statement);
    if target.is_empty() {
        return None;
    }
    let turns = scrollback.replay(session).ok()?;
    for turn in turns.iter().rev() {
        if turn.role != "user" || turn.is_tool() {
            continue;
        }
        if matches!(turn.kind.as_str(), "from" | "branch" | "summary") {
            continue;
        }
        if balthasar_model::pasted_conversation(&turn.text)
            || balthasar_model::asks_for_generation(&turn.text)
        {
            continue;
        }
        let lines = balthasar_model::eligible_lines(&turn.text, Quoting::Lenient);
        if lines.is_empty() {
            continue;
        }
        let spoken = lines.join("\n");
        if whole_sentences(&spoken, &target) {
            return Some(turn.cursor);
        }
    }
    None
}

fn fold(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn canon(text: &str) -> String {
    fold(text)
        .trim_end_matches(['.', '!', ';'])
        .trim_end()
        .to_owned()
}

fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut held = String::new();
    let letters: Vec<char> = text.chars().collect();
    for (at, c) in letters.iter().enumerate() {
        let ends = match c {
            '\n' | '!' | ';' => true,
            '.' => letters.get(at + 1).is_none_or(|next| next.is_whitespace()),
            _ => false,
        };
        if ends {
            let trimmed = held.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_owned());
            }
            held.clear();
        } else {
            held.push(*c);
        }
    }
    let trimmed = held.trim();
    if !trimmed.is_empty() {
        out.push(trimmed.to_owned());
    }
    out
}

fn whole_sentences(spoken: &str, target: &str) -> bool {
    if canon(spoken) == target {
        return true;
    }
    let parts = sentences(spoken);
    for start in 0..parts.len() {
        let mut joined = String::new();
        for part in parts.iter().skip(start) {
            if !joined.is_empty() {
                joined.push(' ');
            }
            joined.push_str(part);
            if canon(&joined) == target {
                return true;
            }
        }
    }
    false
}
