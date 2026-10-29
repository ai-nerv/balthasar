//! Reading the configuration, once, before anything else happens.

use balthasar_lua::{Engine, LuaError, Roots, Settings};
use balthasar_model::ScopeId;
use std::path::Path;

/// What a configuration said about embedding.
fn embedder_from(config: &balthasar_lua::Config) -> Option<Box<dyn balthasar_embed::Embed>> {
    let said = config.get("embedder").map(|value| balthasar_embed::Value {
        kind: value
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        model: value
            .get("model")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        path: value
            .get("path")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
    });
    balthasar_embed::open(&balthasar_embed::Spec::read(said.as_ref()))
}

/// The configuration, and the VM that produced it.
pub struct Loaded {
    engine: Engine,
    settings: Settings,
    embedder: Option<Box<dyn balthasar_embed::Embed>>,
    /// `balthasar.window` and `balthasar.memory`, read once.
    window: balthasar_host::Rules,
    keeping: balthasar_host::Keeping,
    /// What the mask hook said for each turn as it stood, so a layout does not ask again.
    stubs: std::collections::HashMap<String, Option<String>>,
}

impl balthasar_host::Hooks for Loaded {
    fn stub(&mut self, turn: &balthasar_store::Turn) -> Option<String> {
        let said = self.mask(turn);
        if let Some(stub) = &said {
            balthasar_model::noted!(
                "hook: stub for cursor {} in {} chars",
                turn.cursor,
                stub.chars().count()
            );
        }
        said
    }

    fn policy(
        &mut self,
        budget: &serde_json::Value,
        items: &serde_json::Value,
    ) -> Option<serde_json::Value> {
        let said = self.engine.policy(budget, items);
        balthasar_model::noted!(
            "hook: policy {}",
            if said.is_some() {
                "changed the layout"
            } else {
                "left it"
            }
        );
        said
    }

    fn redact(&mut self, text: &str, memory: &balthasar_model::Memory) -> Option<String> {
        let said = Loaded::redact(self, text, memory, true, &mut Vec::new());
        if said.is_some() {
            balthasar_model::noted!("hook: redact rewrote a memory");
        }
        said
    }

    fn withhold(&mut self, turn: &balthasar_store::Turn) -> Option<String> {
        let row = serde_json::json!({
            "text": turn.text, "role": turn.role, "kind": turn.kind, "cursor": turn.cursor,
        });
        let Some(said) = self.engine.ask("withhold", &[row]) else {
            return Some(turn.text.clone());
        };
        if said.get("drop").and_then(serde_json::Value::as_bool) == Some(true) {
            return None;
        }
        Some(
            said.get("text")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(&turn.text)
                .to_owned(),
        )
    }

    fn window(&self) -> balthasar_host::Rules {
        self.window.clone()
    }

    fn memory(&self) -> balthasar_host::Keeping {
        self.keeping.clone()
    }
}

impl Loaded {
    /// Read the runtimepath for `cwd`.
    ///
    /// A file that exists and does not load is fatal.
    pub fn read(cwd: &Path) -> Result<Self, LuaError> {
        Self::from_roots(&Roots::discovered(cwd))
    }

    /// The same, from roots already found.
    fn from_roots(roots: &Roots) -> Result<Self, LuaError> {
        let mut files = balthasar_lua::runtimepath(roots);

        // A package under `site/pack/` runs once acknowledged, and again only once it has been
        // re-acknowledged after a change. See `balthasar_lua::acknowledged`.
        let held = withheld(roots);
        files.retain(|(path, _)| !held.contains(path));

        // Trust is decided after the owner's own files have run; they set `balthasar.trusted`.
        let mut engine = Engine::new();
        engine.own_config(roots.config.clone());

        let owned: Vec<(std::path::PathBuf, bool)> =
            files.iter().filter(|(_, t)| *t).cloned().collect();
        engine.read(&owned)?;
        let owner = Settings::from(&engine.config());
        let owner_window = balthasar_host::Rules::read(engine.config().get("window"));
        files.retain(|(_, t)| !*t);
        for (path, allowed) in &mut files {
            *allowed = path
                .parent()
                .is_some_and(|dir| balthasar_lua::vouched_for(&owner.trusted, dir));
        }
        // The project's own file first, so the owner's files and the coordinator's override it.
        let unvouched = files.iter().any(|(_, allowed)| !*allowed);
        if !files.is_empty() {
            engine = Engine::new();
            engine.own_config(roots.config.clone());
            engine.read(&files)?;
            engine.read(&owned)?;
        }

        let mut settings = Settings::from(&engine.config());
        let embedder = embedder_from(&engine.config());
        let mut window = balthasar_host::Rules::read(engine.config().get("window"));
        // A repository nobody vouched for may raise a floor, never lower one, and never words
        // the warning put in front of a model.
        if unvouched {
            settings.floors = settings.floors.at_least(&owner.floors);
            window.warning = owner_window.warning;
        }
        let keeping = balthasar_host::Keeping::read(engine.config().get("memory"));
        Ok(Self {
            engine,
            settings,
            embedder,
            window,
            keeping,
            stubs: std::collections::HashMap::new(),
        })
    }

