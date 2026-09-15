//! Notes, Letta-style: pinned ones are in front of the model every time, deferred ones only as a
//! line each until opened. A helper extracts them in the background, every change is logged with
//! what it replaced, and a setting has the main model review changes before they apply.

use crate::Answering;
use crate::queue::{can_run, json_in, pending};
use balthasar_ipc::{Reply, Request};
use balthasar_model::{SessionId, Timestamp};
use balthasar_store::{Change, Job, Note, Prompt, StoreError, Transcript, Turn, Want};
use serde_json::{Value, json};

mod ruling;
use ruling::{echoes, laid_down, unpinned};

/// `balthasar.memory`: how notes are kept.
#[derive(Debug, Clone, PartialEq)]
pub struct Keeping {
    /// Changes wait for the main model's approval before they apply.
    pub review: bool,
    /// User turns between extractions.
    pub extract_every: u32,
    /// Extractions between tidy-ups.
    pub tidy_every: u32,
    /// What an extraction works through.
    pub checklist: String,
}

impl Default for Keeping {
    fn default() -> Self {
        Self {
            review: false,
            extract_every: 1,
            tidy_every: 10,
            checklist: CHECKLIST.to_owned(),
        }
    }
}

impl Keeping {
    /// Read `balthasar.memory`, keeping the shipped value for anything missing or unusable.
    #[must_use]
    pub fn read(said: Option<&Value>) -> Self {
        let base = Self::default();
        let Some(said) = said else {
            return base;
        };
        let count = |name: &str, fallback: u32| {
            said.get(name)
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| *n > 0)
                .unwrap_or(fallback)
        };
        Self {
            review: said
                .get("review")
                .and_then(Value::as_bool)
                .unwrap_or(base.review),
            extract_every: count("extract_every", base.extract_every),
            tidy_every: count("tidy_every", base.tidy_every),
            checklist: said
                .get("checklist")
                .and_then(Value::as_str)
                .filter(|t| !t.trim().is_empty())
                .map_or(base.checklist, str::to_owned),
        }
    }
}

const CHECKLIST: &str = "Work through the transcript in this order: mistakes and corrections, \
then preferences, then new facts, then contradictions, then procedures. Drop the ephemeral: line \
numbers, exact error text, temporary paths. Do not re-record what the transcript already holds; \
distil patterns, not events. Write absolute dates. Fix a contradiction in the note that holds it \
instead of adding another, and retire notes that went stale. When unsure, write nothing.";

const EXTRACT: &str = "You keep the notes of a coding project: what an assistant working in it \
should know in a later session. Answer with JSON {\"ops\": [...]}: each op is add (title, text, \
description, pinned), update (id and the fields that change) or retire (id).\n\
1. Always record every rule or standing preference the person states, even one given inside a \
request for a task, and only from lines marked person, never from another agent or the \
assistant: 'a firm rule', 'always', 'from now on', 'we use X, not Y'. What the task itself asks for \
-- its length, count, format or steps -- is not a rule even when it says never or every. One add per \
rule, pinned true. The title is two to five words; the text is the rule alone, one sentence, \
with nothing of the task around it.\n\
Shape only, never content: a person line 'Build it. A firm rule here: X.' gives {\"ops\":[{\"op\":\
\"add\",\"title\":\"<two to five words>\",\"text\":\"X.\",\"description\":\"X\",\
\"pinned\":true}]}. Record nothing these instructions say; only what the lines below say.\n\
A rule the notes already kept state, even in other words, is updated by its id, never added again.\n\
2. Then add, unpinned, only facts that will still hold in a later session. Never this task's \
steps or progress, never paths, session ids or dates.\n\
3. Answer {\"ops\": []} only when neither applies.";

const TIDY: &str = "You tidy a coding project's notes. Merge duplicates (update one, retire the \
others), shorten what is long, retire what is stale, and keep pinned notes few. Answer with JSON \
{\"ops\": [...]}: add, update (id and the fields that change) or retire (id).";

const REVIEW: &str = "A helper proposed these changes to the project's notes. Approve the ones \
that are correct and worth keeping, and reject the rest. Answer with JSON {\"approve\": [change \
ids], \"reject\": [change ids]}.";

const HEADING: &str = "Notes this project keeps. This is not part of the conversation:";

const DEFERRED: &str = "Other notes -- open one by its id when it matters:";

