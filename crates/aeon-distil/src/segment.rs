//! Where one piece of work ends and the next begins.
//!
//! An episode is a coherent change in what is being worked on, not N turns or N tokens. A fixed
//! window cuts through the middle of a repair as readily as between two unrelated tasks, and an
//! episode that begins mid-repair cannot answer *what fixed it* because half the evidence is in
//! the neighbour.
//!
//! Everything here is rules. No model, no embeddings, no clock beyond what the turns carry — so
//! segmentation produces the same answer on every machine, and a distiller that proposes better
//! boundaries is an improvement on something that already works rather than a dependency.
//!
//! Three properties are load-bearing, and each has a test.
//!
//! **Every boundary says why it is there.** A segmenter whose reasons are implicit is one nobody
//! can debug, and a causal claim built on an unexplained boundary is unfalsifiable.
//!
//! **Appending a turn changes only the open segment.** A segmenter that rewrites the whole
//! session on every observation is unusable on a live transcript and makes every derived record
//! churn. Closed segments are closed.
//!
//! **The transcript is untouched.** Segments are derived and rebuildable; the turns they point
//! at are the authority.

use crate::observation::{Kind, Observation, Role};
use aeon_model::Timestamp;

/// Which version of these rules produced a segment.
///
/// Stored with every derived record so that a change here can be rolled out beside the old
/// segmentation and compared, rather than silently replacing it.
pub const DERIVATION: u32 = 1;

/// What this module calls itself in a derived record.
pub const METHOD: &str = "rules";

/// Why a boundary is where it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// The run began.
    SessionStart,
    /// The run ended.
    SessionEnd,
    /// The person asked for something different.
    GoalChange,
    /// Work moved to another directory or repository.
    DirectoryChange,
    /// A tool failed and a different one then worked — a repair, which is one episode.
    Repair,
    /// The person corrected the agent.
    Correction,
    /// Nothing happened for a long time.
    Idle,
    /// A summary stood in for turns that left the window.
    Compaction,
    /// The caller said so.
    Marker,
    /// The span grew past what one episode should hold.
    TooLong,
}

impl Signal {
    /// Whether this splits on its own, or only in company.
    ///
    /// Hard signals are the ones where continuing would join two genuinely different pieces of
    /// work. Weak ones are suggestive: a single idle gap in the middle of a repair is somebody
    /// getting coffee, and cutting there would separate a failure from its fix.
    #[must_use]
    pub fn is_hard(self) -> bool {
        matches!(
            self,
            Self::SessionStart
                | Self::SessionEnd
                | Self::GoalChange
                | Self::DirectoryChange
                | Self::Compaction
                | Self::Marker
                | Self::TooLong
        )
    }

    /// What it is called.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SessionStart => "session-start",
            Self::SessionEnd => "session-end",
            Self::GoalChange => "goal-change",
            Self::DirectoryChange => "directory-change",
            Self::Repair => "repair",
            Self::Correction => "correction",
            Self::Idle => "idle",
            Self::Compaction => "compaction",
            Self::Marker => "marker",
            Self::TooLong => "too-long",
        }
    }
}

/// One place the work changed, and the case for it.
#[derive(Debug, Clone, PartialEq)]
pub struct Boundary {
    /// The first cursor of what comes after.
    pub cursor: u64,
    /// Why.
    pub signal: Signal,
    /// The sentence a person reads.
    pub why: String,
}

/// One coherent piece of work.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// First turn.
    pub start_cursor: u64,
    /// Last turn.
    pub end_cursor: u64,
    /// When it began.
    pub started_at: Timestamp,
    /// When it ended.
    pub ended_at: Timestamp,
    /// What opened it.
    pub before: Boundary,
    /// What closed it, when something has.
    pub after: Option<Boundary>,
    /// Which rules produced it.
    pub method: &'static str,
    /// Which version of them.
    pub derivation: u32,
}

impl Segment {
    /// Whether this segment is still being written to.
    ///
    /// An open segment is the only one an appended turn may change. Everything else is settled.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.after.is_none()
    }

    /// How many turns it spans.
    #[must_use]
    pub fn turns(&self) -> u64 {
        self.end_cursor.saturating_sub(self.start_cursor) + 1
    }
}

