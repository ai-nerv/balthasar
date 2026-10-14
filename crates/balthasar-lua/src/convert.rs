//! Between a Lua value and JSON.
//!
//! Conversion happens once, at the boundary, because luna's `Value<'gc>` cannot leave
//! `lua.enter`. Everything above this crate sees `serde_json::Value`.
//!
//! The list-or-map rule is the family's: a table whose keys are exactly `1..n` is an array and
//! anything else is an object. Lua to JSON is one walk, and it refuses a table that holds itself,
//! nesting past [`DEEPEST`], more than [`NODES`] values and more than [`BYTES`] of text.

use luna::{Table, Value};
use serde_json::{Map, Number};
use std::collections::HashSet;

/// How many tables deep one conversion may nest.
pub const DEEPEST: usize = 32;

/// The most values one conversion may visit, keys included. A policy's answer laying out 20,000
/// rows is 100,000; a source's answer for the longest journal line on hand is 25.
pub const NODES: usize = 1 << 21;

/// The most bytes of text one conversion may copy, keys included: four times the family's 16 MiB
/// frame cap, the most any one call or reply carries.
pub const BYTES: usize = 64 << 20;

/// Why a Lua value has no JSON form.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Undescribable {
    /// A function, thread or userdata, at the place named.
    #[error("{0} is code, not data")]
    Code(String),
    #[error("a table key is neither a string nor an integer")]
    Key,
    #[error("a table holds itself")]
    Cycle,
    #[error("tables nest deeper than {DEEPEST}")]
    Deep,
    #[error("more than {NODES} values")]
    Wide,
    #[error("more than {} MiB of text", BYTES >> 20)]
    Long,
}

/// A Lua value as JSON, `at` naming it. A field holding code is left out, so a spec may carry its
/// callbacks beside its data; code anywhere else is refused.
///
/// # Errors
/// What makes the value undescribable.
pub fn to_json<'gc>(
    ctx: luna::Context<'gc>,
    value: Value<'gc>,
    at: &str,
) -> Result<serde_json::Value, Undescribable> {
    Walk::new(ctx, at, false).value(value)
}

/// A Lua value as JSON, `at` naming it, refusing code wherever it is by the place it was found.
///
/// # Errors
/// What makes the value undescribable; for code, the first place holding some.
pub fn data_to_json<'gc>(
    ctx: luna::Context<'gc>,
    value: Value<'gc>,
    at: &str,
) -> Result<serde_json::Value, Undescribable> {
    Walk::new(ctx, at, true).value(value)
}

/// One conversion: what it has spent, the tables it is inside, and the keys that led here.
struct Walk<'gc, 'a> {
    ctx: luna::Context<'gc>,
    at: &'a str,
    /// Refuse a field holding code, rather than leave it out.
    strict: bool,
    nodes: usize,
    bytes: usize,
    open: HashSet<Table<'gc>>,
    path: Vec<Value<'gc>>,
}

impl<'gc, 'a> Walk<'gc, 'a> {
    fn new(ctx: luna::Context<'gc>, at: &'a str, strict: bool) -> Self {
        Self {
            ctx,
            at,
            strict,
            nodes: 0,
            bytes: 0,
            open: HashSet::new(),
            path: Vec::new(),
        }
    }

    fn spend(&mut self, bytes: usize) -> Result<(), Undescribable> {
        self.nodes += 1;
        self.bytes = self.bytes.saturating_add(bytes);
        if self.nodes > NODES {
            return Err(Undescribable::Wide);
        }
        if self.bytes > BYTES {
            return Err(Undescribable::Long);
        }
        Ok(())
    }

    fn text(&mut self, text: luna::String<'gc>) -> Result<String, Undescribable> {
        self.spend(text.as_bytes().len())?;
        Ok(String::from_utf8_lossy(text.as_bytes()).into_owned())
    }

    /// Where the walk is, as `at.key.key`.
    fn place(&self) -> String {
        let mut place = self.at.to_owned();
        for key in &self.path {
            place.push('.');
            match key {
                Value::String(s) => place.push_str(&String::from_utf8_lossy(s.as_bytes())),
                Value::Integer(i) => place.push_str(&i.to_string()),
                _ => place.push('?'),
            }
        }
        place
    }

    fn value(&mut self, value: Value<'gc>) -> Result<serde_json::Value, Undescribable> {
        Ok(match value {
            Value::String(s) => serde_json::Value::String(self.text(s)?),
            Value::Table(t) => {
                self.spend(0)?;
                self.table(t)?
            }
            other => {
                self.spend(0)?;
                match other {
                    Value::Nil => serde_json::Value::Null,
                    Value::Boolean(b) => serde_json::Value::Bool(b),
                    Value::Integer(i) => serde_json::Value::Number(i.into()),
                    Value::Number(f) => {
                        Number::from_f64(f).map_or(serde_json::Value::Null, Into::into)
                    }
                    _ => return Err(Undescribable::Code(self.place())),
                }
            }
        })
    }

    fn table(&mut self, table: Table<'gc>) -> Result<serde_json::Value, Undescribable> {
        if self.open.len() >= DEEPEST {
            return Err(Undescribable::Deep);
        }
        if !self.open.insert(table) {
            return Err(Undescribable::Cycle);
        }
        // One past what the budget has left, so a table wider than that is refused unread.
        let room = NODES.saturating_sub(self.nodes) + 1;
        let entries: Vec<(Value<'gc>, Value<'gc>)> = table.iter(self.ctx).take(room).collect();
        let out = self.entries(entries);
        self.open.remove(&table);
        out
    }