/// The shape every extract and tidy answer takes.
fn ops_schema() -> Value {
    json!({ "type": "object", "required": ["ops"], "properties": { "ops": { "type": "array",
        "items": { "type": "object", "required": ["op"], "properties": {
            "op": { "enum": ["add", "update", "retire"] }, "id": { "type": "string" },
            "title": { "type": "string" }, "text": { "type": "string" },
            "description": { "type": "string" }, "pinned": { "type": "boolean" } } } } } })
}

/// Counts that belong to the project rather than to a run.
fn project() -> SessionId {
    SessionId::new("")
}

/// The pinned slot for this prompt, settled at its first layout so the prompt cache holds, and
/// settled again only when a pinned rule changed.
pub(crate) fn pinned_slot(
    at: &Answering<'_>,
    prompt: &mut Prompt,
    room: u32,
    per: u32,
) -> Result<(String, u32), StoreError> {
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Ok((String::new(), 0));
    };
    let notes = scrollback.notes()?;
    // A rule pinned since the prompt began is worth one miss of the prompt cache.
    let rules: String = notes
        .iter()
        .filter(|n| n.pinned)
        .map(|n| format!("{}: {}\n", n.title, n.text.trim()))
        .collect();
    if let Some(kept) = &prompt.pinned
        && kept["rules"].as_str().unwrap_or_default() == rules
    {
        let tokens = kept["tokens"].as_u64().and_then(|n| u32::try_from(n).ok());
        return Ok((
            kept["text"].as_str().unwrap_or_default().to_owned(),
            tokens.unwrap_or(0),
        ));
    }
    let total = room as usize * per.max(1) as usize;
    let mut out = format!("{HEADING}\n");
    let mut wrote = false;
    for note in notes.iter().filter(|n| n.pinned) {
        let line = format!("- {}: {}\n", note.title, note.text.trim());
        if out.len() + line.len() <= total {
            out.push_str(&line);
            wrote = true;
        }
    }
    let mut under = false;
    for note in notes.iter().filter(|n| !n.pinned) {
        let line = format!("- {} {}: {}\n", note.id, note.title, note.description);
        let heading = if under { 0 } else { DEFERRED.len() + 1 };
        if out.len() + heading + line.len() > total {
            continue;
        }
        if !under {
            out.push_str(DEFERRED);
            out.push('\n');
            under = true;
        }
        out.push_str(&line);
        wrote = true;
    }
    let text = if wrote {
        out.trim_end().to_owned()
    } else {
        String::new()
    };
    let tokens = u32::try_from(text.len().div_ceil(per.max(1) as usize)).unwrap_or(u32::MAX);
    prompt.pinned = Some(json!({ "text": text, "tokens": tokens, "rules": rules }));
    Ok((text, tokens))
}

/// Queue what is due between turns: an extraction over what nothing has read yet, up to `upto`,
/// and a tidy-up every so many extractions. Only for a harness that can run memory jobs.
pub(crate) fn background(
    at: &Answering<'_>,
    session: &SessionId,
    helpers: &[String],
    upto: Option<u64>,
    keeping: &Keeping,
) -> Result<(), StoreError> {
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Ok(());
    };
    if !can_run(helpers, "memory") {
        return Ok(());
    }
    let jobs = scrollback.jobs_of(session)?;
    let outline = scrollback.outline(session)?;
    let Some(last) = outline.last().map(|t| t.cursor) else {
        return Ok(());
    };
    let upto = upto.unwrap_or(last).min(last);
    let from = scrollback
        .counter(session, "extracted")?
        .map_or(0, |done| done + 1);
    let users = outline
        .iter()
        .filter(|t| t.role == "user" && (from..=upto).contains(&t.cursor))
        .count();
    if from <= upto && users >= keeping.extract_every as usize && !pending(&jobs, "extract", at.now)
    {
        extract(at, session, (from, upto), keeping, false)?;
    }
    let since = scrollback.counter(&project(), "extracts")?.unwrap_or(0);
    if since >= u64::from(keeping.tidy_every)
        && !pending(&jobs, "tidy", at.now)
        && !scrollback.notes()?.is_empty()
    {
        tidy(at, session)?;
        scrollback.set_counter(&project(), "extracts", 0)?;
    }
    Ok(())
}

