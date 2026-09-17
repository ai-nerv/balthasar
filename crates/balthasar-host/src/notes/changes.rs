//! Proposed note changes, application, and review decisions.

use super::{Keeping, authorization, provenance, ruling::echoes};
use balthasar_model::{SessionId, Timestamp};
use balthasar_store::{Change, Job, Note, StoreError, Transcript};
use serde_json::{Value, json};

pub(super) fn undo(
    scrollback: &Transcript,
    id: &str,
    now: Timestamp,
) -> Result<Result<String, String>, StoreError> {
    scrollback.atomic(|scrollback| {
        let Some(change) = scrollback.change(id)? else {
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
            at: now,
            ..Change::default()
        };
        let made = scrollback.record_change(&revert, None)?;
        apply(scrollback, &made, now)?;
        scrollback.set_change(id, "undone", None)?;
        Ok::<_, StoreError>(Ok(made))
    })
}

/// Log each op as a change, and apply it unless changes are reviewed first. An add whose title a
/// live note already has is an update of that note. Returns the change count and validity.
/// Runs inside the caller's job-completion transaction.
pub(super) fn propose(
    scrollback: &Transcript,
    session: &SessionId,
    job: &Job,
    ops: &[Value],
    root: &str,
    keeping: &Keeping,
    now: Timestamp,
) -> Result<(usize, bool), StoreError> {
    let sources = provenance::validated(scrollback, job)?;
    let mut made = 0;
    let mut accepted = true;
    for original in ops {
        let clean = super::scrubbed(original.clone(), root);
        let live = scrollback.notes()?;
        let target = original["id"]
            .as_str()
            .and_then(|id| live.iter().find(|n| n.id == id))
            .or_else(|| {
                let title = original["title"].as_str()?.trim();
                live.iter().find(|n| n.title.eq_ignore_ascii_case(title))
            })
            .or_else(|| {
                let title = clean["title"].as_str()?.trim();
                live.iter().find(|n| n.title.eq_ignore_ascii_case(title))
            });
        let op = if target.is_some_and(|n| n.pinned) || original["pinned"] == true {
            original
        } else {
            &clean
        };
        if ["title", "text", "description"].iter().any(|field| {
            op[*field]
                .as_str()
                .is_some_and(|value| value.trim().is_empty())
        }) {
            accepted = false;
            scrollback.record_change(
                &Change {
                    op: op["op"].as_str().unwrap_or("invalid").into(),
                    state: "rejected".into(),
                    by: job.id.clone(),
                    at: now,
                    reason: Some("note fields are empty after normalization".into()),
                    evidence: json!({"job":job.id,"requested":original}),
                    ..Change::default()
                },
                Some(session),
            )?;
            continue;
        }
        let invalid = authorization::selected(op, &sources).err();
        if invalid.is_none()
            && op["op"].as_str() == Some("add")
            && target.is_none()
            && echoes(op, &live)
        {
            continue;
        }
        let (note, before, mut after, kind) = match (op["op"].as_str(), target) {
            (Some("retire"), Some(n)) => (Some(n.id.clone()), Some(n.fields()), None, "retire"),
            (Some("add" | "update"), Some(n)) => {
                let Some(after) =
                    merged(op, Some(n)).filter(|a| invalid.is_some() || *a != n.fields())
                else {
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
            _ => {
                accepted = false;
                scrollback.record_change(
                    &Change {
                        op: op["op"].as_str().unwrap_or("invalid").into(),
                        state: "rejected".into(),
                        by: job.id.clone(),
                        at: now,
                        reason: Some("note target does not exist".into()),
                        ..Change::default()
                    },
                    Some(session),
                )?;
                continue;
            }
        };
        let evidence =
            authorization::authorize(job, op, target, after.as_ref(), &sources, &live, ops);
        let (evidence, reason) = match evidence {
            Ok(evidence) => (evidence, None),
            Err(reason) => (
                json!({"version":1,"job":job.id,"requested":op["evidence"]}),
                Some(reason.to_owned()),
            ),
        };
        if let Some(after) = after.as_mut() {
            after["evidence"] = evidence.clone();
        }
        let rejected = reason.is_some();
        let change = Change {
            note,
            op: kind.to_owned(),
            before,
            after,
            state: if rejected { "rejected" } else { "staged" }.to_owned(),
            by: job.id.clone(),
            at: now,
            evidence,
            reason,
            ..Change::default()
        };
        let id = scrollback.record_change(&change, Some(session))?;
        if rejected {
            accepted = false;
            continue;
        }
        if !keeping.review && !apply(scrollback, &id, now)? {
            accepted = false;
            continue;
        }
        made += 1;
    }
    Ok((made, accepted))
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
    let mut title = pick("title", base.map(|n| n.title.as_str()))?;
    if let Some(base) = base.filter(|base| base.title.eq_ignore_ascii_case(&title)) {
        title.clone_from(&base.title);
    }
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
    Some(
        json!({ "title": title, "text": text, "description": description, "pinned": pinned,
        "evidence":base.map(|n| n.evidence.clone()).unwrap_or(Value::Null) }),
    )
}

/// Make a logged change so.
fn apply(scrollback: &Transcript, id: &str, now: Timestamp) -> Result<bool, StoreError> {
    let Some(change) = scrollback.change(id)? else {
        return Ok(false);
    };
    if change.state != "staged" {
        return Ok(false);
    }
    if change.op != "revert" {
        let current = change
            .note
            .as_deref()
            .map(|id| scrollback.note(id))
            .transpose()?
            .flatten()
            .filter(|note| note.retired.is_none());
        let reason = if current.as_ref().map(Note::fields) != change.before {
            Some("target note changed before application")
        } else if change.evidence["version"] != 1
            || !provenance::current(scrollback, &change.evidence)?
        {
            Some("source evidence is missing or changed before application")
        } else if !authorization::supports_change(scrollback, &change, current.as_ref())? {
            Some("source evidence no longer supports the rule operation")
        } else {
            None
        };
        if let Some(reason) = reason {
            scrollback.reject_change(id, reason)?;
            return Ok(false);
        }
    }
    let note = scrollback.put_note(change.note.as_deref(), change.after.as_ref(), now)?;
    scrollback.set_change(id, "applied", note.as_deref())?;
    Ok(true)
}

/// Approve or reject staged changes inside the caller's transaction. Answers which ones were.
pub(super) fn decide(
    scrollback: &Transcript,
    ids: &[String],
    approve: bool,
    now: Timestamp,
) -> Result<Vec<String>, StoreError> {
    let mut done = Vec::new();
    for id in ids {
        let Some(_) = scrollback.change(id)?.filter(|c| c.state == "staged") else {
            continue;
        };
        if approve {
            if !apply(scrollback, id, now)? {
                continue;
            }
        } else {
            scrollback.set_change(id, "rejected", None)?;
        }
        done.push(id.clone());
    }
    Ok(done)
}
