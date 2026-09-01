//! Between a Lua value and JSON.
//!
//! Conversion happens once, at the boundary, because luna's `Value<'gc>` cannot leave
//! `lua.enter`. Everything above this crate sees `serde_json::Value`.
//!
//! The list-or-map rule is the family's: a table whose keys are exactly `1..n` is an array and
//! anything else is an object. Disagreeing with it would have a config and a socket describe
//! one table two different ways.

use luna::{Table, Value};
use serde_json::{Map, Number};

/// How deep a table may nest before it is refused.
///
/// A config is input, and a cyclic table is a stack overflow rather than an error if this
/// recurses freely.
const MAX_DEPTH: usize = 32;

/// A Lua value as JSON, or `None` when it cannot be written down.
#[must_use]
pub fn to_json<'gc>(
    ctx: luna::Context<'gc>,
    value: Value<'gc>,
    depth: usize,
) -> Option<serde_json::Value> {
    if depth > MAX_DEPTH {
        return None;
    }
    Some(match value {
        Value::Nil => serde_json::Value::Null,
        Value::Boolean(b) => serde_json::Value::Bool(b),
        Value::Integer(i) => serde_json::Value::Number(i.into()),
        Value::Number(f) => Number::from_f64(f).map_or(serde_json::Value::Null, Into::into),
        Value::String(s) => serde_json::Value::String(String::from_utf8_lossy(s.as_bytes()).into()),
        Value::Table(t) => table_to_json(ctx, t, depth)?,
        // A function belongs to the VM that made it. Registrars that carry one keep it there.
        _ => return None,
    })
}

fn table_to_json<'gc>(
    ctx: luna::Context<'gc>,
    table: Table<'gc>,
    depth: usize,
) -> Option<serde_json::Value> {
    let entries: Vec<(Value<'gc>, Value<'gc>)> = table.iter(ctx).collect();

    let is_list = !entries.is_empty()
        && entries
            .iter()
            .enumerate()
            .all(|(index, (key, _))| matches!(key, Value::Integer(i) if *i == index as i64 + 1));

    if is_list {
        let mut out = Vec::with_capacity(entries.len());
        for (_, value) in entries {
            out.push(to_json(ctx, value, depth + 1)?);
        }
        return Some(serde_json::Value::Array(out));
    }

    let mut out = Map::new();
    for (key, value) in entries {
        let name = match key {
            Value::String(s) => String::from_utf8_lossy(s.as_bytes()).into_owned(),
            Value::Integer(i) => i.to_string(),
            _ => return None,
        };
        // A field holding a function is skipped rather than refusing the whole table: a spec
        // may legitimately mix data with a callback the VM keeps.
        if let Some(json) = to_json(ctx, value, depth + 1) {
            out.insert(name, json);
        }
    }
    Some(serde_json::Value::Object(out))
}

/// A JSON value as Lua.
#[must_use]
pub fn from_json<'gc>(ctx: luna::Context<'gc>, value: &serde_json::Value) -> Value<'gc> {
    match value {
        serde_json::Value::Null => Value::Nil,
        serde_json::Value::Bool(b) => Value::Boolean(*b),
        serde_json::Value::Number(n) => n
            .as_i64()
            .map_or_else(|| Value::Number(n.as_f64().unwrap_or(0.0)), Value::Integer),
        serde_json::Value::String(s) => Value::String(luna::String::from_slice(&ctx, s.as_bytes())),
        serde_json::Value::Array(items) => {
            let table = Table::new(&ctx);
            for (index, item) in items.iter().enumerate() {
                table.set(ctx, index as i64 + 1, from_json(ctx, item)).ok();
            }
            Value::Table(table)
        }
        serde_json::Value::Object(fields) => {
            let table = Table::new(&ctx);
            for (key, item) in fields {
                let name = luna::String::from_slice(&ctx, key.as_bytes());
                table.set(ctx, name, from_json(ctx, item)).ok();
            }
            Value::Table(table)
        }
    }
}

/// Raise a message into Lua, so a `pcall` in a config sees a string.
pub fn raise<'gc>(ctx: luna::Context<'gc>, message: &str) -> luna::Error<'gc> {
    luna::Error::from_value(Value::String(luna::String::from_slice(
        &ctx,
        message.as_bytes(),
    )))
}
