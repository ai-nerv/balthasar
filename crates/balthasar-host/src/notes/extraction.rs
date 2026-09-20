//! Bounded whole-row extraction, visible failures, and completion shape validation.

use super::{EXTRACT, Keeping, day, line, ops_schema, provenance};
use crate::Answering;
use balthasar_model::SessionId;
use balthasar_store::{Budget, JobState, StoreError, Transcript, Want};
use serde_json::{Value, json};

pub(crate) fn queue(
    at: &Answering<'_>,
    session: &SessionId,
    upto: Option<u64>,
    keeping: &Keeping,
    cut: bool,
    retry: bool,
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
        let users = outline.iter().filter(|r| r.role == "user" && (from..=to).contains(&r.cursor)).count();
        if !cut && !retry && users < keeping.extract_every as usize { return Ok(()); }
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
        for turn in rows.turns.iter().filter(|t| (from..=to).contains(&t.cursor)) {
            let shown = line(turn, usize::MAX);
            if input.len() + shown.len() > limit {
                if end.is_none() {
                    scrollback.block_extraction(session, Some(&json!({"cursor":turn.cursor,"required_bytes":input.len()+shown.len(),"limit_bytes":limit,"reason":"original row exceeds extraction input budget; increase extract_bytes and explicitly retry"})))?;
                    return Ok(());
                }
                break;
            }
            input.push_str(&shown);
            sources.push(provenance::source(turn, usize::MAX));
            end = Some(turn.cursor);
        }
        let Some(to) = end else { return Ok(()) };
        let spec = json!({"kind":"extract","role":"notes","fallback":"skip",
            "instruction":format!("{EXTRACT}\n\n{}", keeping.checklist),"input":input,"schema":ops_schema(),
            // Reading a whole transcript for what is worth keeping is work, not a lookup: with
            // nothing to think with, a model answers `{"ops": []}` in six tokens every time.
            "thinking":"low",
            "max_tokens":2_000,"blocking":false,"timeout_ms":60_000,"covers":[from,to]});
        let context = json!({"from":from,"to":to,"cut":cut,"coverage":{"version":1,"from":from,"to":to},
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
    Ok(json!({"extraction":scrollback.extraction_progress(session)?,"jobs":jobs}))
}
