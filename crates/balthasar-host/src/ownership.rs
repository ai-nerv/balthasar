//! Connection-owned transcript writers and transient mutation fences.

use crate::{Answering, Door};
use balthasar_ipc::{Reply, Request};
use balthasar_model::SessionId;
use balthasar_store::{StoreError, Writer};

/// At most one persistent transcript writer per authenticated connection.
#[derive(Default)]
pub struct Ownership {
    held: Option<(SessionId, Writer)>,
}

impl balthasar_ipc::ConnectionState for Ownership {
    fn retains_connection(&self) -> bool {
        self.held.is_some()
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    revision: u32,
    operation: Option<String>,
}

fn asked(request: &Request) -> Result<(SessionId, Options), String> {
    let [key, options] = request.args.as_slice() else {
        return Err("writer needs (transcript, options)".into());
    };
    let key = key
        .as_str()
        .filter(|key| !key.is_empty() && key.len() <= 1024)
        .ok_or("writer transcript key must contain 1..1024 bytes")?;
    let options: Options = serde_json::from_value(options.clone())
        .map_err(|_| "writer options must contain revision and an optional operation")?;
    if options.revision != 1 {
        return Err("unsupported writer revision".into());
    }
    Ok((SessionId::new(key), options))
}

pub(crate) fn special(
    at: &Answering<'_>,
    door: &Door,
    request: &Request,
    state: Option<&mut Ownership>,
) -> Option<Reply> {
    if !matches!(request.call.as_str(), "writer" | "writer_status") {
        return None;
    }
    Some(control(at, door, request, state).unwrap_or_else(Reply::refused))
}

fn control(
    at: &Answering<'_>,
    door: &Door,
    request: &Request,
    state: Option<&mut Ownership>,
) -> Result<Reply, String> {
    let (key, options) = asked(request)?;
    let persistent = at
        .scrollback
        .as_ref()
        .is_some_and(|store| store.is_persistent());
    let supported = persistent && state.is_some() && matches!(door, Door::Socket(_));
    if request.call == "writer_status" {
        if options.operation.is_some() {
            return Err("writer_status takes no operation".into());
        }
        let owned = if let Some((held_key, writer)) =
            state.as_ref().and_then(|state| state.held.as_ref())
        {
            at.scrollback
                .as_ref()
                .ok_or("no transcript store")?
                .check_writer(writer, held_key)
                .map_err(|why| why.to_string())?;
            held_key == &key
        } else {
            false
        };
        return Ok(Reply::one(serde_json::json!({
            "revision": 1, "supported": supported, "owned": owned,
            "lifetime": "connection", "scope": "store-transcript"
        })));
    }
    if !supported {
        return Err("writer reservations require a persistent socket connection and store".into());
    }
    let state = state.ok_or("no connection state")?;
    let store = at.scrollback.as_ref().ok_or("no transcript store")?;
    match options.operation.as_deref() {
        Some("reserve") => {
            if let Some((held_key, writer)) = &state.held {
                if held_key != &key {
                    return Err("this connection already owns another transcript".into());
                }
                store
                    .check_writer(writer, &key)
                    .map_err(|why| why.to_string())?;
            } else {
                let writer = store
                    .reserve_writer(&key)
                    .map_err(|why| why.to_string())?
                    .ok_or("this transcript already has a writer")?;
                state.held = Some((key, writer));
            }
            Ok(Reply::one(
                serde_json::json!({ "revision": 1, "owned": true }),
            ))
        }
        Some("release") => {
            let (held_key, writer) = state.held.as_ref().ok_or("this connection has no writer")?;
            if held_key != &key {
                return Err("this connection owns a different transcript".into());
            }
            store
                .check_writer(writer, &key)
                .map_err(|why| why.to_string())?;
            state.held = None;
            Ok(Reply::one(
                serde_json::json!({ "revision": 1, "owned": false }),
            ))
        }
        _ => Err("writer operation must be reserve or release".into()),
    }
}

pub(crate) fn fence(
    at: &Answering<'_>,
    request: &Request,
    state: Option<&Ownership>,
) -> Result<Option<balthasar_store::Mutation>, StoreError> {
    let mutating = matches!(
        request.call.as_str(),
        "observe" | "amend" | "model" | "layout" | "applied" | "overflowed" | "jobs" | "job_done"
    );
    let reading = matches!(request.call.as_str(), "replay" | "resume" | "scroll");
    if !mutating && !reading {
        return Ok(None);
    }
    let (Some(store), Some(key)) = (
        at.scrollback.as_ref(),
        request.args.first().and_then(|key| key.as_str()),
    ) else {
        return Ok(None);
    };
    let key = SessionId::new(key);
    if let Some((held_key, writer)) = state.and_then(|state| state.held.as_ref()) {
        if held_key == &key {
            store.check_writer(writer, &key)?;
            return Ok(None);
        }
        if mutating {
            return Err(StoreError::Foreign(
                "this connection owns a different transcript".into(),
            ));
        }
    }
    if !mutating || !store.is_persistent() {
        return Ok(None);
    }
    store
        .reserve_mutation(&key)?
        .map(Some)
        .ok_or_else(|| StoreError::Foreign("this transcript already has a writer".into()))
}
