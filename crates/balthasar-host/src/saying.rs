//! One memory, as a caller outside this program receives it. Here rather than beside either
//! caller: `recall` and `disagreements` answer with the same shape.

use crate::Answering;
use balthasar_model::{Memory, Timestamp};

pub(crate) fn describe(
    memory: &Memory,
    inject_floor: f64,
    now: Timestamp,
    names: &std::collections::HashMap<String, String>,
) -> serde_json::Value {
    serde_json::json!({
        "id": memory.id.to_string(),
        "text": memory.text(),
        "tier": memory.tier.as_str(),
        "project": memory.scope.to_string(),
        // Both: the identity a caller stores, and the name it shows a person.
        "session": memory.session.as_ref().map(ToString::to_string),
        "session_name": memory.session.as_ref()
            .and_then(|id| names.get(id.as_str()))
            .cloned(),
        "confidence": memory.confidence,
        // Computed here rather than left to the caller to derive from a number and a threshold.
        "asserted": memory.is_assertable(inject_floor, now, true),
        "since": memory.temporal.valid_from,
        "until": memory.temporal.valid_to,
    })
}

/// Session ids and the names they are printed under.
pub(crate) fn session_names(at: &mut Answering<'_>) -> std::collections::HashMap<String, String> {
    at.store
        .sessions(usize::MAX)
        .unwrap_or_default()
        .into_iter()
        .map(|s| (s.id.to_string(), s.name))
        .collect()
}
