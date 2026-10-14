//! The VM a project's own file runs in: data, not a program.
//!
//! A file that arrived with a checkout gets no socket, no filesystem, no environment, no `load`
//! and no hook. It may assign [`crate::engine::PROJECT_SETTINGS`] and add sections. Whatever else
//! it reaches for is recorded by name as it happens, so a `pcall` cannot hide it, and the
//! refusal is fatal once the file has run.

use crate::config::Config;
use crate::engine::PROJECT_SETTINGS;
use crate::{convert, handler};
use luna::{Callback, CallbackReturn, Lua, Table, Value};
use std::cell::RefCell;
use std::rc::Rc;

/// Names a project file reached for and was refused.
pub type Refused = Rc<RefCell<Vec<String>>>;

/// Where the prelude finds the recorder, before it hides it.
const RECORDER: &str = "__balthasar_refuse";

/// Fields of the project module that are tools rather than settings.
pub const LENT: &[&str] = &[
    "json",
    "text",
    "path",
    "log",
    "looks_like_secret",
    "section",
    "source",
    "extractor",
    "tool",
    "load",
    "on",
    "did",
];

/// Settings whose fresh value is an empty table, so `balthasar.decay.normal = …` works.
const NESTED: &[&str] = &["decay", "witness", "weights", "window"];

/// Guards every unlisted name, so reaching for one is refused by name rather than as a nil.
const PRELUDE: &str = r#"
local refuse = __balthasar_refuse
__balthasar_refuse = nil
local allowed = {}
for _, name in ipairs(__balthasar_allowed) do allowed[name] = true end
__balthasar_allowed = nil
setmetatable(balthasar, { __index = function(_, key)
  if allowed[key] then return nil end
  refuse("balthasar." .. tostring(key))
end })
for _, space in ipairs({ "on", "did" }) do
  setmetatable(balthasar[space], { __index = function(_, key)
    return function() refuse("balthasar." .. space .. "." .. tostring(key)) end
  end })
end
"#;

/// Install the data-only module into a sandboxed VM.
pub fn install(lua: &mut Lua, config: &Rc<RefCell<Config>>, refused: &Refused) {
    lua.enter(|ctx| {
        for name in ["load", "loadstring"] {
            ctx.set_global(name, Value::Nil);
        }
        if let Value::Table(os) = ctx.get_global_value("os") {
            os.set(ctx, "getenv", Value::Nil).ok();
        }

        let balthasar = Table::new(&ctx);
        for name in NESTED {
            balthasar.set(ctx, *name, Table::new(&ctx)).ok();
        }
        for name in ["source", "extractor", "tool", "load"] {
            balthasar
                .set(
                    ctx,
                    name,
                    refusing(ctx, refused, &format!("balthasar.{name}")),
                )
                .ok();
        }
        for (space, names) in [("on", handler::ASKED), ("did", handler::TOLD)] {
            let table = Table::new(&ctx);
            for name in names {
                let said = format!("balthasar.{space}.{name}");
                table.set(ctx, *name, refusing(ctx, refused, &said)).ok();
            }
            balthasar.set(ctx, space, table).ok();
        }
        balthasar
            .set(ctx, "section", section(ctx, config, refused))
            .ok();
        crate::helpers::install_data(ctx, balthasar, &Rc::new(RefCell::new(Vec::new())));

        let held = Rc::clone(refused);
        let recorder = Callback::from_fn(&ctx, move |ctx, _exec, mut stack| {
            let name: Value = stack.consume(ctx)?;
            let name = match name {
                Value::String(s) => String::from_utf8_lossy(s.as_bytes()).into_owned(),
                _ => "balthasar.?".to_owned(),
            };
            held.borrow_mut().push(name.clone());
            Err(convert::raise(
                ctx,
                &format!("{name} is not a project file's to use"),
            ))
        });
        ctx.set_global(RECORDER, recorder);
        let allowed = Table::new(&ctx);
        for (index, name) in PROJECT_SETTINGS.iter().chain(LENT).enumerate() {
            allowed.set(ctx, index as i64 + 1, *name).ok();
        }
        ctx.set_global("__balthasar_allowed", allowed);
        ctx.set_global("balthasar", balthasar);
    });
}

/// The prelude that guards unlisted names. Run once, before the file.
#[must_use]
pub fn prelude() -> &'static str {
    PRELUDE
}

/// A callback that records `name` as refused and raises. A registrar is named with the identity
/// it was handed, as `balthasar.source("shared")`.
fn refusing<'gc>(ctx: luna::Context<'gc>, refused: &Refused, name: &str) -> Callback<'gc> {
    let held = Rc::clone(refused);
    let name = name.to_owned();
    Callback::from_fn(&ctx, move |ctx, _exec, stack| {
        let said = match stack.get(0) {
            Value::String(id) if !name.contains(".on.") && !name.contains(".did.") => {
                format!("{name}(\"{}\")", String::from_utf8_lossy(id.as_bytes()))
            }
            _ => name.clone(),
        };
        held.borrow_mut().push(said.clone());
        Err(convert::raise(
            ctx,
            &format!("{said} is not a project file's to use"),
        ))
    })
}

