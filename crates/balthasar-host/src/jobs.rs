//! Jobs: helper-model work balthasar asks the harness to run, and what it does with the answers.
//! balthasar never calls a model; a job is text in and text out, run by whoever holds one.

use crate::queue::{json_in, stale};
use crate::supply;
use crate::{Answering, Hooks};
use balthasar_buffer::Rules;
use balthasar_ipc::{Reply, Request};
use balthasar_model::SessionId;
use balthasar_store::{Job, JobState, StoreError, Want};
use serde_json::{Value, json};

/// A failed job is handed out once more, and then left failed.
const RETRIES: u32 = 1;

/// How many search hits a curate job chooses from.
const CURATE_FROM: usize = 50;

/// What one row may bring into a summary's input, in characters: even a long reply fits, and the
/// whole input is still held to [`INPUT`].
const PER_ROW: usize = 64_000;

/// What a large tool output brings instead: its head, since its size is the point.
const TOOL_HEAD: usize = 2_000;

/// What a summary's whole input may be, in characters.
const INPUT: usize = 200_000;

const SUMMARISE: &str = "You keep the running summary of a coding session between a person and \
an assistant. Fold the turns given into the previous summary, if there is one, and write the \
whole summary again. Sections: Goal; Decisions and why; State (files touched, what works, what \
fails -- every call that failed, what it was for and what it said); Open threads and next steps; \
Lookup keywords -- one line of exact file paths, \
identifiers, commands and error names someone would search the transcript for to find the \
details again. Quote the person's requests word for word. Write absolute dates. Answer with \
the summary alone.";

const CURATE: &str = "You choose what an assistant should be reminded of before it answers a \
prompt. From the memories given, pick the few that matter for this prompt -- none, if none do \
-- and write each as one short note. Never invent a fact that is not in a memory. Answer with \
JSON: {\"chosen\": [memory ids], \"notes\": [one short note each]}.";

/// Queue a summary of `from..=to`, folding in the stored one.
pub(crate) fn summarise(
    at: &Answering<'_>,
    session: &SessionId,
    span: (u64, u64),
    rules: &Rules,
    max_tokens: u32,
) -> Result<(), StoreError> {
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Ok(());
    };
    let previous = scrollback.summary(session)?;
    let fresh = previous.as_ref().map_or(span.0, |s| s.to + 1);
    let everything = balthasar_store::Budget {
        tokens: usize::MAX,
        turns: usize::MAX,
    };
    let rows = scrollback.read(
        session,
        &Want::Span {
            from: fresh,
            to: span.1,
        },
        &everything,
    )?;
    // Dated, since the summary is told to write absolute dates: without it the helper invented one.
    let mut input = format!("Today is {}.\n\n", crate::calendar::day(at.now));
    if let Some(s) = &previous {
        input.push_str(&format!(
            "Previous summary (turns {}–{}):\n{}\n\n",
            s.from, s.to, s.text
        ));
    }
    input.push_str(&format!("Turns {fresh}–{} to fold in:\n", span.1));
    for turn in &rows.turns {
        let who = turn.tool.as_deref().map_or_else(
            || turn.role.clone(),
            |tool| format!("{} ({tool})", turn.role),
        );
        let text = if input.len() > INPUT {
            turn.stub.clone().unwrap_or_else(|| "(elided)".to_owned())
        } else if turn.is_tool() && turn.weight() > rules.stub_over {
            let head: String = turn.text.chars().take(TOOL_HEAD).collect();
            format!(
                "{} … ({} tokens of output)",
                turn.stub.as_deref().unwrap_or(&head),
                turn.weight()
            )
        } else {
            turn.text.chars().take(PER_ROW).collect()
        };
        input.push_str(&format!("[{}] {who}: {text}\n", turn.cursor));
    }
    let spec = json!({
        "kind": "summarise", "role": "memory", "fallback": "main",
        "instruction": SUMMARISE, "input": input, "schema": null,
        "max_tokens": max_tokens.clamp(256, 8_000), "blocking": false, "timeout_ms": 60_000,
        "covers": [span.0, span.1],
    });
    scrollback.queue_job(
        session,
        "summarise",
        &spec,
        &json!({ "from": span.0, "to": span.1 }),
        at.now,
    )?;
    Ok(())
}

/// What heads the calls a summary carries as they were. Looked for, too: a helper folding the last
/// summary into the next copies this part as readily as the rest, and it is written again whole.
const FAILED: &str = "Calls that failed in the turns summarised above, as they were made and \
                      answered (not the summariser's words):";