/// Before a summary replaces a span, what nothing has read of it yet is read for notes.
pub(crate) fn before_cut(
    at: &Answering<'_>,
    session: &SessionId,
    to: u64,
    helpers: &[String],
    keeping: &Keeping,
) -> Result<(), StoreError> {
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Ok(());
    };
    if !can_run(helpers, "memory") {
        return Ok(());
    }
    let from = scrollback
        .counter(session, "extracted")?
        .map_or(0, |done| done + 1);
    if from <= to {
        extract(at, session, (from, to), keeping, true)?;
    }
    Ok(())
}

/// Queue an extraction over `span`, with the notes already kept and what came just before as
/// context, and move the run's cursor past it.
fn extract(
    at: &Answering<'_>,
    session: &SessionId,
    (from, to): (u64, u64),
    keeping: &Keeping,
    cut: bool,
) -> Result<(), StoreError> {
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Ok(());
    };
    let everything = balthasar_store::Budget {
        tokens: usize::MAX,
        turns: usize::MAX,
    };
    let mut input = format!("Today is {}.\n\nNotes already kept:\n", day(at.now));
    let notes = scrollback.notes()?;
    if notes.is_empty() {
        input.push_str("(none)\n");
    }
    for note in &notes {
        let pinned = if note.pinned { "[pinned] " } else { "" };
        input.push_str(&format!(
            "- {} {pinned}{}: {}\n",
            note.id, note.title, note.description
        ));
    }
    if from > 0 {
        let span = Want::Span {
            from: from.saturating_sub(6),
            to: from - 1,
        };
        let before = scrollback.read(session, &span, &everything)?;
        input.push_str("\nAlready read, for context only:\n");
        for turn in &before.turns {
            input.push_str(&line(turn, 300));
        }
    }
    input.push_str(&format!("\nNew since then (turns {from}–{to}):\n"));
    let rows = scrollback.read(session, &Want::Span { from, to }, &everything)?;
    for turn in &rows.turns {
        if input.len() > 100_000 {
            input.push_str("…\n");
            break;
        }
        input.push_str(&line(turn, 2_000));
    }
    let spec = json!({
        "kind": "extract", "role": "memory", "fallback": "skip",
        "instruction": format!("{EXTRACT}\n\n{}", keeping.checklist), "input": input,
        "schema": ops_schema(), "max_tokens": 2_000, "blocking": false, "timeout_ms": 60_000,
        "covers": [from, to],
    });
    let context = json!({ "from": from, "to": to, "cut": cut });
    scrollback.queue_job(session, "extract", &spec, &context, at.now)?;
    scrollback.set_counter(session, "extracted", to)
}

/// One turn as a helper reads it.
fn line(turn: &Turn, limit: usize) -> String {
    let who = match (turn.role.as_str(), turn.kind.as_str(), turn.tool.as_deref()) {
        (_, _, Some(tool)) => format!("tool ({tool})"),
        ("user", "from", _) => "another agent".to_owned(),
        ("user", _, _) => "person".to_owned(),
        (role, _, _) => role.to_owned(),
    };
    let text: String = match &turn.stub {
        Some(stub) if turn.text.len() > limit => stub.clone(),
        _ => turn.text.chars().take(limit).collect(),
    };
    format!("[{}] {who}: {text}\n", turn.cursor)
}

/// Queue a tidy-up of every live note, saying what the pinned ones cost.
fn tidy(at: &Answering<'_>, session: &SessionId) -> Result<(), StoreError> {
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Ok(());
    };
    let notes = scrollback.notes()?;
    let pinned = pinned_tokens(&notes);
    let mut input =
        format!("Pinned notes cost about {pinned} tokens in every request.\n\nThe notes:\n");
    for note in &notes {
        let mark = if note.pinned { "[pinned] " } else { "" };
        input.push_str(&format!(
            "- {} {mark}{}: {} (listed as: {})\n",
            note.id, note.title, note.text, note.description
        ));
    }
    let spec = json!({
        "kind": "tidy", "role": "memory", "fallback": "skip", "instruction": TIDY,
        "input": input, "schema": ops_schema(), "max_tokens": 2_000, "blocking": false,
        "timeout_ms": 60_000,
    });
    scrollback.queue_job(
        session,
        "tidy",
        &spec,
        &json!({ "pinned_tokens": pinned }),
        at.now,
    )?;
    Ok(())
}