/// `balthasar.section(id, spec)`, data only: a spec carrying a function is refused.
fn section<'gc>(
    ctx: luna::Context<'gc>,
    config: &Rc<RefCell<Config>>,
    refused: &Refused,
) -> Callback<'gc> {
    let held = Rc::clone(config);
    let refusals = Rc::clone(refused);
    Callback::from_fn(&ctx, move |ctx, _exec, mut stack| {
        let (id, spec): (Value, Value) = stack.consume(ctx)?;
        let Value::String(id) = id else {
            return Err(convert::raise(
                ctx,
                "balthasar.section: the first argument must be a name",
            ));
        };
        let id = String::from_utf8_lossy(id.as_bytes()).into_owned();
        let said = format!("balthasar.section(\"{id}\")");
        let carried = code_in(ctx, spec, &said, 0);
        if let Some(first) = carried.first() {
            refusals.borrow_mut().push(first.clone());
            return Err(convert::raise(
                ctx,
                &format!("{first} is not a project file's to use"),
            ));
        }
        let Some(value) = convert::to_json(ctx, spec, 0) else {
            return Err(convert::raise(
                ctx,
                &format!("{said}: this table cannot be described"),
            ));
        };
        held.borrow_mut().registered.insert("section", &id, value);
        stack.replace(ctx, ());
        Ok(CallbackReturn::Return)
    })
}

/// Every place under `value` that holds something other than data, named from `at`.
pub fn code_in<'gc>(
    ctx: luna::Context<'gc>,
    value: Value<'gc>,
    at: &str,
    depth: usize,
) -> Vec<String> {
    match value {
        Value::Function(_) | Value::Thread(_) | Value::UserData(_) => vec![at.to_owned()],
        Value::Table(table) if depth < 32 => {
            let mut found = Vec::new();
            for (key, inner) in table.iter(ctx) {
                let name = match key {
                    Value::String(s) => String::from_utf8_lossy(s.as_bytes()).into_owned(),
                    Value::Integer(i) => i.to_string(),
                    _ => "?".to_owned(),
                };
                found.extend(code_in(ctx, inner, &format!("{at}.{name}"), depth + 1));
            }
            found
        }
        Value::Table(_) => vec![at.to_owned()],
        _ => Vec::new(),
    }
}

/// What a project file chose: settings it assigned, and sections it added.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Chosen {
    pub settings: Vec<(String, serde_json::Value)>,
    pub sections: Vec<(String, serde_json::Value)>,
}

/// Read what a project VM holds once its file has run, refusing anything outside the list.
///
/// # Errors
/// The first name refused, whether it was reached for while running or left behind after.
pub fn chosen(lua: &mut Lua, config: &Config, refused: &Refused) -> Result<Chosen, String> {
    if let Some(first) = refused.borrow().first() {
        return Err(first.clone());
    }
    let mut out = Chosen::default();
    let mut wrong = None;
    lua.enter(|ctx| {
        let Value::Table(balthasar) = ctx.get_global_value("balthasar") else {
            wrong = Some("balthasar".to_owned());
            return;
        };
        for (key, value) in balthasar.iter(ctx) {
            let Value::String(name) = key else {
                wrong = Some("balthasar[?]".to_owned());
                return;
            };
            let name = String::from_utf8_lossy(name.as_bytes()).into_owned();
            if LENT.contains(&name.as_str())
                && matches!(value, Value::Function(_) | Value::Table(_))
            {
                continue;
            }
            if !PROJECT_SETTINGS.contains(&name.as_str()) {
                wrong = Some(format!("balthasar.{name}"));
                return;
            }
            if let Some(first) = code_in(ctx, value, &format!("balthasar.{name}"), 0).first() {
                wrong = Some(first.clone());
                return;
            }
            let json = convert::to_json(ctx, value, 0).unwrap_or(serde_json::Value::Null);
            let empty = json.is_null() || json == serde_json::json!({});
            if !(NESTED.contains(&name.as_str()) && empty) {
                out.settings.push((name, json));
            }
        }
    });
    if let Some(name) = wrong {
        return Err(name);
    }
    out.sections = config
        .all("section")
        .into_iter()
        .map(|(id, spec)| (id.to_owned(), spec.clone()))
        .collect();
    Ok(out)
}

/// Lay a project's chosen setting over what the owner's VM already holds: tables merge key by
/// key, so an owner's function beside the chosen data stays where it was.
pub fn merge<'gc>(
    ctx: luna::Context<'gc>,
    into: Table<'gc>,
    name: &str,
    value: &serde_json::Value,
) {
    match (value, into.get_value(ctx, name)) {
        (serde_json::Value::Object(fields), Value::Table(existing)) => {
            for (key, inner) in fields {
                merge(ctx, existing, key, inner);
            }
        }
        _ => {
            let key = luna::String::from_slice(&ctx, name.as_bytes());
            into.set(ctx, key, convert::from_json(ctx, value)).ok();
        }
    }
}