/// What counts as a boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rules {
    /// Turns below which weak signals are ignored, so a repair is never cut in half.
    pub min_turns: u64,
    /// Turns above which the span is split whatever else is happening.
    pub max_turns: u64,
    /// Seconds of silence that suggest the work moved on.
    pub idle_seconds: i64,
}

impl Default for Rules {
    /// The shipped numbers.
    ///
    /// `min_turns` is three because the shortest complete piece of work is ask, attempt,
    /// result. `max_turns` is forty because an episode nobody can read is not a summary of
    /// anything. `idle_seconds` is twenty minutes: long enough that lunch is not a boundary,
    /// short enough that tomorrow is.
    fn default() -> Self {
        Self {
            min_turns: 3,
            max_turns: 40,
            idle_seconds: 20 * 60,
        }
    }
}

/// Cut a run into episodes.
///
/// The turns must be in cursor order, which is how a transcript stores them.
#[must_use]
pub fn segment(turns: &[Observation], rules: &Rules) -> Vec<Segment> {
    if turns.is_empty() {
        return Vec::new();
    }
    let found = signals(turns, rules);
    resolve(turns, &found, rules)
}

/// Every candidate boundary, before anything decides which survive.
fn signals(turns: &[Observation], rules: &Rules) -> Vec<Boundary> {
    let mut out = vec![Boundary {
        cursor: cursor_of(turns, 0),
        signal: Signal::SessionStart,
        why: "the run began".to_owned(),
    }];

    for (at, turn) in turns.iter().enumerate().skip(1) {
        let previous = &turns[at - 1];
        let cursor = cursor_of(turns, at);

        if let Some(marker) = phase_marker(turn) {
            out.push(Boundary {
                cursor,
                signal: Signal::Marker,
                why: format!("the caller marked a phase: {marker}"),
            });
            continue;
        }

        if turn.kind == Kind::Summary {
            out.push(Boundary {
                cursor,
                signal: Signal::Compaction,
                why: "a summary stood in for turns that left the window".to_owned(),
            });
            continue;
        }

        if let Some(moved) = directory_change(previous, turn) {
            out.push(Boundary {
                cursor,
                signal: Signal::DirectoryChange,
                why: format!("work moved to {moved}"),
            });
            continue;
        }

        if turn.role == Role::User && !turn.text.trim().is_empty() {
            // A correction continues the work; a new request replaces it. Which one it is
            // decides whether the failure and its fix stay in one episode.
            if is_correction(&turn.text) {
                out.push(Boundary {
                    cursor,
                    signal: Signal::Correction,
                    why: "the person corrected the agent".to_owned(),
                });
            } else {
                out.push(Boundary {
                    cursor,
                    signal: Signal::GoalChange,
                    why: "the person asked for something else".to_owned(),
                });
            }
            continue;
        }

        if previous.failed() && turn.worked() && !same_tool(previous, turn) {
            out.push(Boundary {
                cursor,
                signal: Signal::Repair,
                why: "a different approach worked after a failure".to_owned(),
            });
            continue;
        }

        if let (Some(was), Some(now)) = (previous.at, turn.at)
            && now - was >= rules.idle_seconds
        {
            out.push(Boundary {
                cursor,
                signal: Signal::Idle,
                why: format!("nothing happened for {} minutes", (now - was) / 60),
            });
        }
    }

    out
}

