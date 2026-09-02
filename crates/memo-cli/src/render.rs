//! Turning a memory into something a person reads.
//!
//! Terminal output, so no colour library and no table crate: escape codes where a terminal is
//! attached, plain text where one is not. Anything richer belongs in the harness that is
//! showing it, not in a memory layer's diagnostics.

use memo_model::{Memory, Timestamp, Witness};
use std::io::Write;

/// Write one line to standard output, and stop quietly when nobody is reading.
///
/// `println!` panics on a broken pipe, so `memo recall | head` ended in a backtrace rather than
/// in output. A reader that has gone away is not an error — it is the ordinary end of a pipe —
/// so this exits successfully instead.
pub fn write_line(args: std::fmt::Arguments<'_>) {
    let mut out = std::io::stdout().lock();
    if let Err(why) = writeln!(out, "{args}")
        && why.kind() == std::io::ErrorKind::BrokenPipe
    {
        std::process::exit(0);
    }
}

/// `println!`, but a closed pipe ends the program rather than panicking in it.
#[macro_export]
macro_rules! say {
    () => { $crate::render::write_line(format_args!("")) };
    ($($arg:tt)*) => { $crate::render::write_line(format_args!($($arg)*)) };
}

/// Whether to spend escape codes.
///
/// `$NO_COLOR` first, because it is the convention and because a person who set it means it.
/// Otherwise: only when stdout is a terminal, so `memo recall | grep` sees plain text.
#[must_use]
pub fn styled() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    std::io::IsTerminal::is_terminal(&std::io::stdout())
}

/// Dim, when dimming is worth it.
#[must_use]
pub fn dim(text: &str) -> String {
    if styled() {
        format!("\x1b[2m{text}\x1b[0m")
    } else {
        text.to_owned()
    }
}

/// Emphasised.
#[must_use]
pub fn bold(text: &str) -> String {
    if styled() {
        format!("\x1b[1m{text}\x1b[0m")
    } else {
        text.to_owned()
    }
}

/// How long ago, in the largest unit that still says something.
///
/// "3 months ago" beats "1710000000" and beats "91 days ago". Nobody reading a memory store
/// wants to do arithmetic to find out whether something is stale.
#[must_use]
pub fn ago(then: Timestamp, now: Timestamp) -> String {
    let seconds = (now - then).max(0);
    let (count, unit) = match seconds {
        ..60 => return "just now".to_owned(),
        60..3600 => (seconds / 60, "minute"),
        3600..86_400 => (seconds / 3600, "hour"),
        86_400..2_592_000 => (seconds / 86_400, "day"),
        2_592_000..31_536_000 => (seconds / 2_592_000, "month"),
        _ => (seconds / 31_536_000, "year"),
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural} ago")
}

/// A `0..1` as a short bar, so a column of them can be compared at a glance.
#[must_use]
pub fn bar(value: f64) -> String {
    const WIDTH: usize = 10;
    let filled = (value.clamp(0.0, 1.0) * WIDTH as f64).round() as usize;
    format!("{}{}", "█".repeat(filled), "·".repeat(WIDTH - filled))
}

/// What a memory's confidence means, in words.
///
/// The two floors are the design, so the words name them rather than describing a number. A
/// person should be able to tell "the model is being told this" from "you can find it if you
/// look" without knowing what 0.35 is.
#[must_use]
pub fn standing(memory: &Memory, inject_floor: f64, now: Timestamp) -> &'static str {
    if memory.archived_at.is_some() {
        "archived"
    } else if !memory.temporal.is_live() {
        "superseded"
    } else if memory.strength.pinned {
        "pinned"
    } else if memory.confidence >= inject_floor
        && memory.strength.at(now) >= memo_model::floor::SPENT
    {
        "asserted"
    } else {
        "findable"
    }
}

