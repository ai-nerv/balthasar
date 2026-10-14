//! `replay`, `scroll` and `resume`: how a run is read back.

use crate::Answering;
use balthasar_ipc::{MAX_FRAME, Reply, Request};
use balthasar_model::SessionId;

/// What the rows of one reply may come to: the frame cap, less room for the reply around them.
pub(crate) const ROOM: usize = MAX_FRAME - 64 * 1024;

/// The least a page of a raw record is cut to, whatever budget was asked for.
const RAW_PAGE_FLOOR: usize = 4 * 1024;

/// Room in a row for everything but the piece of a raw record it carries.
const RAW_SHELL: usize = 256;

/// Everything a run said, in order.
///
/// What a harness restores from. Turns come back as they finally stood. With `{from, bytes}` it
/// comes a page at a time; a row too large for any page comes back truncated and marked, with
/// `raw_bytes` saying how long its raw record is, and the next page starts after it. That record
/// is read back with `{from: cursor, raw: offset, bytes}`, a piece at a time.
pub fn replay(at: &mut Answering<'_>, request: &Request) -> Reply {
    let Some(session) = request.args.first().and_then(|v| v.as_str()) else {
        return Reply::refused("replay needs a session");
    };
    let session = SessionId::new(session);
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Reply::refused("this balthasar keeps no scrollback");
    };
    let paging = request.args.get(1);
    let from = paging.and_then(|p| p["from"].as_u64()).unwrap_or(0);
    let budget = paging
        .and_then(|p| p["bytes"].as_u64())
        .and_then(|b| usize::try_from(b).ok());
    if let Some(offset) = paging.and_then(|p| p.get("raw")) {
        let Some(offset) = offset.as_u64() else {
            return Reply::refused("raw is a byte offset: a whole number of zero or more");
        };
        return raw_page(scrollback, &session, from, offset, budget);
    }

    let limit = budget.map_or(ROOM, |b| b.min(ROOM));
    let (mut rows, mut spent, mut over) = (Vec::new(), 0, false);
    let read = scrollback.replay_while(&session, from, |turn| {
        let row = serde_json::json!(turn);
        let size = row.to_string().len();
        if budget.is_none() && spent + size > ROOM {
            over = true;
            return false;
        }
        if rows.is_empty() && size > ROOM {
            let mut cut = fitted(&turn, ROOM);
            if let Some(raw) = &turn.raw
                && let Some(fields) = cut.as_object_mut()
            {
                fields.insert("raw_bytes".to_owned(), serde_json::json!(raw.len()));
            }
            rows.push(cut);
            return false;
        }
        if !rows.is_empty() && spent + size > limit {
            return false;
        }
        spent += size;
        rows.push(row);
        true
    });
    match read {
        Ok(()) if over => Reply::failed(format!(
            "this run is over the {MAX_FRAME}-byte frame cap whole; ask for it a page at a time \
             with {{\"from\": 0, \"bytes\": …}}"
        )),
        Ok(()) => Reply::rows(rows),
        Err(why) => Reply::failed(why.to_string()),
    }
}

/// One piece of the raw record at `cursor`, from byte `offset`: `{cursor, raw, offset, next,
/// raw_bytes}`, with `next` absent once the record is done. Within `budget` when one is asked
/// for, never under [`RAW_PAGE_FLOOR`], and always within a frame once escaped.
fn raw_page(
    scrollback: &balthasar_store::Transcript,
    session: &SessionId,
    cursor: u64,
    offset: u64,
    budget: Option<usize>,
) -> Reply {
    let room = budget.map_or(ROOM, |b| b.clamp(RAW_PAGE_FLOOR, ROOM));
    let part = match scrollback.raw_part(session, cursor, offset, room - RAW_SHELL) {
        Ok(Some(part)) => part,
        Ok(None) => return Reply::refused(format!("turn {cursor} holds no raw record")),
        Err(why) => return Reply::refused(why.to_string()),
    };
    let piece = within(&part.text, room - RAW_SHELL);
    let end = offset + piece.len() as u64;
    let mut row = serde_json::json!({
        "cursor": cursor,
        "raw": piece,
        "offset": offset,
        "raw_bytes": part.total,
    });
    if end < part.total
        && let Some(fields) = row.as_object_mut()
    {
        fields.insert("next".to_owned(), serde_json::json!(end));
    }
    Reply::rows(vec![row])
}

