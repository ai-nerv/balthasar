//! The VM, and the module it hands `init.lua`.

use crate::{Config, LuaError, config::Registered, convert, handler, helpers};
use luna::{Callback, CallbackReturn, Closure, Executor, Lua, Table, Value};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

/// The keyed registrars the `memo` module offers.
///
/// Named for the thing being described, never for when it happens. Adding one here is the only
/// way a config gains a new kind of declaration, which keeps the surface enumerable.
pub const REGISTRARS: &[&str] = &["source", "extractor", "section", "tool"];

/// Where registered specs live inside the VM, keyed by registrar then by identity.
///
/// A Lua table rather than a Rust map, because what a source or an extractor carries is
/// *functions* and a function cannot cross the boundary. The VM keeps the whole declaration;
/// Rust keeps the part that can be written down, plus the name.
pub const SPECS: &str = "__memo_specs";

/// Registrars a project's own `.memo.lua` may not use.
///
/// A source or an extractor names how somebody else's files are read and what is believed as a
/// result; a tool names a schema handed to a model. A file that arrived with `git clone` may
/// choose — set a floor, add a section — but it may not declare.
pub const PRIVILEGED: &[&str] = &["source", "extractor", "tool"];

/// Settings a project's own file may not assign, for the same reason.
pub const PRIVILEGED_SETTINGS: &[&str] = &["distiller", "embedder", "trusted"];

/// The Lua VM, holding whatever the configuration has declared so far.
pub struct Engine {
    lua: Lua,
    config: Rc<RefCell<Config>>,
    logged: helpers::Logged,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// A VM with the `memo` module installed and nothing declared.
    #[must_use]
    pub fn new() -> Self {
        let mut engine = Self {
            lua: Lua::full(),
            config: Rc::new(RefCell::new(Config::default())),
            logged: Rc::new(RefCell::new(Vec::new())),
        };
        engine.install();
        engine
    }

    /// What the configuration has declared.
    #[must_use]
    pub fn config(&self) -> Config {
        let mut config = self.config.borrow().clone();
        config.log = self.logged.borrow().clone();
        config
    }

    /// Run one configuration file.
    ///
    /// A raise while loading is fatal and names the file: a config that did not finish has not
    /// said what it wanted, and applying half of it is worse than refusing.
    pub fn run_file(&mut self, path: &Path) -> Result<(), LuaError> {
        let source = std::fs::read_to_string(path).map_err(|source| LuaError::Io {
            file: path.display().to_string(),
            source,
        })?;
        self.run(&source, &path.display().to_string())
    }

    /// Run one chunk.
    pub fn run(&mut self, source: &str, chunk: &str) -> Result<(), LuaError> {
        let executor = self
            .lua
            .try_enter(|ctx| {
                let closure = Closure::load(ctx, Some(chunk), source.as_bytes())?;
                Ok(ctx.stash(Executor::start(ctx, closure.into(), ())))
            })
            .map_err(|e| LuaError::Syntax {
                file: chunk.to_owned(),
                message: e.to_string(),
            })?;

        self.lua
            .execute::<()>(&executor)
            .map_err(|e| LuaError::Runtime {
                file: chunk.to_owned(),
                message: e.to_string(),
            })
    }

    /// Read the settings the configuration assigned, and forget the module's own fields.
    ///
    /// Called once after every file has run. Settings live as plain fields so a config can read
    /// its own back and re-assign them; harvesting here is what keeps that true without a write
    /// barrier, and only the value it finished with is the one it meant.
    pub fn harvest(&mut self) {
        let config = Rc::clone(&self.config);
        self.lua.enter(|ctx| {
            let Value::Table(memo) = ctx.get_global_value("memo") else {
                return;
            };
            let mut held = config.borrow_mut();
            for (key, value) in memo.iter(ctx) {
                let Value::String(name) = key else { continue };
                let name = String::from_utf8_lossy(name.as_bytes()).into_owned();
                // A registrar is a function and cannot be described. Skipping those is what
                // makes "every other field is a setting" work without a list to keep in step.
                if let Some(json) = convert::to_json(ctx, value, 0) {
                    held.settings.insert(name, json);
                }
            }
        });
    }

    /// Ask every handler registered for `question` until one answers.
    ///
    /// `None` means nobody claimed it, which is the `nil` contract: not mine, carry on.
    pub fn ask(&mut self, question: &str, args: &[serde_json::Value]) -> Option<serde_json::Value> {
        self.call_chunk(&handler::asking(question), args)
    }

    /// Tell every handler registered for `event`.
    pub fn tell(&mut self, event: &str, args: &[serde_json::Value]) {
        self.call_chunk(&handler::telling(event), args);
    }

    /// Run a generated chunk with `args` in place, and take what it left.
    fn call_chunk(&mut self, chunk: &str, args: &[serde_json::Value]) -> Option<serde_json::Value> {
        let packed = serde_json::Value::Array(args.to_vec());
        self.lua.enter(|ctx| {
            let value = convert::from_json(ctx, &packed);
            ctx.set_global(handler::ARGS, value);
        });
        self.run(chunk, "handler").ok()?;

        let mut out = None;
        self.lua.enter(|ctx| {
            out = convert::to_json(ctx, ctx.get_global_value(handler::ANSWER), 0);
        });
        out.filter(|value| !value.is_null())
    }

