//! Bounded whole-row extraction, visible failures, and completion shape validation.

use super::{EXTRACT, Keeping, day, ops_schema, provenance};
use crate::Answering;

use balthasar_model::SessionId;
use balthasar_store::{Budget, JobState, StoreError, Transcript, Want};
use serde_json::{Value, json};

/// What decides that an uncovered span is worth extracting from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Progress {
    /// User rows in the span.
    pub users: usize,
    /// Rows of any kind in it.
    pub rows: usize,
    /// A cut is about to take these rows out of the live set.
    pub cut: bool,
    /// Asked for again after a failure.
    pub retry: bool,
}

/// Whether that span is due yet: enough user rows, or enough rows of any kind, or a cut or a
/// retry asking regardless.
///
/// Rows as well as users, because a long autonomous run produces tool rows and no user turns:
/// counted in users alone it never came due, and what it read was never read back.
pub(crate) fn due(at: Progress, every: u32, rows_every: u32) -> bool {
    at.cut
        || at.retry
        || at.users >= every as usize
        || (rows_every > 0 && at.rows >= rows_every as usize)
}

pub(crate) fn queue(
    at: &Answering<'_>,
    session: &SessionId,
    upto: Option<u64>,
    keeping: &Keeping,
    cut: bool,
    retry: bool,
    hooks: &mut dyn crate::Hooks,
) -> Result<(), StoreError> {
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Ok(());
    };
    scrollback.atomic(|scrollback| {
        let progress = scrollback.extraction_progress(session)?;
        if progress.blocked.is_some() && !retry { return Ok(()); }
        let from = progress.through.map_or(0, |n| n.saturating_add(1));
        let jobs = scrollback.jobs_of(session)?;
        if jobs.iter().any(|j| j.kind == "extract" && matches!(j.state, JobState::Queued | JobState::Issued)) { return Ok(()); }
        if !retry && jobs.iter().any(|j| j.kind == "extract" && j.state == JobState::Failed
            && j.context["coverage"]["from"].as_u64().is_some_and(|n| n <= from)
            && j.context["coverage"]["to"].as_u64().is_some_and(|n| n >= from)) { return Ok(()); }
        let outline = scrollback.outline(session)?;
        let Some(last) = outline.last().map(|row| row.cursor) else { return Ok(()) };
        let to = upto.unwrap_or(last).min(last);
        if from > to { return Ok(()); }
        let within = |r: &&balthasar_store::Turn| (from..=to).contains(&r.cursor);
        let progress = Progress {
            users: outline.iter().filter(|r| r.role == "user").filter(within).count(),
            rows: outline.iter().filter(within).count(),
            cut,
            retry,
        };
        if !due(progress, keeping.extract_every, keeping.extract_rows) { return Ok(()); }
        let limit = keeping.extract_bytes.clamp(4_096, 1_000_000) as usize;
        let mut input = format!("Today is {}.\n\nNotes already kept:\n", day(at.now));
        for note in scrollback.notes()? {
            let pin = if note.pinned { "[pinned] " } else { "" };
            let listed = format!("- {} {pin}{}: {}\n", note.id, note.title, note.description);
            if input.len() + listed.len() > limit / 8 { break; }
            input.push_str(&listed);
        }
        input.push_str("\nNew original transcript rows, each starting with its cursor in square brackets:\n");
        let rows = scrollback.read(session, &Want::Span {from, to}, &Budget {tokens:usize::MAX,turns:usize::MAX})?;
        let mut sources = Vec::new();
        let mut end = None;
        let mut piece = Value::Null;
        let mut withheld = Vec::new();
        for turn in rows.turns.iter().filter(|t| (from..=to).contains(&t.cursor)) {
            let (shown, kept_back) = crate::queue::guarded(turn, usize::MAX, hooks);
            if kept_back {
                withheld.push(turn.cursor);
            }
            if input.len() + shown.len() > limit {
                // A row wider than the whole budget is read a piece at a time, head first, and
                // only when it is the first thing here: a piece taken after other rows would
                // leave them re-read on every pass.
                if end.is_none() {
                    piece = chunk(scrollback, session, turn, limit - input.len(), &mut input)?;
                    if piece.is_null() {
                        scrollback.block_extraction(session, Some(&json!({"cursor":turn.cursor,"required_bytes":input.len()+shown.len(),"limit_bytes":limit,"reason":"extraction input budget leaves no room for even one piece of this row; raise extract_bytes"})))?;
                        return Ok(());
                    }
                    sources.push(provenance::source(turn, usize::MAX));
                }
                break;
            }
            input.push_str(&shown);
            sources.push(provenance::source(turn, usize::MAX));
            end = Some(turn.cursor);
        }
        // What it covers is still recorded, so evidence is checked the same way; whether that
        // becomes coverage is `acknowledge`'s, and a row only partly read never does.
        let Some(to) = end.or_else(|| piece["cursor"].as_u64()) else { return Ok(()) };
        let covers = json!({"version":1,"from":from,"to":to});
        let spec = json!({"kind":"extract","role":"notes","fallback":"skip",
            "instruction":format!("{EXTRACT}\n\n{}", keeping.checklist),"input":input,"schema":ops_schema(),
            // Reading a whole transcript for what is worth keeping is work, not a lookup: with
            // nothing to think with, a model answers `{"ops": []}` in six tokens every time.
            "thinking":"low",
            "max_tokens":2_000,"blocking":false,"timeout_ms":60_000,"covers":[from,to]});
        // What was kept back is recorded, so nothing reading this back can take the span for
        // fully read: coverage advances over a withheld row, and says that it was withheld.
        let context = json!({"from":from,"to":to,"cut":cut,"coverage":covers,"piece":piece,
            "withheld":withheld,
            "provenance":{"version":1,"sources":sources}});
        scrollback.block_extraction(session, None)?;
        scrollback.queue_job(session, "extract", &spec, &context, at.now)?;
        Ok(())
    })
}