    /// A configuration that says nothing, for `--no-config` and for tests.
    ///
    /// Not "no configuration at all": the shipped defaults still apply.
    #[must_use]
    pub fn bare() -> Self {
        Self {
            engine: Engine::new(),
            settings: Settings::default(),
            embedder: balthasar_embed::open(&balthasar_embed::Spec::default()),
            window: balthasar_host::Rules::default(),
            keeping: balthasar_host::Keeping::default(),
            stubs: std::collections::HashMap::new(),
        }
    }

    /// The embedder, if one is configured and available.
    #[must_use]
    pub fn embedder(&self) -> Option<&dyn balthasar_embed::Embed> {
        self.embedder.as_deref()
    }

    /// The model backends a configuration asked for, and anything it asked for that is not
    /// carried by this build.
    #[must_use]
    pub fn distillers(&self) -> (Vec<Box<dyn balthasar_distil::Distil>>, Vec<String>) {
        balthasar_distil::backends(self.engine.config().get("distiller"))
    }

    /// The query as a vector, when there is something to embed it with.
    ///
    /// `None` is ordinary on a store nobody has reindexed; the scorer redistributes the weight.
    #[must_use]
    pub fn embed_query(&self, text: &str) -> Option<Vec<f32>> {
        if text.trim().is_empty() {
            return None;
        }
        self.embedder()?
            .embed(&[text.to_owned()])
            .ok()?
            .into_iter()
            .next()
    }

    /// What the configuration decided.
    #[must_use]
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Which memory a directory belongs to.
    ///
    /// Lua first. What nothing claims falls to the repository root, so worktrees share a memory.
    pub fn scope_of(&mut self, cwd: &Path) -> ScopeId {
        let asked = self
            .engine
            .ask("scope", &[serde_json::json!(cwd.to_string_lossy())]);
        if let Some(answer) = asked
            && let Some(id) = answer.get("id").and_then(serde_json::Value::as_str)
            && !id.is_empty()
        {
            return ScopeId::new(id);
        }
        balthasar_store::scope_of(cwd)
    }

    /// Walk a source's transcripts and offer what they teach.
    pub fn ingest(
        &mut self,
        store: &mut balthasar_store::Store,
        settings: &Settings,
        ask: &balthasar_distil::Ingest,
    ) -> Result<balthasar_distil::Report, balthasar_distil::DistilError> {
        balthasar_distil::ingest(&mut self.engine, store, settings, ask)
    }

    /// One consolidation pass, with this configuration's `promote` gate.
    pub fn consolidate(
        &mut self,
        store: &mut balthasar_store::Store,
        scratch: Option<&mut balthasar_store::Scratchpad>,
        scope: &ScopeId,
        at: balthasar_model::Timestamp,
        dry_run: bool,
    ) -> Result<balthasar_distil::Consolidated, balthasar_distil::DistilError> {
        let settings = self.settings.clone();
        let gate = Some(&mut self.engine);
        balthasar_distil::consolidate(store, scratch, gate, &settings, scope, at, dry_run)
    }

    /// Read one of this project's own runs and offer what it taught.
    pub fn distil(
        &mut self,
        store: &mut balthasar_store::Store,
        held: &balthasar_store::Transcript,
        session: &balthasar_model::SessionId,
        ask: &balthasar_distil::Ingest,
    ) -> Result<balthasar_distil::Report, balthasar_distil::DistilError> {
        let settings = self.settings.clone();
        balthasar_distil::distil_run(store, &mut self.engine, &settings, held, session, ask)
    }

    /// Every source a configuration declared.
    #[must_use]
    pub fn sources(&self) -> Vec<String> {
        self.engine
            .config()
            .all("source")
            .into_iter()
            .map(|(id, _)| id.to_owned())
            .collect()
    }

    /// What the configuration declared, for whoever needs to read a registrar.
    #[must_use]
    pub fn config(&self) -> balthasar_lua::Config {
        self.engine.config()
    }

