//! What a VM can reach, built from an allowlist.
//!
//! Every VM starts from luna's core library, which opens no file and spawns nothing; only a
//! person's own configuration is given `os` as well. Every global, and every field of a library
//! table, that is not named below is then taken out, before anything is installed, so a name a
//! later luna adds is unreachable until it is listed here. `io`, `package` and `debug` are never
//! loaded at all. No object is ever enrolled for a finalizer: `setmetatable` refuses a metatable
//! carrying `__gc`, and `getmetatable` hands out no userdata's metatable.

use luna::{Callback, CallbackReturn, Context, Lua, MetaMethod, Table, Value};

/// What a metatable carrying `__gc` is told: no finalizer ever runs here.
const FINALIZED: &str = "a metatable with __gc is refused: finalizers never run here";

/// Which VM is being built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// The person's own configuration, its plugins and its hooks.
    Owner,
    /// A file that arrived with a checkout: data only.
    Project,
}

/// A global and the fields kept of it. An empty list keeps the value as it is.
type Kept = (&'static str, &'static [&'static str]);

/// What every VM keeps. Not listed: `print` and `warn`, which write to this process's own output;
/// `collectgarbage`, which drives the collector; `loadfile` and `dofile`, which read code from a
/// file; and `string.dump`, which writes the bytecode nothing here loads.
const CORE: &[Kept] = &[
    ("_G", &[]),
    ("_VERSION", &[]),
    ("assert", &[]),
    ("error", &[]),
    ("getmetatable", &[]),
    ("ipairs", &[]),
    ("next", &[]),
    ("pairs", &[]),
    ("pcall", &[]),
    ("rawequal", &[]),
    ("rawget", &[]),
    ("rawlen", &[]),
    ("rawset", &[]),
    ("select", &[]),
    ("setmetatable", &[]),
    ("tonumber", &[]),
    ("tostring", &[]),
    ("type", &[]),
    ("xpcall", &[]),
    (
        "coroutine",
        &[
            "close",
            "continue",
            "create",
            "isyieldable",
            "resume",
            "running",
            "status",
            "wrap",
            "yield",
            "yieldto",
        ],
    ),
    (
        "math",
        &[
            "abs",
            "acos",
            "asin",
            "atan",
            "ceil",
            "cos",
            "deg",
            "exp",
            "floor",
            "fmod",
            "huge",
            "log",
            "max",
            "maxinteger",
            "min",
            "mininteger",
            "modf",
            "pi",
            "rad",
            "random",
            "randomseed",
            "sin",
            "sqrt",
            "tan",
            "tointeger",
            "type",
            "ult",
        ],
    ),
    (
        "string",
        &[
            "byte", "char", "find", "format", "gmatch", "gsub", "len", "lower", "match", "pack",
            "packsize", "rep", "reverse", "sub", "unpack", "upper",
        ],
    ),
    (
        "table",
        &[
            "clear",
            "concat",
            "freeze",
            "insert",
            "interceptall",
            "isfrozen",
            "move",
            "pack",
            "remove",
            "sort",
            "unpack",
        ],
    ),
    (
        "utf8",
        &["char", "charpattern", "codepoint", "codes", "len", "offset"],
    ),
];

/// What a person's own configuration keeps besides: `load`, because the family's clients are
/// loaded chunks, and the clock and the environment from `os`.
const OWNER: &[Kept] = &[
    ("load", &[]),
    ("os", &["clock", "date", "difftime", "getenv", "time"]),
];

/// A VM holding exactly what `surface` is allowed, and nothing installed in it yet.
#[must_use]
pub fn vm(surface: Surface) -> Lua {
    let mut lua = Lua::core();
    let mut kept: Vec<Kept> = CORE.to_vec();
    if surface == Surface::Owner {
        lua.load_os();
        kept.extend_from_slice(OWNER);
    }
    lua.enter(|ctx| {
        let globals = ctx.globals();
        prune(ctx, globals, |name| {
            kept.iter().any(|(held, _)| *held == name)
        });
        for (name, fields) in &kept {
            if fields.is_empty() {
                continue;
            }
            if let Value::Table(library) = globals.get_value(ctx, *name) {
                prune(ctx, library, |field| fields.contains(&field));
            }
        }
        guard(ctx, globals, "setmetatable", finalized);
        hide_userdata_metatables(ctx, globals);
    });
    lua
}

/// `getmetatable`, answering `nil` for a userdata: luna re-attaches one shared metatable to every
/// error it raises as a userdata, and `__gc` set on it would enrol each later error.
fn hide_userdata_metatables<'gc>(ctx: Context<'gc>, globals: Table<'gc>) {
    let Value::Function(original) = globals.get_value(ctx, "getmetatable") else {
        return;
    };
    let hidden = Callback::from_fn_with(&ctx, original, |original, ctx, _exec, mut stack| {
        if matches!(stack.get(0), Value::UserData(_)) {
            stack.replace(ctx, Value::Nil);
            return Ok(CallbackReturn::Return);
        }
        Ok(CallbackReturn::Call {
            function: *original,
            then: None,
        })
    });
    globals.set(ctx, "getmetatable", hidden).ok();
}

/// A check on a call's first two arguments, naming why it refuses them.
type Refusal = for<'gc> fn(Context<'gc>, Value<'gc>, Value<'gc>) -> Option<&'static str>;