/// Queue a review of every staged change, unless one is already on its way.
fn review(at: &Answering<'_>, session: &SessionId) -> Result<(), StoreError> {
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Ok(());
    };
    if pending(&scrollback.jobs_of(session)?, "review", at.now) {
        return Ok(());
    }
    let staged: Vec<Change> = scrollback
        .changes(500)?
        .into_iter()
        .filter(|c| c.state == "staged")
        .collect();
    if staged.is_empty() {
        return Ok(());
    }
    let mut input = String::from("The proposed changes:\n");
    for change in staged.iter().rev() {
        input.push_str(&format!(
            "- {}: {} {} — before: {} — after: {}\n",
            change.id,
            change.op,
            change.note.as_deref().unwrap_or("(a new note)"),
            change
                .before
                .as_ref()
                .map_or("nothing".into(), ToString::to_string),
            change
                .after
                .as_ref()
                .map_or("retired".into(), ToString::to_string),
        ));
    }
    let spec = json!({
        "kind": "review", "role": "main", "fallback": "skip", "instruction": REVIEW,
        "input": input, "max_tokens": 1_000, "blocking": false, "timeout_ms": 60_000,
        "schema": { "type": "object", "required": ["approve", "reject"], "properties": {
            "approve": { "type": "array", "items": { "type": "string" } },
            "reject": { "type": "array", "items": { "type": "string" } } } },
    });
    scrollback.queue_job(session, "review", &spec, &json!({}), at.now)?;
    Ok(())
}

/// Put what an extract, tidy or review job answered into the notes.
pub(crate) fn settle(
    at: &Answering<'_>,
    session: &SessionId,
    job: &Job,
    text: &str,
    keeping: &Keeping,
) -> Result<(), StoreError> {
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Ok(());
    };
    let Some(said) = json_in(text) else {
        return Ok(());
    };
    match job.kind.as_str() {
        "extract" | "tidy" => {
            let ops = scrubbed(
                said["ops"].as_array().cloned().unwrap_or_default(),
                &at.scope.to_string(),
            );
            let ruled =
                job.kind != "extract" || laid_down(job.spec["input"].as_str().unwrap_or_default());
            let ops = if ruled { ops } else { unpinned(ops) };
            let pinned = |s: &Transcript| -> Result<usize, StoreError> {
                Ok(s.notes()?.iter().filter(|n| n.pinned).count())
            };
            let before = pinned(scrollback)?;
            let made = propose(scrollback, session, &job.id, &ops, keeping, at.now)?;
            if job.kind == "extract" {
                let since = scrollback.counter(&project(), "extracts")?.unwrap_or(0);
                scrollback.set_counter(&project(), "extracts", since + 1)?;
                // A new pinned rule may say what one already does in other words: tidied now, not ten
                // extractions later, since every pinned note rides along with every request.
                let after = pinned(scrollback)?;
                if after > before
                    && after > 1
                    && !pending(&scrollback.jobs_of(session)?, "tidy", at.now)
                {
                    tidy(at, session)?;
                    scrollback.set_counter(&project(), "extracts", 0)?;
                }
            }
            if keeping.review && made > 0 {
                review(at, session)?;
            }
        }
        "review" => {
            decide(scrollback, &ids(&said["approve"]), true, at.now)?;
            decide(scrollback, &ids(&said["reject"]), false, at.now)?;
        }
        _ => {}
    }
    Ok(())
}

/// Log each op as a change, and apply it unless changes are reviewed first. An add whose title a
/// live note already has is an update of that note. Answers how many changes were made.
fn propose(
    scrollback: &Transcript,
    session: &SessionId,
    by: &str,
    ops: &[Value],
    keeping: &Keeping,
    now: Timestamp,
) -> Result<usize, StoreError> {
    let live = scrollback.notes()?;
    let mut made = 0;
    for op in ops {
        let target = op["id"]
            .as_str()
            .and_then(|id| live.iter().find(|n| n.id == id))
            .or_else(|| {
                let title = op["title"].as_str()?.trim();
                live.iter().find(|n| n.title.eq_ignore_ascii_case(title))
            });
        if op["op"].as_str() == Some("add") && target.is_none() && echoes(op, &live) {
            continue;
        }
        let (note, before, after, kind) = match (op["op"].as_str(), target) {
            (Some("retire"), Some(n)) => (Some(n.id.clone()), Some(n.fields()), None, "retire"),
            (Some("add" | "update"), Some(n)) => {
                let Some(after) = merged(op, Some(n)).filter(|a| *a != n.fields()) else {
                    continue;
                };
                (Some(n.id.clone()), Some(n.fields()), Some(after), "update")
            }
            (Some("add"), None) => {
                let Some(after) = merged(op, None) else {
                    continue;
                };
                (None, None, Some(after), "add")
            }
            _ => continue,
        };
        let change = Change {
            note,
            op: kind.to_owned(),
            before,
            after,
            state: "staged".to_owned(),
            by: by.to_owned(),
            at: now,
            ..Change::default()
        };
        let id = scrollback.record_change(&change, Some(session))?;
        if !keeping.review {
            apply(scrollback, &id, now)?;
        }
        made += 1;
    }
    Ok(made)
}