/// Why a memory is not being asserted, when it is not.
///
/// `--explain` exists so a ranking can be argued with, and "why is the model not being told
/// this" is the question people actually have. A standing of "findable" answers *that* it is
/// not asserted; this answers *why*, which is the difference between a diagnostic and a label.
#[must_use]
pub fn withheld(memory: &Memory, inject_floor: f64, now: Timestamp) -> Option<String> {
    if memory.archived_at.is_some() {
        return Some("archived — found only with --archived".to_owned());
    }
    if !memory.temporal.is_live() {
        return Some("superseded — true once, and still the answer for its own time".to_owned());
    }
    if memory.strength.at(now) < memo_model::floor::SPENT {
        return Some(format!(
            "faded to {:.2} — nobody has needed it",
            memory.strength.at(now)
        ));
    }
    if memory.confidence < inject_floor {
        // The witness counts are only quoted when the caller actually loaded them. A recall
        // that lists memories without their evidence would otherwise report every one of them
        // as having none, which is a worse lie than saying less.
        let evidence = if memory.witnesses.is_empty() {
            String::new()
        } else {
            format!(
                " — {} witness(es) across {} session(s)",
                memory.witnesses.len(),
                memory.distinct_sessions(),
            )
        };
        return Some(format!(
            "confidence {:.2} is under the {inject_floor:.2} needed to assert it{evidence}",
            memory.confidence
        ));
    }
    None
}

/// Where a memory came from, in the two scopes that matter.
///
/// A project has many sessions and they share its durable memory, so "which project" and
/// "which session" are different questions. A line that answers neither leaves a person unable
/// to tell a fact about *this* repository from one they typed somewhere else entirely.
#[must_use]
pub fn origin(memory: &Memory, project: Option<&str>, session: Option<&str>) -> String {
    let where_ = match project {
        Some(name) if memory.scope.is_global() => format!("global (not {name})"),
        _ if memory.scope.is_global() => "global".to_owned(),
        _ => project.map_or_else(
            || memory.scope.to_string(),
            |name| format!("project {name}"),
        ),
    };
    // The session's own name, never its id. A twenty-six character identity is not something
    // a person can carry from one line of output to the next, and "which session" is one of
    // the two questions this line exists to answer.
    match session.or(memory.session.as_ref().map(memo_model::SessionId::as_str)) {
        Some(named) => format!("{where_} · learned in {named}"),
        None => where_,
    }
}

/// One memory, on one line, with its standing and where it came from.
#[must_use]
pub fn line(memory: &Memory, inject_floor: f64, now: Timestamp) -> String {
    format!(
        "{}  {}\n     {}",
        dim(&short(&memory.id.to_string())),
        memory.text(),
        dim(&format!(
            "{} · {} · {} · {}",
            standing(memory, inject_floor, now),
            memory.tier,
            ago(memory.temporal.when(), now),
            confidence(memory.confidence),
        ))
    )
}

/// A confidence, to two places, which is as much as it means.
#[must_use]
pub fn confidence(value: f64) -> String {
    format!("confidence {value:.2}")
}

/// One line of a quoted turn, cut to fit.
///
/// A transcript turn can be a wall of tool output, and `memo why` is showing what a witness saw
/// rather than reprinting the session.
#[must_use]
pub fn clip(text: &str, width: usize) -> String {
    let one_line = text.split('\n').next().unwrap_or_default().trim();
    if one_line.chars().count() <= width {
        return one_line.to_owned();
    }
    let cut: String = one_line.chars().take(width.saturating_sub(1)).collect();
    let end = cut.rfind(char::is_whitespace).unwrap_or(cut.len());
    format!("{}…", cut[..end].trim_end())
}

/// The handle a person types: the last eight characters of an id.
///
/// The *last*, not the first. A ULID's leading ten characters are its millisecond timestamp,
/// so two memories written in the same moment share them — which had `memo recall` printing
/// two different facts under one handle and `memo why` refusing both as ambiguous. The trailing
/// characters are entropy and are what actually tells them apart.
///
/// Ordering is not lost by this: the full id still sorts by time, and this is only how it is
/// spelled to a person.
#[must_use]
pub fn short(id: &str) -> String {
    let count = id.chars().count();
    id.chars().skip(count.saturating_sub(8)).collect()
}