/// Turn candidate boundaries into segments.
///
/// A weak signal inside `min_turns` of the segment's start is dropped rather than honoured: it
/// is the rule that keeps a failure and its fix in one episode, which is the whole reason
/// causal questions can be answered at all.
fn resolve(turns: &[Observation], found: &[Boundary], rules: &Rules) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    let mut open: Option<Segment> = None;

    for (at, turn) in turns.iter().enumerate() {
        let cursor = cursor_of(turns, at);
        let when = turn.at.unwrap_or_default();
        let here = found.iter().find(|b| b.cursor == cursor && at > 0);

        // Long spans are split even with nothing else to say, so one runaway session cannot
        // become one unreadable episode.
        let overlong = open
            .as_ref()
            .is_some_and(|s| cursor.saturating_sub(s.start_cursor) >= rules.max_turns);

        let cut = match (here, overlong) {
            (_, true) => Some(Boundary {
                cursor,
                signal: Signal::TooLong,
                why: format!("the span reached {} turns", rules.max_turns),
            }),
            (Some(boundary), _) => {
                let held = open.as_ref().is_some_and(|s| {
                    !boundary.signal.is_hard()
                        && cursor.saturating_sub(s.start_cursor) < rules.min_turns
                });
                if held { None } else { Some(boundary.clone()) }
            }
            (None, false) => None,
        };

        if let Some(boundary) = cut
            && let Some(mut closing) = open.take()
        {
            closing.end_cursor = cursor.saturating_sub(1);
            closing.after = Some(boundary.clone());
            out.push(closing);
            open = Some(opening(cursor, when, boundary));
            continue;
        }

        match open.as_mut() {
            None => {
                let start = found.first().cloned().unwrap_or(Boundary {
                    cursor,
                    signal: Signal::SessionStart,
                    why: "the run began".to_owned(),
                });
                open = Some(opening(cursor, when, start));
            }
            Some(held) => {
                held.end_cursor = cursor;
                held.ended_at = when;
            }
        }
    }

    if let Some(last) = open {
        out.push(last);
    }
    out
}

/// A fresh segment starting here.
fn opening(cursor: u64, when: Timestamp, before: Boundary) -> Segment {
    Segment {
        start_cursor: cursor,
        end_cursor: cursor,
        started_at: when,
        ended_at: when,
        before,
        after: None,
        method: METHOD,
        derivation: DERIVATION,
    }
}

/// The cursor a turn carries, or its position when the harness sent none.
fn cursor_of(turns: &[Observation], at: usize) -> u64 {
    turns[at].cursor.unwrap_or(at as u64)
}

