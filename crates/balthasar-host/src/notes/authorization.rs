//! Evidence validation for each proposed note operation.

use super::ruling::{canonical, instruction, rule_text, same_rule, unquoted};
use balthasar_store::{Change, Job, Note, StoreError, Transcript};
use serde_json::{Value, json};

pub(super) fn selected(op: &Value, sources: &[Value]) -> Result<Vec<Value>, &'static str> {
    let Some(requested) = op.get("evidence") else {
        return Ok(sources.to_vec());
    };
    let references = requested
        .as_array()
        .filter(|a| !a.is_empty() && a.len() <= 32)
        .ok_or("evidence must contain one to 32 cursor/quote references")?;
    references
        .iter()
        .map(|reference| {
            let cursor = reference["cursor"]
                .as_u64()
                .ok_or("evidence cursor is missing")?;
            let quote = reference["quote"]
                .as_str()
                .filter(|q| !q.is_empty())
                .ok_or("evidence quote is missing")?;
            let source = sources
                .iter()
                .find(|s| s["cursor"] == cursor)
                .ok_or("evidence source is missing, changed, or outside this extraction")?;
            let text = source["text"].as_str().unwrap_or_default();
            let quote = if text.contains(quote) {
                quote
            } else {
                unlabelled(quote, cursor)
            };
            if !text.contains(quote) {
                return Err("evidence quote is not in the original source");
            }
            let mut source = source.clone();
            source["quote"] = json!(quote);
            Ok(source)
        })
        .collect()
}

/// A quote without the label the row was shown under, which is this program's and not the
/// source's: a model copies `[4] person: ` along with the words as readily as the words alone.
fn unlabelled(quote: &str, cursor: u64) -> &str {
    let bare = quote.trim_start();
    let bare = bare.strip_prefix(&format!("[{cursor}] ")).unwrap_or(bare);
    bare.strip_prefix("person: ").unwrap_or(bare)
}

fn matching(sources: &[Value], accepts: impl Fn(&str) -> bool) -> Option<Value> {
    for source in sources.iter().filter(|s| s["eligible"] == true) {
        for line in unquoted(source["text"].as_str().unwrap_or_default()) {
            if source["quote"].as_str().is_some_and(|q| {
                let q = q.trim().strip_prefix("- ").unwrap_or(q.trim());
                q != line && Some(q) != rule_text(line)
            }) {
                continue;
            }
            if accepts(line) {
                let mut found = source.clone();
                if found["quote"].is_null() {
                    found["quote"] = json!(line);
                }
                found.as_object_mut()?.remove("text");
                return Some(found);
            }
        }
    }
    None
}

fn recorded(job: &Job, kind: &str, sources: Vec<Value>) -> Value {
    let sources: Vec<_> = sources
        .into_iter()
        .map(|mut source| {
            if let Some(fields) = source.as_object_mut() {
                fields.remove("text");
            }
            source
        })
        .collect();
    json!({"version":1, "job":job.id, "kind":kind, "sources":sources})
}

pub(super) fn supports_rule(scrollback: &Transcript, note: &Note) -> Result<bool, StoreError> {
    if !note.pinned {
        return Ok(false);
    }
    let Some(sources) = super::provenance::restored(scrollback, &note.evidence)? else {
        return Ok(false);
    };
    Ok(match note.evidence["kind"].as_str() {
        Some("user-rule") => standing(&sources, &note.text).is_some(),
        Some("user-rule-change") => changing(&sources, note, Some(&note.fields())).is_some(),
        _ => false,
    })
}