pub(crate) fn validate(text: &str) -> Result<(), &'static str> {
    let said = crate::queue::json_in(text).ok_or("extraction answer is not a JSON object")?;
    let ops = said
        .get("ops")
        .and_then(Value::as_array)
        .filter(|ops| ops.len() <= 128)
        .ok_or("extraction answer needs an ops array of at most 128 operations")?;
    for op in ops {
        let kind = op
            .get("op")
            .and_then(Value::as_str)
            .ok_or("note operation needs an op")?;
        let nonempty = |name: &str| {
            op.get(name)
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty())
        };
        match kind {
            "add" if nonempty("title") && nonempty("text") => {}
            "update"
                if nonempty("id")
                    && ["title", "text", "description", "pinned"]
                        .iter()
                        .any(|k| op.get(*k).is_some()) => {}
            "retire" if nonempty("id") => {}
            _ => return Err("note operation has an unknown kind or missing required fields"),
        }
        for field in ["id", "title", "text", "description", "duplicate_of"] {
            if op.get(field).is_some() && !nonempty(field) {
                return Err("note text fields must be nonempty strings");
            }
        }
        if op.get("pinned").is_some_and(|v| !v.is_boolean()) {
            return Err("note pinned flag must be boolean");
        }
    }
    Ok(())
}

pub(crate) fn status(scrollback: &Transcript, session: &SessionId) -> Result<Value, StoreError> {
    let jobs: Vec<_> = scrollback
        .jobs_of(session)?
        .iter()
        .map(|j| {
            json!({"id":j.id,"kind":j.kind,
        "state":j.state.as_str(),"attempts":j.attempts,"covers":j.spec["covers"],"result":j.result})
        })
        .collect();
    let mut progress = serde_json::to_value(scrollback.extraction_progress(session)?)?;
    progress["partial"] = partial(scrollback, session)?.unwrap_or(Value::Null);
    Ok(json!({"extraction":progress,"jobs":jobs}))
}

