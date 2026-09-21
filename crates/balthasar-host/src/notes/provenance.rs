//! Typed, immutable transcript evidence retained with extraction jobs.

use balthasar_store::{Budget, Job, StoreError, Transcript, Turn, Want};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn fingerprint(turn: &Turn) -> String {
    let identity = json!([turn.cursor, turn.role, turn.kind, turn.tool, turn.text]);
    format!("{:x}", Sha256::digest(identity.to_string().as_bytes()))
}

fn eligible(turn: &Turn) -> bool {
    turn.role == "user" && turn.kind != "from" && turn.tool.is_none() && !turn.is_tool()
}

pub(super) fn intact(scrollback: &Transcript, job: &Job) -> Result<bool, StoreError> {
    let (Some(from), Some(to), Some(sources)) = (
        job.context["from"].as_u64(),
        job.context["to"].as_u64(),
        job.context["provenance"]["sources"].as_array(),
    ) else {
        return Ok(false);
    };
    let rows = scrollback.read(
        &job.session,
        &Want::Span { from, to },
        &Budget {
            tokens: usize::MAX,
            turns: usize::MAX,
        },
    )?;
    let rows: Vec<_> = rows
        .turns
        .iter()
        .filter(|r| (from..=to).contains(&r.cursor))
        .collect();
    Ok(rows.len() == sources.len()
        && rows.iter().zip(sources).all(|(turn, source)| {
            source["cursor"] == turn.cursor
                && source["text"] == turn.text
                && source["fingerprint"].as_str() == Some(fingerprint(turn).as_str())
        }))
}

pub(super) fn source(turn: &Turn, limit: usize) -> Value {
    let mut text: String = if turn.stub.is_some() && turn.text.len() > limit {
        String::new()
    } else {
        turn.text.chars().take(limit).collect()
    };
    if text.len() < turn.text.len() {
        text.truncate(text.rfind('\n').map_or(0, |end| end + 1));
    }
    json!({ "cursor": turn.cursor, "role": turn.role, "kind": turn.kind,
        "tool": turn.tool, "text": text, "fingerprint": fingerprint(turn) })
}

pub(super) fn validated(scrollback: &Transcript, job: &Job) -> Result<Vec<Value>, StoreError> {
    let mut valid = Vec::new();
    let provenance = &job.context["provenance"];
    if provenance["version"] != 1 {
        balthasar_model::noted!("note: job {} has no supported source provenance", job.id);
        return Ok(valid);
    }
    let Some(sources) = provenance["sources"].as_array() else {
        return Ok(valid);
    };
    let (Some(from), Some(to)) = (job.context["from"].as_u64(), job.context["to"].as_u64()) else {
        return Ok(valid);
    };
    let budget = Budget {
        tokens: usize::MAX,
        turns: usize::MAX,
    };
    for source in sources {
        let Some(cursor) = source["cursor"]
            .as_u64()
            .filter(|cursor| (from..=to).contains(cursor))
        else {
            continue;
        };
        let rows = scrollback.read(
            &job.session,
            &Want::Span {
                from: cursor,
                to: cursor,
            },
            &budget,
        )?;
        let Some(turn) = rows.turns.iter().find(|turn| turn.cursor == cursor) else {
            continue;
        };
        if source["fingerprint"].as_str() != Some(fingerprint(turn).as_str()) {
            balthasar_model::noted!(
                "note: job {} source {} changed after extraction was queued",
                job.id,
                cursor
            );
            continue;
        }
        let captured = source["text"].as_str().unwrap_or_default();
        if !captured.is_empty() && turn.text.starts_with(captured) {
            let mut source = source.clone();
            source["session"] = json!(job.session.as_str());
            source["eligible"] = json!(eligible(turn));
            valid.push(source);
        }
    }
    Ok(valid)
}

pub(super) fn current(scrollback: &Transcript, evidence: &Value) -> Result<bool, StoreError> {
    if evidence["kind"] == "duplicate" {
        let Some(id) = evidence["retained"]["id"].as_str() else {
            return Ok(false);
        };
        if !scrollback.note(id)?.is_some_and(|n| {
            n.pinned && n.retired.is_none() && n.fields() == evidence["retained"]["fields"]
        }) {
            return Ok(false);
        }
    }
    Ok(restored(scrollback, evidence)?.is_some())
}

/// Reload saved references from unchanged original rows, recomputing their typed authority.
pub(super) fn restored(
    scrollback: &Transcript,
    evidence: &Value,
) -> Result<Option<Vec<Value>>, StoreError> {
    if evidence["version"] != 1 {
        return Ok(None);
    }
    let Some(sources) = evidence["sources"].as_array() else {
        return Ok(None);
    };
    let mut restored = Vec::new();
    for saved in sources {
        let (Some(session), Some(cursor), Some(digest)) = (
            saved["session"].as_str(),
            saved["cursor"].as_u64(),
            saved["fingerprint"].as_str(),
        ) else {
            return Ok(None);
        };
        let Some(turn) = scrollback.at(&balthasar_model::SessionId::new(session), cursor)? else {
            return Ok(None);
        };
        if fingerprint(&turn) != digest {
            return Ok(None);
        }
        let mut row = source(&turn, usize::MAX);
        row["session"] = json!(session);
        row["eligible"] = json!(eligible(&turn));
        if let Some(quote) = saved.get("quote") {
            let Some(quote) = quote
                .as_str()
                .filter(|q| !q.is_empty() && turn.text.contains(q))
            else {
                return Ok(None);
            };
            row["quote"] = json!(quote);
        }
        restored.push(row);
    }
    Ok(Some(restored))
}
