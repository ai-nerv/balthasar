//! Reading the configuration, once, before anything else happens.
//!
//! Everything a command decides is downstream of this: which memory it works in, where
//! assertion begins, how fast things fade. A command that read the configuration itself would
//! be a command that could disagree with the others about all three.

use memo_lua::{Engine, LuaError, Roots, Settings};
use memo_model::ScopeId;
use std::path::Path;

/// What a configuration said about embedding.
fn embedder_from(config: &memo_lua::Config) -> Option<Box<dyn memo_embed::Embed>> {
    let said = config.get("embedder").map(|value| memo_embed::Value {
        kind: value
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        model: value
            .get("model")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
    });
    memo_embed::open(&memo_embed::Spec::read(said.as_ref()))
}

/// The configuration, and the VM that produced it.
pub struct Loaded {
    engine: Engine,
    settings: Settings,
    embedder: Option<Box<dyn memo_embed::Embed>>,
}

impl Loaded {
    /// Read the runtimepath for `cwd`.
    ///
    /// A file that exists and does not load is fatal. It expressed an intention that has not
    /// been carried out, and applying half of it is worse than refusing.
    pub fn read(cwd: &Path) -> Result<Self, LuaError> {
        let roots = Roots::discovered(cwd);
        let mut files = memo_lua::runtimepath(&roots);

        // Trust is decided after the owner's own files have run, because `memo.trusted` is one
        // of the things they set. A project file listed under it may declare like any other.
        let mut engine = Engine::new();
        let owned: Vec<(std::path::PathBuf, bool)> =
            files.iter().filter(|(_, t)| *t).cloned().collect();
        engine.read(&owned)?;
        let trusted = Settings::from(&engine.config()).trusted;
        files.retain(|(_, t)| !*t);
        for (path, allowed) in &mut files {
            *allowed = path
                .parent()
                .is_some_and(|dir| memo_lua::vouched_for(&trusted, dir));
        }
        engine.read(&files)?;

        let settings = Settings::from(&engine.config());
        let embedder = embedder_from(&engine.config());
        Ok(Self {
            engine,
            settings,
            embedder,
        })
    }

    /// A configuration that says nothing, for `--no-config` and for tests.
    ///
    /// Not "no configuration at all": the shipped defaults still apply, because a run with a
    /// broken config and a run with none must not behave differently in a way nobody notices.
    #[must_use]
    pub fn bare() -> Self {
        Self {
            engine: Engine::new(),
            settings: Settings::default(),
            embedder: memo_embed::open(&memo_embed::Spec::default()),
        }
    }

    /// The embedder, if one is configured and available.
    #[must_use]
    pub fn embedder(&self) -> Option<&dyn memo_embed::Embed> {
        self.embedder.as_deref()
    }

    /// The model backends a configuration asked for, and anything it asked for that is not
    /// carried by this build.
    #[must_use]
    pub fn distillers(&self) -> (Vec<Box<dyn memo_distil::Distil>>, Vec<String>) {
        memo_distil::backends(self.engine.config().get("distiller"))
    }

    /// The query as a vector, when there is something to embed it with.
    ///
    /// `None` is the ordinary case on a store nobody has reindexed, and the scorer redistributes
    /// the semantic weight rather than scoring every candidate at zero.
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
    /// Lua first, so a monorepo can be one scope and `~/scratch` can be none. What nothing
    /// claims falls to the repository root, which is what makes worktrees share a memory.
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
        memo_store::scope_of(cwd)
    }

    /// Walk a source's transcripts and offer what they teach.
    ///
    /// Here rather than in the command, because ingest needs the VM as well as the settings and
    /// this is the one place that holds both.
    pub fn ingest(
        &mut self,
        store: &mut memo_store::Store,
        settings: &Settings,
        ask: &memo_distil::Ingest,
    ) -> Result<memo_distil::Report, memo_distil::DistilError> {
        memo_distil::ingest(&mut self.engine, store, settings, ask)
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
    pub fn config(&self) -> memo_lua::Config {
        self.engine.config()
    }

    /// Ask the configuration whether a line may leave, and in what form.
    ///
    /// `None` withholds it. A handler that rewrites answers the rewritten text, so a key can be
    /// masked rather than the whole memory dropped.
    pub fn redact(
        &mut self,
        text: &str,
        memory: &memo_model::Memory,
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
    /// Keyed on the tool, because only its author knows what a useful stub is: "`make test` —
    /// exit 1, 41 failures" is worth sending and "[output omitted]" is not. `None` leaves the
    /// turn alone, which is right when nobody has said.
    pub fn mask(&mut self, entry: &memo_store::Turn) -> Option<String> {
        let tool = entry.tool.as_deref()?;
        let item = serde_json::json!({
            "cursor": entry.cursor,
            "tool": tool,
            "tokens": entry.tokens,
            "kind": entry.kind,
        });
        self.engine.mask_for(tool, &item)
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