/// Write the next unread piece of an oversized row into `input`, and describe it.
///
/// Head first from the contiguous coverage recorded for this revision, so a row amended since
/// starts again rather than looking partly read. `Null` when not even one character fits.
fn chunk(
    scrollback: &Transcript,
    session: &SessionId,
    turn: &balthasar_store::Turn,
    room: usize,
    input: &mut String,
) -> Result<Value, StoreError> {
    let revision = balthasar_store::revision(&turn.text);
    let read = scrollback.pieces_read(session, turn.cursor, &revision)?;
    let from = balthasar_store::covered(&read);
    let head = format!(
        "[{}] (continued at {from} of {}) ",
        turn.cursor,
        turn.text.len()
    );
    let Some(rest) = turn.text.get(from..).filter(|rest| !rest.is_empty()) else {
        return Ok(Value::Null);
    };
    let Some(taken) = balthasar_store::cut(turn.cursor, rest, room.saturating_sub(head.len() + 1))
        .into_iter()
        .next()
        .filter(|taken| head.len() + taken.text.len() < room)
    else {
        return Ok(Value::Null);
    };
    input.push_str(&head);
    input.push_str(&taken.text);
    input.push('\n');
    Ok(json!({ "cursor": turn.cursor, "revision": revision,
               "offset": from, "length": taken.text.len(),
               "of": turn.text.len() }))
}

/// Record what one finished extraction read: whole rows as coverage, a piece as a piece.
///
/// A piece that completes its row advances coverage over that row as well; a piece that does not
/// advances nothing, so the unread tail stays unread rather than being marked seen.
pub(crate) fn acknowledge(
    scrollback: &Transcript,
    session: &SessionId,
    job: &balthasar_store::Job,
    id: &str,
) -> Result<(), StoreError> {
    let piece = &job.context["piece"];
    if let (Some(cursor), Some(revision), Some(offset), Some(length)) = (
        piece["cursor"].as_u64(),
        piece["revision"].as_str(),
        piece["offset"].as_u64(),
        piece["length"].as_u64(),
    ) {
        scrollback.read_piece(
            session,
            cursor,
            revision,
            balthasar_store::Piece {
                offset: offset as usize,
                length: length as usize,
            },
        )?;
        let read = scrollback.pieces_read(session, cursor, revision)?;
        let whole = piece["of"].as_u64().unwrap_or(u64::MAX) as usize;
        if balthasar_store::covered(&read) >= whole {
            let from = job.context["from"].as_u64().unwrap_or(cursor);
            scrollback.acknowledge_extraction(session, id, from, cursor)?;
        }
        return Ok(());
    }
    if job.context["coverage"]["version"] != 1 {
        return Err(StoreError::Foreign("extraction coverage is missing".into()));
    }
    let coverage = &job.context["coverage"];
    let (Some(from), Some(to)) = (coverage["from"].as_u64(), coverage["to"].as_u64()) else {
        return Err(StoreError::Foreign("extraction coverage is missing".into()));
    };
    scrollback.acknowledge_extraction(session, id, from, to)
}

/// How far a row that is being read in pieces has been read, for the status to say so.
pub(crate) fn partial(
    scrollback: &Transcript,
    session: &SessionId,
) -> Result<Option<Value>, StoreError> {
    for job in scrollback.jobs_of(session)? {
        let piece = &job.context["piece"];
        let (Some(cursor), Some(revision), Some(of)) = (
            piece["cursor"].as_u64(),
            piece["revision"].as_str(),
            piece["of"].as_u64(),
        ) else {
            continue;
        };
        let read = balthasar_store::covered(&scrollback.pieces_read(session, cursor, revision)?);
        if (read as u64) < of {
            return Ok(Some(json!({ "cursor": cursor, "read": read, "of": of })));
        }
    }
    Ok(None)
}