/// A note's fields after an op: what the op says, else what the note had. A description nobody
/// wrote is the text's first line.
fn merged(op: &Value, base: Option<&Note>) -> Option<Value> {
    let pick = |name: &str, fallback: Option<&str>| {
        op[name]
            .as_str()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .or(fallback)
            .map(str::to_owned)
    };
    let title = pick("title", base.map(|n| n.title.as_str()))?;
    let text = pick("text", base.map(|n| n.text.as_str()))?;
    let description =
        pick("description", base.map(|n| n.description.as_str())).unwrap_or_else(|| {
            text.lines()
                .next()
                .unwrap_or_default()
                .chars()
                .take(120)
                .collect()
        });
    let pinned = op["pinned"]
        .as_bool()
        .or(base.map(|n| n.pinned))
        .unwrap_or(false);
    Some(json!({ "title": title, "text": text, "description": description, "pinned": pinned }))
}

/// Make a logged change so.
fn apply(scrollback: &Transcript, id: &str, now: Timestamp) -> Result<(), StoreError> {
    let Some(change) = scrollback.change(id)? else {
        return Ok(());
    };
    let note = scrollback.put_note(change.note.as_deref(), change.after.as_ref(), now)?;
    balthasar_model::noted!(
        "note: change {id} {} {} applied, by {}",
        change.op,
        note.as_deref().unwrap_or("-"),
        change.by
    );
    scrollback.set_change(id, "applied", note.as_deref())
}

/// Approve or reject staged changes. Answers which ones were.
fn decide(
    scrollback: &Transcript,
    ids: &[String],
    approve: bool,
    now: Timestamp,
) -> Result<Vec<String>, StoreError> {
    let mut done = Vec::new();
    for id in ids {
        if scrollback.change(id)?.is_none_or(|c| c.state != "staged") {
            continue;
        }
        if approve {
            apply(scrollback, id, now)?;
        } else {
            balthasar_model::noted!("note: change {id} rejected");
            scrollback.set_change(id, "rejected", None)?;
        }
        done.push(id.clone());
    }
    Ok(done)
}

fn ids(said: &Value) -> Vec<String> {
    said.as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn pinned_tokens(notes: &[Note]) -> usize {
    notes
        .iter()
        .filter(|n| n.pinned)
        .map(|n| n.title.len() + n.text.len() + 4)
        .sum::<usize>()
        .div_ceil(4)
}

/// `notes(session)`: the pinned notes whole, the rest by title and description.
pub fn notes(at: &mut Answering<'_>) -> Reply {
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Reply::refused("notes need a scrollback");
    };
    match scrollback.notes() {
        Ok(notes) => Reply::one(json!({
            "pinned": notes.iter().filter(|n| n.pinned)
                .map(|n| json!({ "id": n.id, "title": n.title, "text": n.text }))
                .collect::<Vec<_>>(),
            "deferred": notes.iter().filter(|n| !n.pinned)
                .map(|n| json!({ "id": n.id, "title": n.title, "description": n.description }))
                .collect::<Vec<_>>(),
            "pinned_tokens": pinned_tokens(&notes),
        })),
        Err(why) => Reply::refused(why.to_string()),
    }
}

/// `note_open(session, {id})`: one note in full.
pub fn note_open(at: &mut Answering<'_>, request: &Request) -> Reply {
    let Some(id) = named(request, "id") else {
        return Reply::refused("note_open needs a note's id");
    };
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Reply::refused("notes need a scrollback");
    };
    match scrollback.note(&id) {
        Ok(Some(note)) => {
            let mut out = note.fields();
            out["id"] = json!(note.id);
            out["updated"] = json!(note.updated);
            out["retired"] = json!(note.retired);
            Reply::one(out)
        }
        Ok(None) => Reply::refused(format!("no note called '{id}'")),
        Err(why) => Reply::refused(why.to_string()),
    }
}