    /// Call one function of a registered spec.
    ///
    /// `None` means the registrar, the identity, the function, or the call itself produced
    /// nothing — all of which a caller treats the same way: the adapter cannot answer, so the
    /// line is skipped rather than guessed at.
    pub fn call(
        &mut self,
        registrar: &str,
        id: &str,
        method: &str,
        args: &[serde_json::Value],
    ) -> Option<serde_json::Value> {
        let source = format!(
            "local held = {SPECS} and {SPECS}[{registrar:?}]\n\
             local spec = held and held[{id:?}]\n\
             local fn = spec and spec[{method:?}]\n\
             if fn then\n\
               local ok, answer = pcall(fn, table.unpack({args}))\n\
               {answer} = ok and answer or nil\n\
             else {answer} = nil end",
            args = handler::ARGS,
            answer = handler::ANSWER,
        );
        self.call_chunk(&source, args)
    }

    /// Ask the mask handler for a tool what a masked result should say.
    ///
    /// `memo.mask["shell"] = function(item) ... end`. Keyed rather than a list, because only
    /// one description can be sent and a second handler would be silently ignored. `None`
    /// means nobody has one, and a turn nobody can describe is left alone: there is nothing
    /// honest to put in its place.
    pub fn mask_for(&mut self, tool: &str, item: &serde_json::Value) -> Option<String> {
        let source = format!(
            "local held = memo and memo.mask\n\
             local fn = held and held[{tool:?}]\n\
             if type(fn) == \"function\" then\n\
               local ok, said = pcall(fn, table.unpack({args}))\n\
               {answer} = (ok and type(said) == \"string\") and said or nil\n\
             else {answer} = nil end",
            args = handler::ARGS,
            answer = handler::ANSWER,
        );
        self.call_chunk(&source, std::slice::from_ref(item))?
            .as_str()
            .map(str::to_owned)
    }