/// A phase the caller named for itself.
fn phase_marker(turn: &Observation) -> Option<String> {
    turn.args
        .as_ref()
        .and_then(|a| a.get("aeon_phase"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

/// Where the work moved, when it moved.
fn directory_change(previous: &Observation, turn: &Observation) -> Option<String> {
    let was = working_dir(previous)?;
    let now = working_dir(turn)?;
    (was != now).then_some(now)
}

/// The directory a tool call names, when it names one.
fn working_dir(turn: &Observation) -> Option<String> {
    turn.args
        .as_ref()
        .and_then(|a| a.get("cwd"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

/// Whether two turns used the same tool on the same thing.
fn same_tool(one: &Observation, two: &Observation) -> bool {
    one.tool == two.tool
        && one.args.as_ref().and_then(|a| a.get("command"))
            == two.args.as_ref().and_then(|a| a.get("command"))
}

/// Whether a user turn is fixing the agent rather than asking for something new.
///
/// Deliberately a short list of openings rather than a classifier. Being wrong here costs a
/// boundary in the wrong place, and a rule anybody can read and extend is worth more than an
/// accuracy nobody can audit.
fn is_correction(text: &str) -> bool {
    const OPENINGS: &[&str] = &[
        "no,",
        "no ",
        "not ",
        "wrong",
        "actually",
        "that's not",
        "thats not",
        "it's not",
        "its not",
        "don't",
        "dont ",
        "stop",
        "undo",
        "revert",
        "instead",
        "i said",
        "i meant",
    ];
    let head: String = text.trim().to_lowercase().chars().take(40).collect();
    OPENINGS.iter().any(|opening| head.starts_with(opening))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = 1_756_000_000;

    fn user(cursor: u64, text: &str, at: Timestamp) -> Observation {
        Observation {
            cursor: Some(cursor),
            role: Role::User,
            text: text.to_owned(),
            at: Some(at),
            ..Observation::default()
        }
    }

    fn tool(cursor: u64, command: &str, ok: bool, at: Timestamp) -> Observation {
        Observation {
            cursor: Some(cursor),
            role: Role::Tool,
            kind: Kind::ToolResult,
            tool: Some("shell".to_owned()),
            args: Some(serde_json::json!({ "command": command })),
            ok: Some(ok),
            text: if ok { "ok" } else { "failed" }.to_owned(),
            at: Some(at),
            ..Observation::default()
        }
    }

    /// Ask, fail, repair — the shortest complete piece of work.
    fn a_repair(from: u64, at: Timestamp) -> Vec<Observation> {
        vec![
            user(from, "get the tests passing", at),
            tool(from + 1, "cargo test", false, at + 10),
            tool(from + 2, "make test", true, at + 20),
        ]
    }

    #[test]
    fn a_run_with_nothing_in_it_has_no_episodes() {
        assert!(segment(&[], &Rules::default()).is_empty());
    }

    #[test]
    fn one_piece_of_work_is_one_episode() {
        let held = segment(&a_repair(0, NOW), &Rules::default());
        assert_eq!(held.len(), 1, "{held:#?}");
        assert_eq!(held[0].start_cursor, 0);
        assert_eq!(held[0].end_cursor, 2);
    }

    #[test]
    fn a_failure_and_its_fix_are_never_separated() {
        // The property every causal question depends on. A boundary between the failure and the
        // repair would put "what broke" in one episode and "what fixed it" in the next, and
        // neither could answer the question on its own.
        let held = segment(&a_repair(0, NOW), &Rules::default());
        assert_eq!(held.len(), 1);
        let only = &held[0];
        assert!(only.start_cursor <= 1 && only.end_cursor >= 2, "{only:#?}");
    }

    #[test]
    fn a_new_request_starts_a_new_episode() {
        let mut turns = a_repair(0, NOW);
        turns.extend(a_repair(3, NOW + 100));
        let held = segment(&turns, &Rules::default());

        assert_eq!(held.len(), 2, "{held:#?}");
        assert_eq!(held[0].end_cursor, 2);
        assert_eq!(held[1].start_cursor, 3);
        assert_eq!(held[1].before.signal, Signal::GoalChange);
    }

    #[test]
    fn a_correction_is_not_a_new_goal() {
        // "no, use make" continues the work it corrects. Treating it as a new request would
        // start an episode whose first turn is the fix to a failure it does not contain.
        let mut turns = a_repair(0, NOW);
        turns.push(user(3, "no, use make instead", NOW + 30));
        turns.push(tool(4, "make test", true, NOW + 40));
        let held = segment(&turns, &Rules::default());

        assert_eq!(held.len(), 2, "{held:#?}");
        assert_eq!(held[1].before.signal, Signal::Correction);
    }

    #[test]
    fn every_boundary_says_why_it_is_there() {
        // A boundary without a reason cannot be argued with, and anything derived from it
        // inherits that.
        let mut turns = a_repair(0, NOW);
        turns.extend(a_repair(3, NOW + 100));
        for held in segment(&turns, &Rules::default()) {
            assert!(!held.before.why.is_empty(), "{held:#?}");
            assert!(!held.before.signal.as_str().is_empty());
            if let Some(after) = &held.after {
                assert!(!after.why.is_empty(), "{held:#?}");
            }
        }
    }

    #[test]
    fn appending_an_ordinary_turn_changes_only_the_open_segment() {
        // The stability property. A segmenter that rewrites the session on every observation
        // makes every derived record churn and cannot run on a live transcript.
        let mut turns = a_repair(0, NOW);
        turns.extend(a_repair(3, NOW + 100));
        let before = segment(&turns, &Rules::default());

        turns.push(tool(6, "make build", true, NOW + 130));
        let after = segment(&turns, &Rules::default());

        let closed_before: Vec<_> = before.iter().filter(|s| !s.is_open()).collect();
        let closed_after: Vec<_> = after.iter().filter(|s| !s.is_open()).collect();
        assert_eq!(
            closed_before.len(),
            closed_after.len(),
            "a closed segment moved"
        );
        for (was, now) in closed_before.iter().zip(closed_after.iter()) {
            assert_eq!(was, now, "a settled segment was rewritten");
        }
    }

    #[test]
    fn a_long_silence_ends_an_episode() {
        let mut turns = a_repair(0, NOW);
        turns.push(tool(3, "make build", true, NOW + 4 * 3600));
        turns.push(tool(4, "make lint", true, NOW + 4 * 3600 + 10));
        let held = segment(&turns, &Rules::default());

        assert_eq!(held.len(), 2, "{held:#?}");
        assert_eq!(held[1].before.signal, Signal::Idle);
        assert!(held[1].before.why.contains("minutes"));
    }

    #[test]
    fn a_silence_inside_a_short_span_is_somebody_getting_coffee() {
        // A weak signal too close to the start is dropped. Otherwise a pause between asking and
        // the first attempt would produce a one-turn episode containing only the question.
        let turns = vec![
            user(0, "get the tests passing", NOW),
            tool(1, "cargo test", false, NOW + 3600),
            tool(2, "make test", true, NOW + 3610),
        ];
        let held = segment(&turns, &Rules::default());
        assert_eq!(
            held.len(),
            1,
            "the pause did not split the repair: {held:#?}"
        );
    }

    #[test]
    fn a_runaway_session_is_still_cut_into_readable_pieces() {
        let rules = Rules {
            max_turns: 10,
            ..Rules::default()
        };
        let turns: Vec<Observation> = (0..25)
            .map(|n| tool(n, "make build", true, NOW + n as i64))
            .collect();
        let held = segment(&turns, &rules);

        assert!(held.len() >= 2, "{} segments", held.len());
        assert!(held.iter().all(|s| s.turns() <= rules.max_turns + 1));
        assert!(
            held.iter()
                .skip(1)
                .any(|s| s.before.signal == Signal::TooLong)
        );
    }

    #[test]
    fn moving_to_another_directory_starts_new_work() {
        let turns = vec![
            user(0, "build it", NOW),
            Observation {
                cursor: Some(1),
                role: Role::Tool,
                tool: Some("shell".to_owned()),
                args: Some(serde_json::json!({ "command": "make", "cwd": "/w/one" })),
                ok: Some(true),
                at: Some(NOW + 10),
                ..Observation::default()
            },
            Observation {
                cursor: Some(2),
                role: Role::Tool,
                tool: Some("shell".to_owned()),
                args: Some(serde_json::json!({ "command": "make", "cwd": "/w/two" })),
                ok: Some(true),
                at: Some(NOW + 20),
                ..Observation::default()
            },
        ];
        let held = segment(&turns, &Rules::default());
        assert_eq!(held.len(), 2, "{held:#?}");
        assert_eq!(held[1].before.signal, Signal::DirectoryChange);
        assert!(held[1].before.why.contains("/w/two"));
    }

    #[test]
    fn a_caller_may_name_its_own_boundary() {
        let mut turns = a_repair(0, NOW);
        turns.push(Observation {
            cursor: Some(3),
            role: Role::Tool,
            tool: Some("shell".to_owned()),
            args: Some(serde_json::json!({ "aeon_phase": "validation" })),
            ok: Some(true),
            at: Some(NOW + 30),
            ..Observation::default()
        });
        let held = segment(&turns, &Rules::default());
        assert_eq!(held.len(), 2, "{held:#?}");
        assert_eq!(held[1].before.signal, Signal::Marker);
        assert!(held[1].before.why.contains("validation"));
    }

    #[test]
    fn segmentation_is_the_same_answer_every_time() {
        // No clock, no model, no map iteration order. A boundary that moved between two runs of
        // the same code would make every comparison meaningless.
        let mut turns = a_repair(0, NOW);
        turns.extend(a_repair(3, NOW + 100));
        assert_eq!(
            segment(&turns, &Rules::default()),
            segment(&turns, &Rules::default())
        );
    }

    #[test]
    fn every_turn_belongs_to_exactly_one_episode() {
        // Otherwise a memory distilled from a span could be attributed to two episodes, or to
        // none, and the cursor range on an Episode would stop meaning anything.
        let mut turns = a_repair(0, NOW);
        turns.extend(a_repair(3, NOW + 100));
        turns.push(tool(6, "make lint", true, NOW + 8 * 3600));
        let held = segment(&turns, &Rules::default());

        let mut covered: Vec<u64> = Vec::new();
        for one in &held {
            for cursor in one.start_cursor..=one.end_cursor {
                covered.push(cursor);
            }
        }
        covered.sort_unstable();
        let expected: Vec<u64> = turns.iter().filter_map(|t| t.cursor).collect();
        assert_eq!(covered, expected, "{held:#?}");
    }
}
