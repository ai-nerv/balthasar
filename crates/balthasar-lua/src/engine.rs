//! The VM, and the module it hands `init.lua`.

use crate::{Config, LuaError, convert, handler, helpers};
use luna::{Callback, CallbackReturn, Closure, Executor, Lua, Table, Value};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

/// The keyed registrars the `balthasar` module offers. Adding one here is the only way a config
/// gains a new kind of declaration.
pub const REGISTRARS: &[&str] = &["source", "extractor", "section", "tool"];

/// Where registered specs live inside the VM, keyed by registrar then by identity. A Lua table
/// rather than a Rust map: a spec carries functions, and a function cannot cross the boundary.
pub const SPECS: &str = "__balthasar_specs";

/// The Lua VM, holding whatever the configuration has declared so far.
pub struct Engine {
    lua: Lua,
    config: Rc<RefCell<Config>>,
    logged: helpers::Logged,
    fuel: crate::fuel::Budget,
    /// What a project file reached for and was refused, by name.
    refused: crate::project::Refused,
    /// Whether the file running now is the person's own, which alone is lent `fs.read`.
    lent: Rc<std::cell::Cell<bool>>,
    /// Where the person's own files live.
    own: Option<std::path::PathBuf>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// The one place a VM is built, sandboxed before anything is installed in it.
    fn bare_vm() -> Self {
        let mut engine = Self {
            lua: Lua::full(),
            config: Rc::new(RefCell::new(Config::default())),
            logged: Rc::new(RefCell::new(Vec::new())),
            fuel: crate::fuel::Budget::default(),
            refused: Rc::new(RefCell::new(Vec::new())),
            lent: Rc::new(std::cell::Cell::new(false)),
            own: None,
        };
        engine.lua.set_memory_limit(Some(crate::fuel::MEMORY));
        // Before anything is installed and long before anything is read -- see `sandbox`.
        crate::sandbox::apply(&mut engine.lua);
        engine
    }

    /// A VM with the `balthasar` module installed and nothing declared.
    #[must_use]
    pub fn new() -> Self {
        let mut engine = Self::bare_vm();
        engine.install();
        engine
    }

    /// A VM for a file that arrived with a checkout: data only. No socket, filesystem,
    /// environment, `load` or hook; it may set only [`crate::PROJECT_SETTINGS`] and add sections.
    #[must_use]
    pub fn project() -> Self {
        let mut engine = Self::bare_vm();
        crate::project::install(&mut engine.lua, &engine.config, &engine.refused);
        engine
    }

    /// What the configuration has declared.
    #[must_use]
    pub fn config(&self) -> Config {
        let mut config = self.config.borrow().clone();
        config.log = self.logged.borrow().clone();
        config
    }

    /// Run one configuration file. A raise while loading is fatal and names the file.
    pub fn run_file(&mut self, path: &Path) -> Result<(), LuaError> {
        let source = std::fs::read_to_string(path).map_err(|source| LuaError::Io {
            file: path.display().to_string(),
            source,
        })?;
        self.run(&source, &path.display().to_string())
    }

    /// Run one chunk, under the file budget.
    pub fn run(&mut self, source: &str, chunk: &str) -> Result<(), LuaError> {
        self.run_within(source, chunk, self.fuel.file)
    }

    /// Run one chunk, stopping it once it has spent `fuel` instructions.
    fn run_within(&mut self, source: &str, chunk: &str, fuel: u64) -> Result<(), LuaError> {
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

        let runtime = |message: String| LuaError::Runtime {
            file: chunk.to_owned(),
            message,
        };
        crate::fuel::drive(&mut self.lua, &executor, fuel).map_err(runtime)?;
        self.lua
            .try_enter(|ctx| ctx.fetch(&executor).take_result::<()>(ctx)?)
            .map_err(|e| runtime(e.to_string()))
    }

    /// Set how many instructions a file and a single hook call may spend.
    pub fn budget(&mut self, file: u64, hook: u64) {
        self.fuel = crate::fuel::Budget { file, hook };
    }

    /// Read the settings the configuration assigned, once after every file has run. Settings live
    /// as plain fields so a config can read its own back and re-assign them.
    pub fn harvest(&mut self) {
        let config = Rc::clone(&self.config);
        self.lua.enter(|ctx| {
            let Value::Table(balthasar) = ctx.get_global_value("balthasar") else {
                return;
            };
            let mut held = config.borrow_mut();
            for (key, value) in balthasar.iter(ctx) {
                let Value::String(name) = key else { continue };
                let name = String::from_utf8_lossy(name.as_bytes()).into_owned();
                // A registrar is a function and cannot be described, so it is skipped here.
                if let Some(json) = convert::to_json(ctx, value, 0) {
                    held.settings.insert(name, json);
                }
            }
        });
    }