    fn entries(
        &mut self,
        entries: Vec<(Value<'gc>, Value<'gc>)>,
    ) -> Result<serde_json::Value, Undescribable> {
        let is_list = !entries.is_empty()
            && entries.iter().enumerate().all(
                |(index, (key, _))| matches!(key, Value::Integer(i) if *i == index as i64 + 1),
            );

        if is_list {
            let mut out = Vec::with_capacity(entries.len());
            for (key, value) in entries {
                out.push(self.under(key, value)?);
            }
            return Ok(serde_json::Value::Array(out));
        }

        let mut out = Map::new();
        for (key, value) in entries {
            let name = match key {
                Value::String(s) => self.text(s)?,
                Value::Integer(i) => {
                    self.spend(0)?;
                    i.to_string()
                }
                _ => return Err(Undescribable::Key),
            };
            let code = matches!(
                value,
                Value::Function(_) | Value::Thread(_) | Value::UserData(_)
            );
            if code && !self.strict {
                continue;
            }
            out.insert(name, self.under(key, value)?);
        }
        Ok(serde_json::Value::Object(out))
    }

    /// `value`, walked with `key` on the path.
    fn under(
        &mut self,
        key: Value<'gc>,
        value: Value<'gc>,
    ) -> Result<serde_json::Value, Undescribable> {
        self.path.push(key);
        let out = self.value(value);
        self.path.pop();
        out
    }
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

#[cfg(test)]
mod tests {
    use super::{BYTES, DEEPEST, NODES};
    use crate::{Engine, LuaError};

    /// What `balthasar.json.encode` said of the `value` that `body` built: the text, or why not.
    fn encoded(body: &str) -> String {
        let mut engine = Engine::new();
        engine
            .run(
                &format!(
                    "{body}\nlocal _, said = pcall(balthasar.json.encode, value)\n\
                     balthasar.answer = said"
                ),
                "encode.lua",
            )
            .expect("run");
        engine.harvest();
        engine
            .config()
            .string("answer")
            .unwrap_or("<absent>")
            .to_owned()
    }

    /// What reading a project file that assigns `balthasar.weights` from `body` says.
    fn project(body: &str) -> String {
        let dir = balthasar_model::scratch::Scratch::new("balthasar-convert", "project");
        let file = dir.join(".balthasar.lua");
        std::fs::write(&file, body).expect("write");
        match Engine::new().read(&[(file, false)]) {
            Err(LuaError::Untrusted { what, .. }) => what,
            other => format!("{other:?}"),
        }
    }

    /// A table nested `depth` deep, as Lua.
    fn nested(depth: usize) -> String {
        format!("local value = {{}} for _ = 2, {depth} do value = {{ value }} end")
    }

    #[test]
    fn a_value_that_holds_itself_is_refused_by_name() {
        let said = encoded("local value = { a = { b = {} } }; value.a.b.c = value");
        assert!(said.ends_with("a table holds itself"), "{said}");
        let said = project("local t = { x = 1 }; t.t = t; balthasar.weights = t\n");
        assert_eq!(said, "balthasar.weights (a table holds itself)");

        let mut engine = Engine::new();
        engine
            .run(
                "balthasar.on.scope(function() local t = {}; t.t = t; return t end)",
                "init.lua",
            )
            .expect("registers");
        assert_eq!(engine.ask("scope", &[serde_json::json!("/w")]), None);
        let log = engine.config().log;
        assert!(log.iter().any(|l| l.contains("holds itself")), "{log:?}");
    }

    #[test]
    fn a_table_shared_twice_is_written_twice_and_is_not_a_cycle() {
        assert_eq!(
            encoded("local s = { 1 }; local value = { s, { s } }"),
            "[[1],[[1]]]"
        );
    }

    #[test]
    fn a_table_wider_than_the_budget_is_refused() {
        let said = encoded(&format!(
            "local value = {{}}; for at = 1, {} do value[at] = true end",
            NODES + 1
        ));
        assert!(
            said.ends_with(&format!("more than {NODES} values")),
            "{said}"
        );
    }

    #[test]
    fn sharing_that_multiplies_past_the_node_budget_is_refused_without_walking_it_all() {
        let doubled = "local value = {} for _ = 1, 30 do value = { value, value } end";
        let said = encoded(doubled);
        assert!(
            said.ends_with(&format!("more than {NODES} values")),
            "{said}"
        );
        let said = project(&format!("{doubled}\nbalthasar.weights = value\n"));
        assert_eq!(
            said,
            format!("balthasar.weights (more than {NODES} values)")
        );
    }

    #[test]
    fn text_past_the_byte_budget_is_refused() {
        let said = encoded(&format!(
            "local piece = string.rep('x', 1 << 20); local value = {{}}\n\
             for at = 1, {} do value[at] = piece end",
            (BYTES >> 20) + 1
        ));
        assert!(
            said.ends_with(&format!("more than {} MiB of text", BYTES >> 20)),
            "{said}"
        );
    }

    #[test]
    fn nesting_past_the_deepest_is_refused_and_up_to_it_is_written() {
        let said = encoded(&nested(DEEPEST));
        assert!(said.starts_with(&"[".repeat(DEEPEST - 1)), "{said}");
        let said = encoded(&nested(DEEPEST + 1));
        assert!(said.ends_with(&format!("deeper than {DEEPEST}")), "{said}");
    }

    #[test]
    fn code_in_a_project_value_is_named_by_where_it_was_found() {
        let said = project("balthasar.weights = { a = { 1, function() end } }\n");
        assert_eq!(said, "balthasar.weights.a.2");
    }
}