    /// Whether a registered spec offers a function by that name.
    #[must_use]
    pub fn offers(&mut self, registrar: &str, id: &str, method: &str) -> bool {
        let source = format!(
            "local held = {SPECS} and {SPECS}[{registrar:?}]\n\
             local spec = held and held[{id:?}]\n\
             {answer} = spec ~= nil and type(spec[{method:?}]) == \"function\"",
            answer = handler::ANSWER,
        );
        self.call_chunk(&source, &[])
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// How many handlers are registered for a name, in either namespace.
    ///
    /// For diagnostics: a config whose gate never fires is usually a config that registered it
    /// against a name memo does not ask.
    #[must_use]
    pub fn handlers(&mut self, namespace: &str, name: &str) -> usize {
        let store = if namespace == "on" {
            handler::ON
        } else {
            handler::DID
        };
        let mut count = 0;
        self.lua.enter(|ctx| {
            if let Value::Table(holder) = ctx.get_global_value(store)
                && let Ok(Value::Table(list)) = holder.get::<_, Value>(ctx, name)
            {
                count = list.length(&ctx).max(0) as usize;
            }
        });
        count
    }

    /// Install the `memo` global.
    fn install(&mut self) {
        let config = Rc::clone(&self.config);
        let logged = Rc::clone(&self.logged);
        self.lua.enter(|ctx| {
            let memo = Table::new(&ctx);

            for registrar in REGISTRARS {
                let held = Rc::clone(&config);
                let name = *registrar;
                let callback = Callback::from_fn(&ctx, move |ctx, _exec, mut stack| {
                    let (id, spec): (Value, Value) = stack.consume(ctx)?;
                    let Value::String(id) = id else {
                        return Err(convert::raise(
                            ctx,
                            &format!("memo.{name}: the first argument must be a name"),
                        ));
                    };
                    let id = String::from_utf8_lossy(id.as_bytes()).into_owned();
                    let Some(value) = convert::to_json(ctx, spec, 0) else {
                        return Err(convert::raise(
                            ctx,
                            &format!("memo.{name}({id}): this table cannot be described"),
                        ));
                    };
                    held.borrow_mut().registered.insert(name, &id, value);

                    // The whole table stays in the VM as well, functions and all. A source
                    // adapter is mostly callbacks, and a registrar that kept only the JSON
                    // would silently discard everything the adapter was written to do.
                    if let Value::Table(specs) = ctx.get_global_value(SPECS) {
                        let slot = match specs.get::<_, Value>(ctx, name) {
                            Ok(Value::Table(existing)) => existing,
                            _ => {
                                let made = Table::new(&ctx);
                                specs.set(ctx, name, made).ok();
                                made
                            }
                        };
                        let key = luna::String::from_slice(&ctx, id.as_bytes());
                        slot.set(ctx, key, spec).ok();
                    }
                    stack.replace(ctx, ());
                    Ok(CallbackReturn::Return)
                });
                memo.set(ctx, *registrar, callback).ok();
            }

            {
                let held = Rc::clone(&config);
                let load = Callback::from_fn(&ctx, move |ctx, _exec, mut stack| {
                    let path: Value = stack.consume(ctx)?;
                    let Value::String(path) = path else {
                        return Err(convert::raise(ctx, "memo.load: expects a path"));
                    };
                    let path = String::from_utf8_lossy(path.as_bytes()).into_owned();
                    let mut held = held.borrow_mut();
                    if !held.loads.contains(&path) {
                        held.loads.push(path);
                    }
                    stack.replace(ctx, ());
                    Ok(CallbackReturn::Return)
                });
                memo.set(ctx, "load", load).ok();
            }

            // Made here so `memo.decay.normal = 0.02` works without a config writing
            // `memo.decay = {}` first. Plain settings tables: nothing registers into them, and
            // a config may still replace either wholesale.
            for nested in ["decay", "witness", "buffer", "weights", "budget", "mask"] {
                memo.set(ctx, nested, Table::new(&ctx)).ok();
            }

            // The socket primitive, so the family's clients run unchanged in this VM. Named
            // twice: `memo.stream` for a client that knows this host, `__stream` for one that
            // does not.
            let stream = crate::stream::table(ctx);
            memo.set(ctx, "stream", stream).ok();
            ctx.set_global("__stream", stream);

            ctx.set_global(SPECS, Table::new(&ctx));
            handler::install(ctx, memo);
            helpers::install(ctx, memo, &logged);
            ctx.set_global("memo", memo);
        });
    }

    /// Everything a set of files declared, applied in order.
    ///
    /// `trusted` decides whether a file may declare as well as choose. What a file names with
    /// `memo.load` is run straight after it, before the next file, so a config reads top to
    /// bottom the way it is written.
    pub fn read(&mut self, files: &[(std::path::PathBuf, bool)]) -> Result<(), LuaError> {
        for (path, trusted) in files {
            let before = self.snapshot();
            self.run_file(path)?;
            if !trusted {
                self.refuse_declarations(path, &before)?;
            }
            self.drain_loads(path.parent(), *trusted)?;
        }
        self.harvest();
        Ok(())
    }

    /// What is declared right now, so an untrusted file's additions can be spotted.
    fn snapshot(&mut self) -> (Registered, Vec<Option<serde_json::Value>>) {
        let registered = self.config.borrow().registered.clone();
        (registered, self.privileged_settings())
    }

    /// The settings a project file may not touch, as they stand.
    ///
    /// Read straight out of the VM rather than out of the harvested config, because harvesting
    /// happens once at the end and this has to be checked after each file.
    fn privileged_settings(&mut self) -> Vec<Option<serde_json::Value>> {
        let mut out = Vec::new();
        self.lua.enter(|ctx| {
            let Value::Table(memo) = ctx.get_global_value("memo") else {
                return;
            };
            for name in PRIVILEGED_SETTINGS {
                let held = memo
                    .get::<_, Value>(ctx, *name)
                    .ok()
                    .and_then(|v| convert::to_json(ctx, v, 0))
                    .filter(|v| !v.is_null());
                out.push(held);
            }
        });
        out
    }

    /// Refuse a project file that declared rather than chose.
    ///
    /// A source or an extractor says how somebody's transcripts are read and what is believed
    /// as a result; a distiller or an embedder names an endpoint text is sent to or a command
    /// to run. A file that arrived with `git clone` may set a floor or add a section. It may
    /// not do either of those.
    fn refuse_declarations(
        &mut self,
        path: &Path,
        before: &(Registered, Vec<Option<serde_json::Value>>),
    ) -> Result<(), LuaError> {
        let (registered, settings) = before;
        let now = self.config.borrow().registered.clone();
        for registrar in PRIVILEGED {
            let added: Vec<String> = now
                .ids(registrar)
                .into_iter()
                .filter(|id| registered.one(registrar, id).is_none())
                .collect();
            if let Some(first) = added.first() {
                return Err(LuaError::Untrusted {
                    file: path.display().to_string(),
                    what: format!("memo.{registrar}(\"{first}\")"),
                });
            }
        }
        for (index, name) in PRIVILEGED_SETTINGS.iter().enumerate() {
            let was = settings.get(index).and_then(Option::as_ref);
            let is = self.privileged_settings();
            let is = is.get(index).and_then(Option::as_ref);
            if is != was {
                return Err(LuaError::Untrusted {
                    file: path.display().to_string(),
                    what: format!("memo.{name}"),
                });
            }
        }
        Ok(())
    }

    /// Run whatever the last file asked for, relative to it.
    fn drain_loads(&mut self, beside: Option<&Path>, trusted: bool) -> Result<(), LuaError> {
        loop {
            let next = {
                let mut held = self.config.borrow_mut();
                if held.loads.is_empty() {
                    break;
                }
                held.loads.remove(0)
            };
            let path = beside.map_or_else(|| Path::new(&next).to_owned(), |dir| dir.join(&next));
            let before = self.snapshot();
            self.run_file(&path)?;
            if !trusted {
                self.refuse_declarations(&path, &before)?;
            }
        }
        Ok(())
    }
}