/// `changes(session, {limit})`: the change log, newest first.
pub fn changes(at: &mut Answering<'_>, request: &Request) -> Reply {
    let limit = request
        .args
        .get(1)
        .and_then(|s| s["limit"].as_u64())
        .unwrap_or(20)
        .min(500) as usize;
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Reply::refused("notes need a scrollback");
    };
    match scrollback.changes(limit) {
        Ok(found) => Reply::rows(found.iter().map(Change::as_json).collect()),
        Err(why) => Reply::refused(why.to_string()),
    }
}

/// `undo(session, {change})`: put a note back as an applied change found it, and log that too.
pub fn undo(at: &mut Answering<'_>, request: &Request) -> Reply {
    let Some(id) = named(request, "change") else {
        return Reply::refused("undo needs a change's id");
    };
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Reply::refused("notes need a scrollback");
    };
    let reverted = (|| {
        let Some(change) = scrollback.change(&id)? else {
            return Ok(Err(format!("no change called '{id}'")));
        };
        if change.state != "applied" {
            return Ok(Err(format!(
                "change '{id}' is {}, not applied",
                change.state
            )));
        }
        let current = match &change.note {
            Some(note) => scrollback
                .note(note)?
                .filter(|n| n.retired.is_none())
                .map(|n| n.fields()),
            None => None,
        };
        let revert = Change {
            note: change.note.clone(),
            op: "revert".to_owned(),
            before: current,
            after: change.before.clone(),
            state: "staged".to_owned(),
            by: format!("undo {id}"),
            at: at.now,
            ..Change::default()
        };
        let made = scrollback.record_change(&revert, None)?;
        apply(scrollback, &made, at.now)?;
        scrollback.set_change(&id, "undone", None)?;
        Ok::<_, StoreError>(Ok(made))
    })();
    match reverted {
        Ok(Ok(made)) => Reply::one(json!({ "undone": id, "change": made })),
        Ok(Err(why)) => Reply::refused(why),
        Err(why) => Reply::refused(why.to_string()),
    }
}

/// `approve(session, {changes})` and `reject(session, {changes})`: settle staged changes.
pub fn decided(at: &mut Answering<'_>, request: &Request, approve: bool) -> Reply {
    let asked = ids(&request.args.get(1).cloned().unwrap_or_default()["changes"]);
    if asked.is_empty() {
        return Reply::refused("name the changes: { changes = [...] }");
    }
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Reply::refused("notes need a scrollback");
    };
    match decide(scrollback, &asked, approve, at.now) {
        Ok(done) => Reply::one(json!({ if approve { "approved" } else { "rejected" }: done })),
        Err(why) => Reply::refused(why.to_string()),
    }
}

/// A field of the second argument, or the second argument itself when it is a bare string.
fn named(request: &Request, field: &str) -> Option<String> {
    let said = request.args.get(1)?;
    said.as_str()
        .or_else(|| said.get(field)?.as_str())
        .map(str::to_owned)
}

/// `YYYY-MM-DD` of a moment, in UTC.
pub(crate) fn day(at: Timestamp) -> String {
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

#[cfg(test)]
mod tests;

/// Ops with the project's own path taken out: a note is read in later sessions from wherever the
/// project then is, so an absolute path into it is the ephemeral the checklist says to drop.
fn scrubbed(ops: Vec<Value>, root: &str) -> Vec<Value> {
    if root.len() < 2 {
        return ops;
    }
    let within = format!("{root}/");
    ops.into_iter()
        .map(|mut op| {
            if let Some(fields) = op.as_object_mut() {
                for name in ["title", "text", "description"] {
                    if let Some(Value::String(said)) = fields.get_mut(name) {
                        *said = said.replace(&within, "").replace(root, ".");
                    }
                }
            }
            op
        })
        .collect()
}

#[cfg(test)]
mod scrubbing {
    use super::*;

    #[test]
    fn a_note_never_keeps_the_projects_absolute_path() {
        let ops = vec![
            json!({ "op": "add", "title": "layout",
            "text": "Tests live in /work/app/tests; run from /work/app.", "pinned": false }),
            json!(7),
        ];
        let clean = scrubbed(ops, "/work/app");
        assert_eq!(clean[0]["text"], "Tests live in tests; run from ..");
        assert_eq!(
            clean[1],
            json!(7),
            "anything that is not an op is left alone"
        );
    }
}
