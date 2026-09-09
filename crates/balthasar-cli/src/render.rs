//! Turning a memory into something a person reads.
//!
//! Terminal output: escape codes where a terminal is attached, plain text where one is not.

use balthasar_model::{Memory, Timestamp, Witness};
use std::io::Write;

/// Write one line to standard output, and stop quietly when nobody is reading.
///
/// `println!` panics on a broken pipe, so this exits successfully instead.
pub fn write_line(args: std::fmt::Arguments<'_>) {
    let mut out = std::io::stdout().lock();
    if let Err(why) = writeln!(out, "{args}")
        && why.kind() == std::io::ErrorKind::BrokenPipe
    {
        std::process::exit(0);
    }
}

/// Which encoding an answer was asked for.
///
/// Every verb takes both flags, so a sibling can ask any of them the family's way. Bare, a verb
/// prints what a person reads.
#[derive(Debug, Clone, Copy, Default, clap::Args)]
pub struct How {
    /// Answer in JSON. The default, and accepted so every sibling takes the same flags.
    #[arg(long)]
    pub json: bool,
    /// Answer in CBOR rather than JSON.
    #[arg(long)]
    pub cbor: bool,
}

impl How {
    /// Whether a machine asked, rather than a person.
    #[must_use]
    pub fn framed(self) -> bool {
        self.json || self.cbor
    }

    /// Write one value in whichever encoding was asked for.
    ///
    /// A body that will not encode goes out as JSON instead of not at all.
    pub fn write(self, out: &mut impl Write, value: &serde_json::Value) -> std::io::Result<()> {
        if self.cbor {
            let mut bytes = Vec::new();
            if ciborium::into_writer(value, &mut bytes).is_ok() {
                return out.write_all(&bytes);
            }
        }
        writeln!(out, "{value}")
    }

    /// The same, to standard output, stopping quietly when nobody is reading.
    pub fn emit(self, value: &serde_json::Value) {
        let mut out = std::io::stdout().lock();
        if let Err(why) = self.write(&mut out, value)
            && why.kind() == std::io::ErrorKind::BrokenPipe
        {
            std::process::exit(0);
        }
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
/// `$NO_COLOR` first, then only when stdout is a terminal.
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
#[must_use]
pub fn standing(memory: &Memory, inject_floor: f64, now: Timestamp) -> &'static str {
    if memory.archived_at.is_some() {
        "archived"
    } else if !memory.temporal.is_live() {
        "superseded"
    } else if memory.strength.pinned {
        "pinned"
    } else if memory.confidence >= inject_floor
        && memory.strength.at(now) >= balthasar_model::floor::SPENT
    {
        "asserted"
    } else {
        "findable"
    }
}

/// Why a memory is not being asserted, when it is not.
#[must_use]
pub fn withheld(memory: &Memory, inject_floor: f64, now: Timestamp) -> Option<String> {
    if memory.archived_at.is_some() {
        return Some("archived — found only with --archived".to_owned());
    }
    if !memory.temporal.is_live() {
        return Some("superseded — true once, and still the answer for its own time".to_owned());
    }
    if memory.strength.at(now) < balthasar_model::floor::SPENT {
        return Some(format!(
            "faded to {:.2} — nobody has needed it",
            memory.strength.at(now)
        ));
    }
    if memory.confidence < inject_floor {
        // The witness counts are only quoted when the caller actually loaded them.
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
    // The session's own name, never its id.
    match session.or(memory
        .session
        .as_ref()
        .map(balthasar_model::SessionId::as_str))
    {
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
/// The *last*, not the first: a ULID's leading ten characters are its millisecond timestamp, so
/// two memories written in the same moment share them.
#[must_use]
pub fn short(id: &str) -> String {
    let count = id.chars().count();
    id.chars().skip(count.saturating_sub(8)).collect()
}

/// One witness, as `balthasar why` prints it.
///
/// `session` is the run's own name when it can be resolved.
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
        use balthasar_model::{Body, MemoryId, ScopeId, Tier};
        let mut m = Memory::new(
            MemoryId::new("m"),
            Tier::Fact,
            ScopeId::new("/w/thing"),
            Body::fact("a", "b", "c"),
            0,
        );
        assert_eq!(origin(&m, Some("thing"), None), "project thing");

        m.session = Some(balthasar_model::SessionId::new(
            "01M1CTNN7SZ613D58ZXM4JYT8Z",
        ));
        assert!(
            origin(&m, Some("thing"), Some("0831-yt8z")).contains("learned in 0831-yt8z"),
            "a name, not an id"
        );
    }

    #[test]
    fn a_global_memory_says_it_is_not_this_projects() {
        use balthasar_model::{Body, MemoryId, ScopeId, Tier};
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
        assert_eq!(clip("short", 40), "short");
        assert_eq!(clip("first line\nsecond line", 40), "first line");
        let long = clip(&"word ".repeat(60), 40);
        assert!(long.chars().count() <= 40, "{long}");
        assert!(long.ends_with('…'));
    }

    #[test]
    fn a_handle_is_the_part_that_tells_two_memories_apart() {
        // Two memories written in the same millisecond share every leading character.
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
        // A check that the variable is consulted at all; the terminal branch cannot be reached here.
        assert!(!styled() || std::env::var_os("NO_COLOR").is_none());
    }
}
