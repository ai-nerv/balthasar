//! A memory as one line a model is shown: its handle, what it says, where it came from and when.

use crate::{Memory, Timestamp, Witness, WitnessKind};

/// The handle a memory is referred to by: the last eight characters of its id.
#[must_use]
pub fn handle(id: &str) -> String {
    let count = id.chars().count();
    id.chars().skip(count.saturating_sub(8)).collect()
}

/// `YYYY-MM-DD` of a moment, in UTC.
#[must_use]
pub fn day(at: Timestamp) -> String {
    let z = at.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Where the strongest of its evidence came from, in the words a model is shown.
#[must_use]
pub fn origin(witnesses: &[Witness]) -> &'static str {
    let strongest = witnesses
        .iter()
        .max_by(|a, b| a.kind.weight().total_cmp(&b.kind.weight()));
    match strongest.map(|w| w.kind) {
        Some(WitnessKind::Imperative) => "you said",
        Some(WitnessKind::Correction) => "you corrected",
        Some(WitnessKind::Choice) => "you chose",
        Some(WitnessKind::Commit) => "the code says",
        Some(WitnessKind::Cost) => "learned from a failure",
        Some(WitnessKind::Manual) => "noted",
        Some(WitnessKind::Inferred) => "inferred",
        Some(WitnessKind::Distillation) => "carried from earlier",
        Some(WitnessKind::Repetition | WitnessKind::Consolidation) => "recurred",
        None => "on record",
    }
}

/// One line: `- [handle] text (origin, date[, n runs][; doubt])`.
#[must_use]
pub fn line(memory: &Memory, text: &str, witnesses: &[Witness], doubt: Option<&str>) -> String {
    let mut runs: Vec<&str> = witnesses.iter().map(|w| w.session.as_str()).collect();
    runs.sort_unstable();
    runs.dedup();
    let mut said = format!("{}, {}", origin(witnesses), day(memory.temporal.valid_from));
    if runs.len() > 1 {
        said.push_str(&format!(", {} runs", runs.len()));
    }
    if let Some(doubt) = doubt {
        said.push_str(&format!("; {doubt}"));
    }
    format!(
        "- [{}] {} ({said})",
        handle(memory.id.as_str()),
        text.trim()
    )
}

/// Why a memory that is offered is not stated as current: stale, or weakly witnessed.
#[must_use]
pub fn doubt(memory: &Memory, floor: f64, now: Timestamp) -> String {
    let confirmed = memory.temporal.confirmed();
    if crate::is_stale(confirmed, memory.temporal.valid_to, now) {
        return format!("not confirmed since {}", day(confirmed));
    }
    if memory.confidence < floor {
        return "weakly witnessed".to_owned();
    }
    "not stated as current".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_day_is_written_in_full() {
        assert_eq!(day(0), "1970-01-01");
        assert_eq!(day(1_756_000_000), "2025-08-24");
    }

    #[test]
    fn a_handle_is_the_tail_of_the_id() {
        assert_eq!(handle("01K9ABCDEFGHJKMNPQRSTVWXYZ"), "RSTVWXYZ");
    }
}
