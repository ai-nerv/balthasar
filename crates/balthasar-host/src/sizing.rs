//! What a helper request may be, measured on the whole of it: instruction, input and schema all
//! reach the helper's window, and appending stops at the bound rather than past it.

use serde_json::Value;

/// What one helper request may hold altogether, in characters.
pub(crate) const WHOLE: usize = 200_000;

/// What is left for the input once the rest of the request is counted.
pub(crate) fn room(instruction: &str, schema: &Value, whole: usize) -> usize {
    let fixed = instruction.len()
        + if schema.is_null() {
            0
        } else {
            schema.to_string().len()
        };
    whole.saturating_sub(fixed)
}

/// Whether `spec` fits, all three parts counted.
pub(crate) fn fits(spec: &Value, whole: usize) -> bool {
    measured(spec) <= whole
}

/// What `spec` costs, in characters.
pub(crate) fn measured(spec: &Value) -> usize {
    let len = |at: &str| spec[at].as_str().map_or(0, str::len);
    let schema = &spec["schema"];
    len("instruction")
        + len("input")
        + if schema.is_null() {
            0
        } else {
            schema.to_string().len()
        }
}

/// Add `line` if it fits, and say whether it did.
pub(crate) fn add(input: &mut String, line: &str, room: usize) -> bool {
    if input.len() + line.len() > room {
        return false;
    }
    input.push_str(line);
    true
}

#[cfg(test)]
#[path = "sizing/tests.rs"]
mod tests;