/// Put `refuses` in front of the function at `table[name]`, which then runs as it did.
fn guard<'gc>(ctx: Context<'gc>, table: Table<'gc>, name: &'static str, refuses: Refusal) {
    let Value::Function(original) = table.get_value(ctx, name) else {
        return;
    };
    let guarded = Callback::from_fn_with(&ctx, original, move |original, ctx, _exec, stack| {
        if let Some(why) = refuses(ctx, stack.get(0), stack.get(1)) {
            return Err(crate::convert::raise(ctx, why));
        }
        Ok(CallbackReturn::Call {
            function: *original,
            then: None,
        })
    });
    table.set(ctx, name, guarded).ok();
}

/// A metatable carrying `__gc`, which would enrol its object for a finalizer.
fn finalized<'gc>(ctx: Context<'gc>, _: Value<'gc>, metatable: Value<'gc>) -> Option<&'static str> {
    matches!(metatable, Value::Table(metatable) if !metatable.get_value(ctx, MetaMethod::Gc).is_nil())
        .then_some(FINALIZED)
}

/// Take every key out of `table` that `keep` does not name. In place, so the string metatable's
/// `__index`, which is the `string` table itself, loses the same fields.
fn prune<'gc>(ctx: luna::Context<'gc>, table: Table<'gc>, keep: impl Fn(&str) -> bool) {
    let named = |key: &Value<'gc>| match key {
        Value::String(name) => std::str::from_utf8(name.as_bytes()).is_ok_and(&keep),
        _ => false,
    };
    let gone: Vec<Value<'gc>> = table
        .iter(ctx)
        .map(|(key, _)| key)
        .filter(|key| !named(key))
        .collect();
    for key in gone {
        table.set(ctx, key, Value::Nil).ok();
    }
}

#[cfg(test)]
mod tests {
    use crate::Engine;
    use std::collections::BTreeSet;

    /// What one expression evaluates to inside a fresh engine.
    fn probe(expression: &str) -> String {
        let mut engine = Engine::new();
        engine
            .run(
                &format!("balthasar.answer = tostring({expression})"),
                "probe.lua",
            )
            .expect("run");
        engine.harvest();
        engine
            .config()
            .string("answer")
            .unwrap_or("<absent>")
            .to_owned()
    }

    #[test]
    fn a_config_cannot_spawn_a_process() {
        assert_eq!(probe("os.execute"), "nil");
        assert_eq!(probe("io"), "nil");
    }

    #[test]
    fn a_config_cannot_write_outside_the_store() {
        for expression in ["os.remove", "os.rename", "os.tmpname"] {
            assert_eq!(probe(expression), "nil", "{expression} is still reachable");
        }
    }

    #[test]
    fn a_config_cannot_end_a_serving_daemon() {
        assert_eq!(probe("os.exit"), "nil");
    }

    #[test]
    fn a_config_cannot_load_arbitrary_files() {
        for expression in ["dofile", "loadfile", "require", "package"] {
            assert_eq!(probe(expression), "nil", "{expression} is still reachable");
        }
    }

    #[test]
    fn a_config_reaches_no_debug_output_or_collector() {
        for expression in ["debug", "print", "warn", "collectgarbage", "string.dump"] {
            assert_eq!(probe(expression), "nil", "{expression} is still reachable");
        }
        assert_eq!(
            probe("('').dump"),
            "nil",
            "dump is reachable through a string"
        );
    }