/// One witness, as `memo why` prints it.
///
/// `session` is the run's own name when it can be resolved. An id here would make the one
/// question a witness list exists to answer — which run saw this — unanswerable without a
/// second command.
#[must_use]
pub fn evidence(witness: &Witness, now: Timestamp, session: Option<&str>) -> String {
    let where_ = witness
        .cursor
        .map_or_else(String::new, |c| format!(" at {c}"));
    let note = witness
        .note
        .as_deref()
        .map_or_else(String::new, |n| format!(" ({n})"));
    format!(
        "  {:<13} {:<5} {} in {}{}{}",
        witness.kind.to_string(),
        format!("{:.2}", witness.value(now)),
        dim(&ago(witness.at, now)),
        session.unwrap_or(witness.session.as_str()),
        where_,
        dim(&note),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = 1_756_000_000;

    #[test]
    fn a_memory_says_which_project_it_belongs_to() {
        use memo_model::{Body, MemoryId, ScopeId, Tier};
        let mut m = Memory::new(
            MemoryId::new("m"),
            Tier::Fact,
            ScopeId::new("/w/thing"),
            Body::fact("a", "b", "c"),
            0,
        );
        assert_eq!(origin(&m, Some("thing"), None), "project thing");

        m.session = Some(memo_model::SessionId::new("01M1CTNN7SZ613D58ZXM4JYT8Z"));
        assert!(
            origin(&m, Some("thing"), Some("0831-yt8z")).contains("learned in 0831-yt8z"),
            "a name, not an id"
        );
    }

    #[test]
    fn a_global_memory_says_it_is_not_this_projects() {
        // The confusion worth spending a word on: a fact typed somewhere else entirely, shown
        // beside this project's own, with nothing to tell them apart.
        use memo_model::{Body, MemoryId, ScopeId, Tier};
        let m = Memory::new(
            MemoryId::new("m"),
            Tier::Fact,
            ScopeId::global(),
            Body::fact("a", "b", "c"),
            0,
        );
        assert_eq!(origin(&m, Some("thing"), None), "global (not thing)");
    }

    #[test]
    fn recent_things_read_as_recent() {
        assert_eq!(ago(NOW, NOW), "just now");
        assert_eq!(ago(NOW - 90, NOW), "1 minute ago");
        assert_eq!(ago(NOW - 7200, NOW), "2 hours ago");
    }

    #[test]
    fn old_things_read_in_the_largest_useful_unit() {
        // "3 months ago" beats "91 days ago" beats a unix timestamp.
        assert_eq!(ago(NOW - 91 * 86_400, NOW), "3 months ago");
        assert_eq!(ago(NOW - 800 * 86_400, NOW), "2 years ago");
    }

    #[test]
    fn a_clock_that_ran_backwards_does_not_print_a_negative_age() {
        assert_eq!(ago(NOW + 500, NOW), "just now");
    }

    #[test]
    fn a_bar_is_always_the_same_width() {
        for value in [0.0, 0.35, 1.0, 2.0, -1.0] {
            assert_eq!(bar(value).chars().count(), 10, "at {value}");
        }
    }

    #[test]
    fn a_quoted_turn_is_one_line_and_fits() {
        // A transcript turn can be a wall of tool output, and `why` is showing what a witness
        // saw rather than reprinting the session.
        assert_eq!(clip("short", 40), "short");
        assert_eq!(clip("first line\nsecond line", 40), "first line");
        let long = clip(&"word ".repeat(60), 40);
        assert!(long.chars().count() <= 40, "{long}");
        assert!(long.ends_with('…'));
    }

    #[test]
    fn a_handle_is_the_part_that_tells_two_memories_apart() {
        // Two memories written in the same millisecond share every leading character. The
        // handle has to come from the end, or it names both of them.
        let one = "01M1CTG4FG0000P18SY0000000";
        let two = "01M1CTG4FG0000PDKA68000000";
        assert_ne!(short(one), short(two));
        assert_eq!(short(one).chars().count(), 8);
    }

    #[test]
    fn a_short_id_is_handled_rather_than_truncated_wrongly() {
        assert_eq!(short("abc"), "abc");
    }

    #[test]
    fn no_color_is_honoured_over_everything() {
        // Not a test of the environment: a check that the variable is consulted at all, since
        // the terminal branch cannot be exercised from a test harness.
        // SAFETY-free: `set_var` is safe in this edition's std for single-threaded setup.
        assert!(!styled() || std::env::var_os("NO_COLOR").is_none());
    }
}