/// The most failed calls a summary carries, newest kept, and the most of each that is shown.
const FAILED_KEPT: usize = 20;
const FAILED_SHOWN: usize = 300;

/// A summary without a list of failed calls copied into it from the last one: the heading and the
/// lines of the list, and nothing the helper wrote before or after them.
fn without_failures(summary: &str) -> String {
    let heading: String = FAILED.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut listing = false;
    let mut out = Vec::new();
    for line in summary.lines() {
        let plain: String = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if plain == heading {
            listing = true;
            continue;
        }
        if listing && (plain.starts_with("- [") || plain.starts_with('(') || plain.is_empty()) {
            continue;
        }
        listing = false;
        out.push(line);
    }
    out.join("\n")
}

/// A summary with the failed calls of everything it covers set under it, word for word. A failure
/// is what stops the same thing being tried again, and a summary is somebody's account of a span:
/// dropping a span spares its failed calls, so summarising it must not lose them either, and a
/// span cannot be summarised around a row in the middle of it.
fn with_failures(
    scrollback: &balthasar_store::Transcript,
    session: &SessionId,
    span: (u64, u64),
    summary: &str,
) -> Result<String, StoreError> {
    let own = without_failures(summary);
    let own = own.trim_end();
    let everything = balthasar_store::Budget {
        tokens: usize::MAX,
        turns: usize::MAX,
    };
    let rows = scrollback.read(
        session,
        &Want::Span {
            from: span.0,
            to: span.1,
        },
        &everything,
    )?;
    let failed: Vec<_> = rows
        .turns
        .iter()
        .filter(|turn| turn.is_tool() && (turn.error || turn.ok == Some(false)))
        .collect();
    if failed.is_empty() {
        return Ok(own.to_owned());
    }
    let shown = |text: &str| -> String {
        let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if one.chars().count() > FAILED_SHOWN {
            format!("{}…", one.chars().take(FAILED_SHOWN).collect::<String>())
        } else {
            one
        }
    };
    let mut out = format!("{own}\n\n{FAILED}\n");
    if failed.len() > FAILED_KEPT {
        out.push_str(&format!(
            "({} earlier ones are not listed.)\n",
            failed.len() - FAILED_KEPT
        ));
    }
    for turn in &failed[failed.len().saturating_sub(FAILED_KEPT)..] {
        out.push_str(&format!(
            "- [{}] {} {} -> {}\n",
            turn.cursor,
            turn.tool.as_deref().unwrap_or("tool"),
            shown(turn.args.as_deref().unwrap_or_default()),
            shown(&turn.text)
        ));
    }
    Ok(out.trim_end().to_owned())
}

/// Queue a choice among what a search found for this prompt, when there is anything to choose.
pub(crate) fn curate(
    at: &mut Answering<'_>,
    session: &SessionId,
    query: &str,
    mark: &str,
    room: u32,
    hooks: &mut dyn Hooks,
) -> Result<(), StoreError> {
    let found = supply::candidates(at, query, CURATE_FROM);
    let mut listed = String::new();
    for hit in &found {
        if let Some(text) = hooks.redact(&hit.memory.text(), &hit.memory) {
            listed.push_str(&format!("- {}: {}\n", hit.memory.id, text.trim()));
        }
    }
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Ok(());
    };
    if listed.is_empty() {
        return Ok(());
    }
    let spec = json!({
        "kind": "curate", "role": "memory", "fallback": "skip",
        "instruction": CURATE,
        "input": format!("The prompt:\n{query}\n\nThe memories:\n{listed}"),
        "schema": {
            "type": "object",
            "properties": {
                "chosen": { "type": "array", "items": { "type": "string" } },
                "notes": { "type": "array", "items": { "type": "string" } },
            },
            "required": ["chosen", "notes"],
        },
        "max_tokens": 1_000, "blocking": true, "timeout_ms": 5_000,
    });
    let context = json!({ "mark": mark, "query": query, "room": room });
    scrollback.queue_job(session, "curate", &spec, &context, at.now)?;
    Ok(())
}