    /// Ask the configuration whether a line may leave, and in what form.
    ///
    /// `None` withholds it. A handler that rewrites answers the rewritten text.
    pub fn redact(
        &mut self,
        text: &str,
        memory: &balthasar_model::Memory,
        remote: bool,
        withheld: &mut Vec<String>,
    ) -> Option<String> {
        let line = serde_json::json!({
            "text": text,
            "privacy": memory.privacy.as_str(),
            "tier": memory.tier.as_str(),
            "confidence": memory.confidence,
        });
        let context = serde_json::json!({
            "destination": if remote { "remote" } else { "local" },
        });

        let Some(said) = self.engine.ask("redact", &[line, context]) else {
            return Some(text.to_owned());
        };
        if said.get("drop").and_then(serde_json::Value::as_bool) == Some(true) {
            withheld.push(text.to_owned());
            return None;
        }
        Some(
            said.get("text")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(text)
                .to_owned(),
        )
    }

    /// What a masked tool result should say instead.
    ///
    /// Keyed on the tool. `None` hands the turn to the tool's own stub, then the generic one.
    /// `tokens` is always a number: the row's own count, or its estimate.
    pub fn mask(&mut self, entry: &balthasar_store::Turn) -> Option<String> {
        let tool = entry.tool.as_deref()?;
        let item = serde_json::json!({
            "cursor": entry.cursor,
            "tool": tool,
            "tokens": entry.weight(),
            "kind": entry.kind,
            "stub": entry.stub,
            "handle": entry.handle,
            "error": entry.error,
        });
        // Keyed by the revision and everything the hook is shown, so a changed turn asks again.
        let key = format!("{}:{item}", entry.revisions);
        if let Some(said) = self.stubs.get(&key) {
            return said.clone();
        }
        let said = self.engine.mask_for(tool, &item);
        if self.stubs.len() >= STUBS {
            self.stubs.clear();
        }
        self.stubs.insert(key, said.clone());
        said
    }

    /// Tell every handler registered for an event.
    pub fn tell(&mut self, event: &str, args: &[serde_json::Value]) {
        self.engine.tell(event, args);
    }

    /// Anything the configuration logged, so a command can show it.
    #[must_use]
    pub fn log(&self) -> Vec<String> {
        self.engine.config().log
    }
}

/// How many hook answers are kept before the cache starts over.
const STUBS: usize = 4096;

/// Installed package files that are not cleared to run.
fn withheld(roots: &Roots) -> std::collections::BTreeSet<std::path::PathBuf> {
    let Some(config) = &roots.config else {
        return std::collections::BTreeSet::new();
    };
    let known =
        balthasar_lua::acknowledged::recorded(&balthasar_lua::acknowledged::manifest_in(config));
    let mut held = std::collections::BTreeSet::new();
    for path in balthasar_lua::installed(roots) {
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        if balthasar_lua::acknowledged::cleared(&known, &path, &source) {
            continue;
        }
        eprintln!(
            "balthasar: {}; run `balthasar acknowledge` to clear it",
            balthasar_lua::acknowledged::Held {
                path: path.clone(),
                known: balthasar_lua::acknowledged::seen(&known, &path),
            }
        );
        held.insert(path);
    }
    held
}

/// Acknowledge every installed package, so it may run.
///
/// # Errors
/// When the manifest cannot be written — a read-only configuration directory, most likely.
pub fn trust(cwd: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    let roots = Roots::discovered(cwd);
    let Some(config) = roots.config.clone() else {
        return Err("no configuration directory to write a manifest in".to_owned());
    };
    let files: Vec<(std::path::PathBuf, String)> = balthasar_lua::installed(&roots)
        .into_iter()
        .filter_map(|path| {
            std::fs::read_to_string(&path)
                .ok()
                .map(|source| (path, source))
        })
        .collect();
    balthasar_lua::acknowledged::acknowledge(
        &balthasar_lua::acknowledged::manifest_in(&config),
        &files,
    )?;
    Ok(files.into_iter().map(|(path, _)| path).collect())
}

#[cfg(test)]
mod stubs {
    use super::*;

    #[test]
    fn a_turn_is_described_once_per_revision_rather_than_once_per_layout() {
        let mut loaded = Loaded::bare();
        loaded
            .engine
            .run(
                "balthasar.mask['shell'] = function(item) balthasar.log('asked') return 'ran it' end",
                "init.lua",
            )
            .expect("registered");
        let turn = balthasar_store::Turn {
            cursor: 3,
            role: "tool".into(),
            kind: "tool_result".into(),
            tool: Some("shell".into()),
            ..balthasar_store::Turn::default()
        };
        for _ in 0..3 {
            assert_eq!(loaded.mask(&turn).as_deref(), Some("ran it"));
        }
        let revised = balthasar_store::Turn {
            revisions: 1,
            ..turn
        };
        assert_eq!(loaded.mask(&revised).as_deref(), Some("ran it"));
        assert_eq!(loaded.log().len(), 2, "{:?}", loaded.log());
    }
}

#[cfg(test)]
mod precedence;
