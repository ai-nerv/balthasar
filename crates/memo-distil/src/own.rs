//! Reading memo's own transcript.
//!
//! The other reader. [`ingest`](crate::ingest) walks a harness's journal files through a Lua
//! adapter, because those are somebody else's format; this walks the scrollback memo already
//! keeps, which is memo's own. Both end at the same extractors and the same gate, so a claim
//! learned here is worth exactly what the same claim learned from a journal is worth.
//!
//! Without this the extractive half of the ladder only ever ran on backfill. A session streamed
//! live through `observe` got TIDE when its turns left the window and CALLUS when another run
//! agreed with it, and nothing at all was watching it for "remember that we deploy with fly.io"
//! — which is the cheapest signal there is and the one a person would most expect to work.

use crate::{DistilError, Ingest, Kind, Observation, Provenance, Report, Role, extract};
use memo_lua::{Engine, Settings};
use memo_model::SessionId;
use memo_store::{Store, Transcript, Turn};

/// What names this reader in a stamp and in a witness note.
pub const SOURCE: &str = "transcript";

/// Run the extractors over one run's turns.
///
/// Idempotent by the same stamp machinery an ingest uses, so calling it after every session is
/// cheap and calling it twice does nothing. Bumping [`EXTRACTOR_VERSION`](crate::EXTRACTOR_VERSION)
/// makes a better rule read every run again without anybody having to remember to say so.
///
/// Takes the same [`Ingest`] ask a source-reading pass takes, because it is the same pass with a
/// different file underneath. `ask.source` should be [`SOURCE`].
pub fn distil_run(
    store: &mut Store,
    engine: &mut Engine,
    settings: &Settings,
    held: &Transcript,
    session: &SessionId,
    ask: &Ingest,
) -> Result<Report, DistilError> {
    let mut report = Report {
        dry_run: ask.dry_run,
        ..Report::default()
    };

    if !ask.dry_run && store.already_read(SOURCE, session.as_str(), crate::EXTRACTOR_VERSION)? {
        report.already_read += 1;
        return Ok(report);
    }

    let turns = held.replay(session)?;
    if turns.is_empty() {
        return Ok(report);
    }
    report.sessions = 1;
    report.observations = turns.len();

    // The whole run, in order. The rules need it: a repair is a failure followed by a success,
    // and a file that matters is one read three times — neither is visible one turn at a time,
    // which is why this is a pass over a finished run rather than a hook on `observe`.
    let seen: Vec<Observation> = turns.iter().map(observation).collect();
    let found = extract(&seen, &settings.imperatives);
    report.proposed = found.candidates.len();

    let from = Provenance {
        scope: ask.scope.clone(),
        session: session.clone(),
        through: memo_model::Through::Ingest,
        who: ask.source.clone(),
        // When the run happened. A month of sessions distilled this evening did not all become
        // true this evening, and the first turn is the closest thing to a start time here.
        happened: turns.first().map_or(ask.now, |t| t.at.max(0)),
        now: ask.now,
        dry_run: ask.dry_run,
    };
    crate::ingest::land(store, engine, settings, &from, &found, &mut report)?;

    if !ask.dry_run {
        store.stamp(SOURCE, session.as_str(), crate::EXTRACTOR_VERSION, ask.now)?;
    }
    Ok(report)
}

/// Every run the scrollback holds that has not been read by this extractor.
///
/// Newest first and capped, so a project with ten thousand runs makes progress on every pass
/// rather than timing out on the first one.
pub fn undistilled(
    store: &Store,
    held: &Transcript,
    cap: usize,
) -> Result<Vec<SessionId>, DistilError> {
    let mut out = Vec::new();
    for run in held.runs(cap)? {
        if !store.already_read(SOURCE, run.session.as_str(), crate::EXTRACTOR_VERSION)? {
            out.push(run.session);
        }
    }
    Ok(out)
}

/// One stored turn, in the shape the extractors read.
///
/// A widening, not a translation: the transcript keeps what a harness sent and this names the
/// parts the rules ask about. Anything the harness left out stays `None` — an extractor that
/// gets no answer proposes nothing, which is the right outcome and not an error.
fn observation(turn: &Turn) -> Observation {
    Observation {
        cursor: Some(turn.cursor),
        role: match turn.role.as_str() {
            "assistant" => Role::Assistant,
            "tool" => Role::Tool,
            _ => Role::User,
        },
        kind: match turn.kind.as_str() {
            "thinking" => Kind::Thinking,
            "tool_call" => Kind::ToolCall,
            "tool_result" => Kind::ToolResult,
            "summary" => Kind::Summary,
            _ => Kind::Prose,
        },
        text: turn.text.clone(),
        tool: turn.tool.clone(),
        args: turn
            .args
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok()),
        ok: turn.ok,
        ms: turn.ms,
        tokens: turn.tokens.map(u64::from),
        at: Some(turn.at),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(cursor: u64, role: &str, text: &str) -> Turn {
        Turn {
            cursor,
            at: 1_756_000_000,
            role: role.to_owned(),
            kind: if role == "tool" {
                "tool_result"
            } else {
                "prose"
            }
            .to_owned(),
            text: text.to_owned(),
            ..Turn::default()
        }
    }

    #[test]
    fn a_stored_turn_reads_as_the_observation_the_rules_expect() {
        let mut held = turn(4, "tool", "ok");
        held.tool = Some("shell".to_owned());
        held.ok = Some(false);
        held.ms = Some(31_000);
        held.args = Some(r#"{"command":"make test"}"#.to_owned());

        let seen = observation(&held);
        assert_eq!(seen.cursor, Some(4));
        assert_eq!(seen.role, Role::Tool);
        assert_eq!(seen.kind, Kind::ToolResult);
        assert!(seen.failed(), "a tool that failed reads as one");
        assert_eq!(seen.ms, Some(31_000));
        assert_eq!(
            seen.args
                .and_then(|a| a["command"].as_str().map(str::to_owned)),
            Some("make test".to_owned()),
            "the harness's own call comes back parsed"
        );
    }

    #[test]
    fn a_turn_the_harness_said_nothing_about_proposes_nothing() {
        // The common case for a harness that streams prose and no tool detail. It must widen
        // cleanly rather than inventing an `ok` that would read as a repair.
        let seen = observation(&turn(1, "user", "carry on"));
        assert_eq!(seen.ok, None);
        assert_eq!(seen.ms, None);
        assert_eq!(seen.args, None);
        assert!(!seen.failed() && !seen.worked());
    }

    #[test]
    fn unparseable_args_are_dropped_rather_than_failing_the_run() {
        // `args` is the harness's own JSON, held as text and never validated on the way in. A
        // reader that panicked on it would make one bad record cost the whole distillation.
        let mut held = turn(2, "tool", "");
        held.args = Some("{not json".to_owned());
        assert_eq!(observation(&held).args, None);
    }
}