/// The longest start of `text` that is at most `room` bytes once written as a JSON string, cut on
/// a character boundary.
fn within(text: &str, room: usize) -> &str {
    let mut keep = room.min(text.len());
    loop {
        let end = text.floor_char_boundary(keep);
        let kept = text.get(..end).unwrap_or_default();
        let size = serde_json::Value::String(kept.to_owned()).to_string().len();
        if size <= room || end == 0 {
            return kept;
        }
        keep = end.saturating_sub(size - room);
    }
}

/// A turn cut down to fit `room`: no `raw` and no `args`, its text shortened at a character
/// boundary, and `"truncated": true` so a reader can tell.
pub(crate) fn fitted(turn: &balthasar_store::Turn, room: usize) -> serde_json::Value {
    let mut cut = turn.clone();
    cut.raw = None;
    cut.args = None;
    let text = std::mem::take(&mut cut.text);
    let shell = serde_json::json!(cut).to_string().len() + 64;
    cut.text = within(&text, room.saturating_sub(shell)).to_owned();
    let mut row = serde_json::json!(cut);
    if let Some(fields) = row.as_object_mut() {
        fields.insert("truncated".to_owned(), serde_json::Value::Bool(true));
    }
    row
}

/// `scroll(session, {want, from, to, cursor, terms, tokens, turns})`.
///
/// Part of a scrollback, within a budget.
///
/// The reply carries what was left out and where to continue from.
pub fn scroll(at: &mut Answering<'_>, request: &Request) -> Reply {
    let Some(session) = request.args.first().and_then(|v| v.as_str()) else {
        return Reply::refused("scroll needs a session");
    };
    let session = SessionId::new(session);
    let said = request.args.get(1);
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Reply::refused("this balthasar keeps no scrollback");
    };

    let number = |name: &str| -> Option<u64> {
        said.and_then(|s| s.get(name))
            .and_then(serde_json::Value::as_u64)
    };
    let budget = balthasar_store::Budget {
        tokens: number("tokens").unwrap_or(4_000) as usize,
        turns: number("turns").unwrap_or(200) as usize,
    };

    let want = match said
        .and_then(|s| s.get("want"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("tail")
    {
        "tail" => balthasar_store::Want::Tail,
        "span" => match (number("from"), number("to")) {
            (Some(from), Some(to)) => balthasar_store::Want::Span { from, to },
            _ => return Reply::refused("a span needs `from` and `to`"),
        },
        "around" => match number("cursor") {
            Some(cursor) => balthasar_store::Want::Around { cursor },
            None => return Reply::refused("`around` needs a cursor"),
        },
        "matching" => balthasar_store::Want::Matching {
            terms: said
                .and_then(|s| s.get("terms"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
        },
        other => {
            return Reply::refused(format!(
                "no such read: '{other}' — try tail, span, around or matching"
            ));
        }
    };

    match scrollback.read(&session, &want, &budget) {
        Err(why) => Reply::refused(why.to_string()),
        Ok(held) => {
            // What a model reads back, inside a window it is trying to save: what was said,
            // and never the harness's own record of it, which is several times the size and
            // is `replay`'s to give.
            let turns: Vec<serde_json::Value> = held
                .turns
                .iter()
                .map(|turn| {
                    let mut row = serde_json::json!(turn);
                    if let Some(fields) = row.as_object_mut() {
                        fields.remove("raw");
                    }
                    if row.to_string().len() > ROOM {
                        return fitted(turn, ROOM);
                    }
                    row
                })
                .collect();
            // What was noticed across the same cursors, beside the turns and never among them.
            let traces = match (held.turns.first(), held.turns.last()) {
                (Some(first), Some(last)) => scrollback
                    .noticed(&session, first.cursor, last.cursor)
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            Reply::one(serde_json::json!({
                "turns": turns,
                "traces": traces,
                "tokens": held.tokens,
                "omitted": held.omitted,
                "next": held.next,
                "complete": held.is_complete(),
            }))
        }
    }
}

/// Where a restarting harness left off.
///
/// Guessing the next cursor wrong overwrites a turn that nothing else holds a copy of.
pub fn resume(at: &mut Answering<'_>, request: &Request) -> Reply {
    let Some(session) = request.args.first().and_then(|v| v.as_str()) else {
        return Reply::refused("resume needs a session");
    };
    let session = SessionId::new(session);
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Reply::refused("this balthasar keeps no scrollback");
    };
    let next = match scrollback.next_cursor(&session) {
        Ok(next) => next,
        Err(why) => return Reply::refused(why.to_string()),
    };
    let turns = scrollback.count(&session).unwrap_or(0);
    let run = match scrollback.run_of(&session) {
        Ok(run) => run,
        Err(why) => return Reply::refused(why.to_string()),
    };
    Reply::one(serde_json::json!({ "next": next, "turns": turns, "run": run.as_str() }))
}