pub(super) fn supports_change(
    scrollback: &Transcript,
    change: &Change,
    before: Option<&Note>,
) -> Result<bool, StoreError> {
    let after = change.after.as_ref();
    let was_pinned = before.is_some_and(|n| n.pinned);
    let pinned = after.is_some_and(|a| a["pinned"] == true);
    if !was_pinned && !pinned {
        return Ok(true);
    }
    if was_pinned && after.is_none() && change.evidence["kind"] == "duplicate" {
        return Ok(before.is_some_and(|n| {
            same_rule(
                &n.text,
                change.evidence["retained"]["fields"]["text"]
                    .as_str()
                    .unwrap_or_default(),
            )
        }));
    }
    let Some(sources) = super::provenance::restored(scrollback, &change.evidence)? else {
        return Ok(false);
    };
    Ok(if let Some(note) = before.filter(|n| n.pinned) {
        change.evidence["kind"] == "user-rule-change" && changing(&sources, note, after).is_some()
    } else {
        change.evidence["kind"] == "user-rule"
            && after
                .and_then(|a| a["text"].as_str())
                .is_some_and(|text| standing(&sources, text).is_some())
    })
}

fn standing(sources: &[Value], text: &str) -> Option<Value> {
    matching(sources, |line| {
        rule_text(line).is_some_and(|rule| same_rule(rule, text))
    })
}

fn changing(sources: &[Value], note: &Note, after: Option<&Value>) -> Option<Value> {
    if let Some(after) = after {
        if after["pinned"] != true {
            let unchanged = ["title", "text", "description"]
                .iter()
                .all(|field| after[field] == note.fields()[field]);
            unchanged
                .then(|| {
                    matching(sources, |line| {
                        canonical(line) == format!("unpin note {}", note.id.to_lowercase())
                    })
                })
                .flatten()
        } else {
            matching(sources, |line| {
                let Some((name, body)) = line.split_once(':') else {
                    return false;
                };
                (canonical(name) == canonical(&note.title)
                    || canonical(name) == canonical(&note.id))
                    && same_rule(body, after["text"].as_str().unwrap_or_default())
                    && !body.trim().is_empty()
                    && after["title"] == note.title
                    && (after["description"] == note.description
                        || after["description"] == after["text"])
            })
        }
    } else {
        matching(sources, |line| {
            canonical(line) == format!("retire note {}", note.id.to_lowercase())
        })
    }
}

pub(super) fn authorize(
    job: &Job,
    op: &Value,
    base: Option<&Note>,
    after: Option<&Value>,
    sources: &[Value],
    kept: &[Note],
    operations: &[Value],
) -> Result<Value, &'static str> {
    let sources = selected(op, sources)?;
    let was_pinned = base.is_some_and(|n| n.pinned);
    let pinned = after.is_some_and(|a| a["pinned"] == true);
    if was_pinned {
        let note = base.ok_or("pinned note is missing")?;
        if job.kind == "tidy" && after.is_none() {
            let retained = kept
                .iter()
                .find(|n| {
                    Some(n.id.as_str()) == op["duplicate_of"].as_str()
                        && n.id != note.id
                        && n.pinned
                        && same_rule(&n.text, &note.text)
                })
                .ok_or("retiring a pinned duplicate requires an identical retained pinned note")?;
            if operations.iter().any(|other| {
                other["id"] == retained.id
                    || other["title"]
                        .as_str()
                        .is_some_and(|t| t.eq_ignore_ascii_case(&retained.title))
            }) {
                return Err("the retained duplicate cannot also be changed in this batch");
            }
            return Ok(
                json!({"version":1,"job":job.id,"kind":"duplicate","sources":[],
                "retained":{"id":retained.id,"fields":retained.fields()}}),
            );
        }
        let found = changing(&sources, note, after);
        return found
            .map(|source| recorded(job, "user-rule-change", vec![source]))
            .ok_or("changing a pinned note requires an unquoted user directive naming that note");
    }
    if pinned {
        let text = after
            .and_then(|a| a["text"].as_str())
            .ok_or("pinned text is missing")?;
        return standing(&sources, text)
            .map(|source| recorded(job, "user-rule", vec![source]))
            .ok_or("pinned text is not a complete eligible user rule in this extraction");
    }
    if let Some(after) = after
        && ["title", "text", "description"]
            .iter()
            .any(|field| after[field].as_str().is_some_and(instruction))
    {
        return Err("an unpinned observation cannot introduce an instruction");
    }
    Ok(recorded(
        job,
        if job.kind == "tidy" {
            "tidy-observation"
        } else {
            "observation"
        },
        sources,
    ))
}