/// Hand out every queued job, retiring what went stale. A curate goes out only in the first
/// round of the prompt it was made for.
pub(crate) fn hand_out(
    at: &Answering<'_>,
    session: &SessionId,
    first_of: Option<&str>,
) -> Result<Vec<Value>, StoreError> {
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for job in scrollback.jobs_of(session)? {
        if stale(&job, at.now) {
            retry_or_fail(at, &job, Some(&json!({"failed":"helper timed out"})))?;
        }
    }
    for job in scrollback.jobs_of(session)? {
        if job.state != JobState::Queued {
            continue;
        }
        if job.kind == "curate" && first_of != job.context["mark"].as_str() {
            scrollback.settle_job(&job.id, JobState::Failed, None, at.now)?;
            continue;
        }
        scrollback.issue_job(&job.id, at.now)?;
        balthasar_model::noted!(
            "job: handed {} {} for {}, attempt {}",
            job.id,
            job.kind,
            job.spec["role"].as_str().unwrap_or("?"),
            job.attempts + 1
        );
        out.push(job.handed());
    }
    Ok(out)
}

/// A failed job goes back in the queue once; after that it stays failed. A blocking one is never
/// retried: it was there to shape one request, and that request has gone without it.
fn retry_or_fail(at: &Answering<'_>, job: &Job, result: Option<&Value>) -> Result<(), StoreError> {
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Ok(());
    };
    let blocking = job.spec["blocking"].as_bool() == Some(true);
    let next = if job.attempts <= RETRIES && !blocking {
        JobState::Queued
    } else {
        JobState::Failed
    };
    scrollback.settle_job(&job.id, next, result, at.now)
}

/// `jobs(session, {helpers})`: the background jobs waiting to be run, with what a finished turn
/// made due. The helpers the last layout named stand unless these name others.
pub fn jobs(at: &mut Answering<'_>, request: &Request, hooks: &mut dyn Hooks) -> Reply {
    let Some(session) = request.args.first().and_then(Value::as_str) else {
        return Reply::refused("jobs needs a session");
    };
    let session = SessionId::new(session);
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Reply::refused("jobs need a scrollback");
    };
    if request
        .args
        .get(1)
        .is_some_and(|opts| opts["inspect"] == true)
    {
        return match crate::notes::extraction::status(scrollback, &session) {
            Ok(status) => Reply::one(status),
            Err(why) => Reply::refused(why.to_string()),
        };
    }
    if request
        .args
        .get(1)
        .is_some_and(|opts| opts["retry_extraction"] == true)
        && let Err(why) =
            crate::notes::extraction::queue(at, &session, None, &hooks.memory(), false, true)
    {
        return Reply::refused(why.to_string());
    }
    let said = request.args.get(1).and_then(|s| s["helpers"].as_array());
    let helpers: Vec<String> = match said {
        Some(named) => named
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        None => scrollback
            .prompt(&session)
            .ok()
            .flatten()
            .map(|p| p.helpers)
            .unwrap_or_default(),
    };
    if let Err(why) = crate::notes::background(at, &session, &helpers, None, &hooks.memory()) {
        return Reply::refused(why.to_string());
    }
    match hand_out(at, &session, None) {
        Ok(jobs) => Reply::rows(jobs),
        Err(why) => Reply::refused(why.to_string()),
    }
}