    #[test]
    fn what_a_config_legitimately_needs_still_works() {
        // `load` must survive: the family's client stubs are loaded chunks.
        assert_ne!(probe("os.getenv"), "nil", "reading the environment is fine");
        assert_ne!(probe("os.time"), "nil");
        assert_ne!(
            probe("load"),
            "nil",
            "the family's clients are loaded chunks"
        );
        assert_ne!(probe("string.format"), "nil");
        assert_ne!(probe("table.concat"), "nil");
        assert_eq!(probe("('ab'):rep(2)"), "abab");
    }

    /// Every name a VM holds, two tables deep: globals, their fields, and those fields' fields.
    const WALK: &str = "local names = {}
        local function walk(held, prefix, depth)
          for key, value in pairs(held) do
            local name = prefix .. tostring(key)
            names[#names + 1] = name
            if type(value) == 'table' and value ~= _G and depth < 2 then
              walk(value, name .. '.', depth + 1)
            end
          end
        end
        walk(_G, '', 0)
        table.sort(names)
        balthasar.answer = table.concat(names, ' ')";

    /// The core library both VMs hold.
    const CORE: &str = "
        _G _VERSION assert error getmetatable ipairs next pairs pcall rawequal rawget rawlen rawset
        select setmetatable tonumber tostring type xpcall
        coroutine coroutine.close coroutine.continue coroutine.create coroutine.isyieldable
        coroutine.resume coroutine.running coroutine.status coroutine.wrap coroutine.yield
        coroutine.yieldto
        math math.abs math.acos math.asin math.atan math.ceil math.cos math.deg math.exp
        math.floor math.fmod math.huge math.log math.max math.maxinteger math.min math.mininteger
        math.modf math.pi math.rad math.random math.randomseed math.sin math.sqrt math.tan
        math.tointeger math.type math.ult
        string string.byte string.char string.find string.format string.gmatch string.gsub
        string.len string.lower string.match string.pack string.packsize string.rep
        string.reverse string.sub string.unpack string.upper
        table table.clear table.concat table.freeze table.insert table.interceptall
        table.isfrozen table.move table.pack table.remove table.sort table.unpack
        utf8 utf8.char utf8.charpattern utf8.codepoint utf8.codes utf8.len utf8.offset
    ";

    /// What a person's own configuration holds besides the core.
    const OWNER: &str = "
        load os os.clock os.date os.difftime os.getenv os.time
        __stream __stream.connect
        __balthasar_specs
        __balthasar_on __balthasar_on.scope __balthasar_on.admit __balthasar_on.promote
        __balthasar_on.importance __balthasar_on.redact __balthasar_on.outcome
        __balthasar_did __balthasar_did.observe __balthasar_did.promote __balthasar_did.contradict
        __balthasar_did.recall __balthasar_did.forget __balthasar_did.consolidate
        __balthasar_did.compact __balthasar_did.used
        balthasar balthasar.source balthasar.extractor balthasar.section balthasar.tool
        balthasar.load balthasar.decay balthasar.witness balthasar.buffer balthasar.weights
        balthasar.budget balthasar.mask balthasar.window balthasar.memory
        balthasar.stream balthasar.stream.connect
        balthasar.on balthasar.on.scope balthasar.on.admit balthasar.on.promote
        balthasar.on.importance balthasar.on.redact balthasar.on.outcome
        balthasar.did balthasar.did.observe balthasar.did.promote balthasar.did.contradict
        balthasar.did.recall balthasar.did.forget balthasar.did.consolidate balthasar.did.compact
        balthasar.did.used
        balthasar.json balthasar.json.encode balthasar.json.decode
        balthasar.path balthasar.path.basename balthasar.text balthasar.text.tail
        balthasar.log balthasar.looks_like_secret
        balthasar.git balthasar.git.common_dir balthasar.fs balthasar.fs.glob balthasar.fs.read
        balthasar.home balthasar.data_home balthasar.config_home
    ";

    /// What a project file holds besides the core, once the prelude has run.
    const PROJECT: &str = "
        balthasar balthasar.source balthasar.extractor balthasar.section balthasar.tool
        balthasar.load balthasar.decay balthasar.witness balthasar.weights balthasar.window
        balthasar.on balthasar.on.scope balthasar.on.admit balthasar.on.promote
        balthasar.on.importance balthasar.on.redact balthasar.on.outcome
        balthasar.did balthasar.did.observe balthasar.did.promote balthasar.did.contradict
        balthasar.did.recall balthasar.did.forget balthasar.did.consolidate balthasar.did.compact
        balthasar.did.used
        balthasar.json balthasar.json.encode balthasar.json.decode
        balthasar.path balthasar.path.basename balthasar.text balthasar.text.tail
        balthasar.log balthasar.looks_like_secret
    ";

    /// The names `engine` holds, against `allowed`, in both directions.
    fn holds_exactly(mut engine: Engine, allowed: &[&str]) {
        engine.run(WALK, "walk.lua").expect("run");
        engine.harvest();
        let answer = engine
            .config()
            .string("answer")
            .unwrap_or_default()
            .to_owned();
        let found: BTreeSet<&str> = answer.split(' ').collect();
        let allowed: BTreeSet<&str> = allowed.iter().flat_map(|l| l.split_whitespace()).collect();
        let added: Vec<_> = found.difference(&allowed).collect();
        let gone: Vec<_> = allowed.difference(&found).collect();
        assert!(added.is_empty(), "reachable and not allowed: {added:?}");
        assert!(gone.is_empty(), "allowed and no longer there: {gone:?}");
        for never in [
            "debug",
            "print",
            "warn",
            "collectgarbage",
            "loadfile",
            "dofile",
            "io",
        ] {
            assert!(!found.contains(never), "{never} is reachable");
        }
    }

    #[test]
    fn the_names_a_configuration_can_reach_are_exactly_the_allowlist() {
        holds_exactly(Engine::new(), &[CORE, OWNER]);
    }

    #[test]
    fn the_names_a_project_file_can_reach_are_exactly_the_allowlist() {
        let mut engine = Engine::project();
        engine
            .run(crate::project::prelude(), "project prelude")
            .expect("prelude");
        holds_exactly(engine, &[CORE, PROJECT]);
    }

    /// What `body` left in `balthasar.answer`, run in a project file's VM.
    fn in_project(body: &str) -> String {
        let mut engine = Engine::project();
        engine
            .run(crate::project::prelude(), "project prelude")
            .expect("prelude");
        engine.run(body, "probe.lua").expect("run");
        engine.harvest();
        engine
            .config()
            .string("answer")
            .unwrap_or("<absent>")
            .to_owned()
    }

    #[test]
    fn a_metatable_carrying_gc_is_refused_and_any_other_is_taken() {
        let refused = "select(2, pcall(setmetatable, {}, { __gc = function() end }))";
        assert!(probe(refused).contains("__gc"), "{}", probe(refused));
        let said = in_project(&format!("balthasar.answer = {refused}"));
        assert!(said.contains("__gc"), "{said}");
        assert_eq!(probe("setmetatable({}, { __index = { x = 7 } }).x"), "7");
        let locked = "setmetatable(setmetatable({}, { __metatable = 1 }), {})";
        assert!(probe(&format!("select(2, pcall(function() {locked} end))")).contains("protected"));
    }

    #[test]
    fn no_route_enrols_an_object_for_a_finalizer() {
        let mut engine = Engine::new();
        let before = engine.held();
        engine
            .run(
                "local gc = function() end
                 local took = 0
                 local _, raised = pcall(string.rep)
                 local shared = getmetatable(raised)
                 if type(shared) == 'table' then shared.__gc = gc end
                 for i = 1, 20000 do
                   local big = string.rep('x', 256) .. i
                   if pcall(setmetatable, { big }, { __gc = gc }) then took = took + 1 end
                   if pcall(rawget(_G, 'setmetatable'), { big }, { __gc = gc }) then
                     took = took + 1
                   end
                   local late = {}
                   setmetatable({ big }, late)
                   late.__gc = gc
                   pcall(string.rep)
                 end
                 balthasar.took = took
                 balthasar.shared = type(shared)",
                "enrol.lua",
            )
            .expect("runs");
        engine.harvest();
        assert_eq!(engine.config().number("took"), Some(0.0));
        assert_eq!(engine.config().string("shared"), Some("nil"));
        let after = engine.held();
        assert!(after < before + (1 << 20), "{before} bytes -> {after}");
    }
}
