//! Reading balthasar's own transcript.
//!
//! [`ingest`](crate::ingest) walks a harness's journal files through a Lua adapter, because those
//! are somebody else's format; this walks the scrollback balthasar already keeps. Both end at the
//! same extractors and the same gate.

use crate::{DistilError, Ingest, Kind, Observation, Provenance, Report, Role, extract};
use balthasar_lua::{Engine, Settings};
use balthasar_model::SessionId;
use balthasar_store::{RunRead, Store, Transcript, Turn};

/// What names this reader in a stamp and in a witness note.
pub const SOURCE: &str = "transcript";

/// Run the extractors over one run's turns.
///
/// Idempotent by the same stamp machinery an ingest uses, which also keeps how far the run was
/// read: a run that went on is read from there, never twice over. Bumping
/// [`EXTRACTOR_VERSION`](crate::EXTRACTOR_VERSION) makes a better rule read every run again.
///
/// `ask.source` should be [`SOURCE`].
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

    let read = if ask.dry_run {
        RunRead::Unread
    } else {
        store.read_of(SOURCE, session.as_str(), crate::EXTRACTOR_VERSION)?
    };
    let turns: Vec<Turn> = held
        .replay(session)?
        .into_iter()
        .filter(|turn| read.pending(turn.cursor))
        .collect();
    if turns.is_empty() {
        if read != RunRead::Unread {
            report.already_read += 1;
        }
        return Ok(report);
    }
    report.sessions = 1;
    report.observations = turns.len();

    // The whole run, in order: a repair is a failure followed by a success and a file that
    // matters is one read three times, neither of them visible one turn at a time.
    let seen: Vec<Observation> = turns.iter().map(observation).collect();
    let mut found = extract(&seen, &settings.imperatives);

    // Then what disagrees with what this project already believes. It needs the store, so it
    // cannot live in `extract` with the rules that read a turn on its own.
    found
        .candidates
        .extend(crate::clashes(store, &ask.scope, &seen));
    found.candidates.extend(crate::decisions(
        store,
        &ask.scope,
        &seen,
        ask.now,
        &settings.decisions,
    ));

    // Then, if a model is configured and reachable, what the rules could not read. It goes into
    // the same list, through the same floors and the same gate, weaker only by its weight.
    let (backends, _) = crate::backends(engine.config().get("distiller"));
    let (guessed, by) = crate::propose(&backends, &seen, crate::Budget::default());
    report.inferred = guessed.len();
    report.by = by;
    found.candidates.extend(guessed);

    // A rule the person took back by undoing its note is not learned again from where they said it.
    let taken = held.taken_back()?;
    found.candidates.retain(|candidate| {
        candidate.cursor.is_none_or(|cursor| {
            !taken
                .iter()
                .any(|back| back.covers(session, cursor, &candidate.text()))
        })
    });

    report.proposed = found.candidates.len();

    let from = Provenance {
        scope: ask.scope.clone(),
        session: session.clone(),
        through: balthasar_model::Through::Ingest,
        who: ask.source.clone(),
        // When the run happened; the first turn is the closest thing to a start time here.
        happened: turns.first().map_or(ask.now, |t| t.at.max(0)),
        now: ask.now,
        dry_run: ask.dry_run,
    };
    crate::ingest::land(store, engine, settings, &from, &found, &mut report)?;

    if !ask.dry_run {
        let through = turns.last().map_or(0, |turn| turn.cursor);
        store.stamp_through(
            SOURCE,
            session.as_str(),
            crate::EXTRACTOR_VERSION,
            through,
            ask.now,
        )?;
    }
    Ok(report)
}

/// Every run the scrollback holds that has not been read by this extractor, newest first and
/// capped.
pub fn undistilled(
    store: &Store,
    held: &Transcript,
    cap: usize,
) -> Result<Vec<SessionId>, DistilError> {
    let mut out = Vec::new();
    for run in held.runs(cap)? {
        let read = store.read_of(SOURCE, run.session.as_str(), crate::EXTRACTOR_VERSION)?;
        let next = held.next_cursor(&run.session)?;
        if read == RunRead::Unread || (next > 0 && read.pending(next - 1)) {
            out.push(run.session);
        }
    }
    Ok(out)
}

/// One stored turn, in the shape the extractors read.
///
/// A widening, not a translation: anything the harness left out stays `None`.
fn observation(turn: &Turn) -> Observation {
    Observation {
        cursor: Some(turn.cursor),
        role: match turn.role.as_str() {
            "user" => Role::User,
            "assistant" => Role::Assistant,
            "tool" => Role::Tool,
            _ => Role::Other,
        },
        kind: match turn.kind.as_str() {
            "thinking" => Kind::Thinking,
            "tool_call" => Kind::ToolCall,
            "tool_result" => Kind::ToolResult,
            "summary" => Kind::Summary,
            "from" => Kind::From,
            "branch" => Kind::Branch,
            "user" => Kind::User,
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
        let seen = observation(&turn(1, "user", "carry on"));
        assert_eq!(seen.ok, None);
        assert_eq!(seen.ms, None);
        assert_eq!(seen.args, None);
        assert!(!seen.failed() && !seen.worked());
    }

    #[test]
    fn unparseable_args_are_dropped_rather_than_failing_the_run() {
        // `args` is the harness's own JSON, held as text and never validated on the way in.
        let mut held = turn(2, "tool", "");
        held.args = Some("{not json".to_owned());
        assert_eq!(observation(&held).args, None);
    }
}