/// `job_done(session, {id, text, usage, model} | {id, failed})`: what a job came back with.
pub fn job_done(at: &mut Answering<'_>, request: &Request, hooks: &mut dyn Hooks) -> Reply {
    let Some(session) = request.args.first().and_then(Value::as_str) else {
        return Reply::refused("job_done needs a session");
    };
    let session = SessionId::new(session);
    let said = request.args.get(1).cloned().unwrap_or(Value::Null);
    let Some(id) = said["id"].as_str() else {
        return Reply::refused("job_done needs the job's id");
    };
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Reply::refused("jobs need a scrollback");
    };
    let job = match scrollback.job(id) {
        Ok(Some(job)) if job.session == session => job,
        Ok(_) => return Reply::refused(format!("no job called '{id}' in '{session}'")),
        Err(why) => return Reply::refused(why.to_string()),
    };
    // A late or repeated answer changes nothing.
    if job.state != JobState::Issued {
        return Reply::none();
    }
    let text = said["text"]
        .as_str()
        .filter(|_| said.get("failed").is_none());
    let curated = match text {
        Some(text) if job.kind == "curate" => curation(at, &job, text, hooks),
        _ => None,
    };
    let keeping = hooks.memory();
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Reply::refused("jobs need a scrollback");
    };
    let settled = scrollback.atomic(|scrollback| {
        let Some(current) = scrollback.job(id)? else {
            return Err(StoreError::Unknown(format!("no job called '{id}'")));
        };
        if current.state != JobState::Issued {
            return Ok(());
        }
        if current != job {
            return Err(StoreError::Unknown(format!(
                "job '{id}' changed during completion"
            )));
        }
        match text {
            None => retry_or_fail(at, &current, Some(&json!({"failed":said["failed"].as_str().unwrap_or("helper returned no text"),"usage":said["usage"],"model":said["model"]}))),
            Some(text) => {
                if matches!(current.kind.as_str(), "extract" | "tidy")
                    && let Err(reason) = crate::notes::extraction::validate(text) {
                    return retry_or_fail(at, &current, Some(&json!({"failed":reason,"text":text,"usage":said["usage"],"model":said["model"]})));
                }
                if !settle(at, &session, &current, text, &keeping, curated.as_ref())? {
                    return retry_or_fail(at, &current, Some(&json!({"failed":"source evidence changed or note operations were rejected","text":text,"usage":said["usage"],"model":said["model"]})));
                }
                if current.kind == "extract" && current.context["coverage"]["version"] == 1 {
                    let coverage = &current.context["coverage"];
                    let (Some(from), Some(to)) = (coverage["from"].as_u64(), coverage["to"].as_u64()) else {
                        return Err(StoreError::Foreign("extraction coverage is missing".into()));
                    };
                    scrollback.acknowledge_extraction(&session, id, from, to)?;
                }
                let kept = json!({ "text": text, "usage": said["usage"], "model": said["model"] });
                scrollback.settle_job(id, JobState::Done, Some(&kept), at.now)
            }
        }
    });
    match settled {
        Ok(()) => Reply::none(),
        Err(why) => Reply::refused(why.to_string()),
    }
}

/// Put what a job wrote where it belongs.
fn settle(
    at: &Answering<'_>,
    session: &SessionId,
    job: &Job,
    text: &str,
    keeping: &crate::Keeping,
    curated: Option<&Value>,
) -> Result<bool, StoreError> {
    match job.kind.as_str() {
        "summarise" => {
            let (Some(from), Some(to)) = (job.context["from"].as_u64(), job.context["to"].as_u64())
            else {
                return Ok(false);
            };
            let Some(scrollback) = at.scrollback.as_ref() else {
                return Ok(false);
            };
            // A summary that already reaches further makes this one old news.
            if scrollback.summary(session)?.is_some_and(|s| s.to >= to) || text.trim().is_empty() {
                return Ok(true);
            }
            let kept = with_failures(scrollback, session, (from, to), text.trim())?;
            scrollback
                .keep_summary(session, from, to, &kept, at.now)
                .map(|()| true)
        }
        "curate" => {
            let Some(scrollback) = at.scrollback.as_ref() else {
                return Ok(false);
            };
            let Some(mut prompt) = scrollback.prompt(session)? else {
                return Ok(true);
            };
            if let Some(kept) = curated
                && Some(prompt.mark.as_str()) == job.context["mark"].as_str()
            {
                prompt.memory = Some(kept.clone());
                scrollback.keep_prompt(session, &prompt)?;
            }
            Ok(true)
        }
        _ => crate::notes::settle(at, session, job, text, keeping),
    }
}

/// Prepare a curated slot without changing its prompt or job.
fn curation(at: &mut Answering<'_>, job: &Job, text: &str, hooks: &mut dyn Hooks) -> Option<Value> {
    let said = json_in(text)?;
    let strings = |name: &str| -> Vec<String> {
        said[name]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    let (chosen, notes) = (strings("chosen"), strings("notes"));
    let query = job.context["query"].as_str().unwrap_or_default();
    let room = job.context["room"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(0);
    let per = hooks.window().estimate_chars_per_token;
    let found: Vec<_> = supply::candidates(at, query, CURATE_FROM)
        .into_iter()
        .filter(|hit| chosen.contains(&hit.memory.id.to_string()))
        .collect();
    let ids: Vec<String> = found.iter().map(|hit| hit.memory.id.to_string()).collect();
    // A rewrite stands in only for what is current truth: rewritten, a memory kept with doubt
    // would be offered without it, so those are packed as they are, under their own heading.
    let doubted = found
        .iter()
        .any(|hit| !hit.memory.is_assertable(at.inject_floor, at.now, true));
    let supplied = if notes.is_empty() || doubted {
        supply::pack(at, &found, room, per, hooks).unwrap_or_default()
    } else {
        supply::from_notes(&notes, ids, room, per).unwrap_or_default()
    };
    let mut kept = supplied.as_json(query);
    kept["curated"] = json!(true);
    Some(kept)
}