    /// Ask every handler registered for `question` until one answers. `None` means nobody claimed
    /// it, which is the `nil` contract: not mine, carry on.
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
            ctx.set_global(handler::ANSWER, Value::Nil);
            ctx.set_global(handler::ARGS, value);
        });
        if let Err(why) = self.run_within(chunk, "handler", self.fuel.hook) {
            let said = format!("a hook failed: {why}");
            balthasar_model::noted!("lua: {said}");
            self.logged.borrow_mut().push(said);
            return None;
        }

        let mut out = None;
        self.lua.enter(|ctx| {
            out = convert::to_json(ctx, ctx.get_global_value(handler::ANSWER), 0);
        });
        out.filter(|value| !value.is_null())
    }

    /// Call one function of a registered spec. `None` means the registrar, the identity, the
    /// function, or the call itself produced nothing.
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
    /// `balthasar.mask["shell"] = function(item) ... end`. Keyed rather than a list, because only
    /// one description can be sent. `None` means nobody has one, and the turn is left alone.
    pub fn mask_for(&mut self, tool: &str, item: &serde_json::Value) -> Option<String> {
        let source = format!(
            "local held = balthasar and balthasar.mask\n\
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

    /// Ask `balthasar.window.policy(budget, items)` for a layout. `None` when there is no policy,
    /// it failed, or it answered nothing.
    pub fn policy(
        &mut self,
        budget: &serde_json::Value,
        items: &serde_json::Value,
    ) -> Option<serde_json::Value> {
        let source = format!(
            "local held = balthasar and balthasar.window\n\
             local fn = type(held) == \"table\" and held.policy\n\
             if type(fn) == \"function\" then\n\
               local ok, said = pcall(fn, table.unpack({args}))\n\
               {answer} = (ok and type(said) == \"table\") and said or nil\n\
             else {answer} = nil end",
            args = handler::ARGS,
            answer = handler::ANSWER,
        );
        self.call_chunk(&source, &[budget.clone(), items.clone()])
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

    /// How many handlers are registered for a name, in either namespace. For diagnostics: a gate
    /// that never fires is usually registered against a name balthasar does not ask.
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

    /// Install the `balthasar` global.
    fn install(&mut self) {
        let config = Rc::clone(&self.config);
        let logged = Rc::clone(&self.logged);
        let lent = Rc::clone(&self.lent);
        self.lua.enter(|ctx| {
            let balthasar = Table::new(&ctx);

            for registrar in REGISTRARS {
                let held = Rc::clone(&config);
                let name = *registrar;
                let callback = Callback::from_fn(&ctx, move |ctx, _exec, mut stack| {
                    let (id, spec): (Value, Value) = stack.consume(ctx)?;
                    let Value::String(id) = id else {
                        return Err(convert::raise(
                            ctx,
                            &format!("balthasar.{name}: the first argument must be a name"),
                        ));
                    };
                    let id = String::from_utf8_lossy(id.as_bytes()).into_owned();
                    let Some(value) = convert::to_json(ctx, spec, 0) else {
                        return Err(convert::raise(
                            ctx,
                            &format!("balthasar.{name}({id}): this table cannot be described"),
                        ));
                    };
                    held.borrow_mut().registered.insert(name, &id, value);

                    // The whole table stays in the VM as well, functions and all; the JSON alone
                    // would discard everything the adapter was written to do.
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
                balthasar.set(ctx, *registrar, callback).ok();
            }

            {
                let held = Rc::clone(&config);
                let load = Callback::from_fn(&ctx, move |ctx, _exec, mut stack| {
                    let path: Value = stack.consume(ctx)?;
                    let Value::String(path) = path else {
                        return Err(convert::raise(ctx, "balthasar.load: expects a path"));
                    };
                    let path = String::from_utf8_lossy(path.as_bytes()).into_owned();
                    let mut held = held.borrow_mut();
                    if !held.loads.contains(&path) {
                        held.loads.push(path);
                    }
                    stack.replace(ctx, ());
                    Ok(CallbackReturn::Return)
                });
                balthasar.set(ctx, "load", load).ok();
            }

            // Made here so `balthasar.decay.normal = 0.02` works without a config writing
            // `balthasar.decay = {}` first.
            for nested in [
                "decay", "witness", "buffer", "weights", "budget", "mask", "window", "memory",
            ] {
                balthasar.set(ctx, nested, Table::new(&ctx)).ok();
            }

            // The socket primitive. Named twice: `balthasar.stream` for a client that knows this
            // host, `__stream` for one that does not.
            let stream = crate::stream::table(ctx);
            balthasar.set(ctx, "stream", stream).ok();
            ctx.set_global("__stream", stream);

            ctx.set_global(SPECS, Table::new(&ctx));
            handler::install(ctx, balthasar);
            helpers::install(ctx, balthasar, &logged, &lent);
            ctx.set_global("balthasar", balthasar);
        });
    }
    /// Everything a set of files declared, applied in order.
    ///
    /// A trusted file runs in this VM, and what it names with `balthasar.load` runs straight after
    /// it. An untrusted one runs in a data-only VM of its own, and only what it may choose is
    /// carried over; anything else it reached for is fatal.
    pub fn read(&mut self, files: &[(std::path::PathBuf, bool)]) -> Result<(), LuaError> {
        for (path, trusted) in files {
            if *trusted {
                let own = self.own.as_ref().is_some_and(|root| path.starts_with(root));
                self.lent.set(own);
                let ran = self
                    .run_file(path)
                    .and_then(|()| self.drain_loads(path.parent()));
                self.lent.set(false);
                ran?;
            } else {
                let chosen = Self::project_file(path)?;
                self.adopt(&chosen);
            }
        }
        self.harvest();
        Ok(())
    }

    /// Name the directory whose files are the person's own: only they are lent `fs.read`.
    pub fn own_config(&mut self, root: Option<std::path::PathBuf>) {
        self.own = root;
    }

    /// Run a project's own file in a data-only VM, and take what it chose.
    fn project_file(path: &Path) -> Result<crate::project::Chosen, LuaError> {
        let mut vm = Self::project();
        vm.run(crate::project::prelude(), "project prelude")?;
        let ran = vm.run_file(path);
        let reached = vm.refused.borrow().first().cloned();
        if let Some(what) = reached {
            return Err(LuaError::Untrusted {
                file: path.display().to_string(),
                what,
            });
        }
        ran?;
        vm.refuse_declarations(path)
    }

    /// Refuse a project file that reached past what it may choose, by the first name it reached
    /// for; otherwise answer what it chose.
    fn refuse_declarations(&mut self, path: &Path) -> Result<crate::project::Chosen, LuaError> {
        let config = self.config.borrow().clone();
        crate::project::chosen(&mut self.lua, &config, &self.refused).map_err(|what| {
            LuaError::Untrusted {
                file: path.display().to_string(),
                what,
            }
        })
    }

    /// Lay what a project file chose over this VM.
    fn adopt(&mut self, chosen: &crate::project::Chosen) {
        let config = Rc::clone(&self.config);
        self.lua.enter(|ctx| {
            let Value::Table(balthasar) = ctx.get_global_value("balthasar") else {
                return;
            };
            for (name, value) in &chosen.settings {
                crate::project::merge(ctx, balthasar, name, value);
            }
            let Value::Table(specs) = ctx.get_global_value(SPECS) else {
                return;
            };
            let slot = match specs.get::<_, Value>(ctx, "section") {
                Ok(Value::Table(existing)) => existing,
                _ => {
                    let made = Table::new(&ctx);
                    specs.set(ctx, "section", made).ok();
                    made
                }
            };
            for (id, spec) in &chosen.sections {
                config
                    .borrow_mut()
                    .registered
                    .insert("section", id, spec.clone());
                let key = luna::String::from_slice(&ctx, id.as_bytes());
                slot.set(ctx, key, convert::from_json(ctx, spec)).ok();
            }
        });
    }

    /// Run whatever the last file asked for, relative to it.
    fn drain_loads(&mut self, beside: Option<&Path>) -> Result<(), LuaError> {
        loop {
            let next = {
                let mut held = self.config.borrow_mut();
                if held.loads.is_empty() {
                    break;
                }
                held.loads.remove(0)
            };
            let path = beside.map_or_else(|| Path::new(&next).to_owned(), |dir| dir.join(&next));
            self.run_file(&path)?;
        }
        Ok(())
    }
}
