//! Being told how to behave, by whoever is coordinating.
//!
//! balthasar runs alone perfectly well and reads its own `config/` when it does. Under a
//! coordinator it should not: the coordinator decides, and two configurations that must agree are one
//! that will not. So this is the other way in — the same Lua, the same VM, the same registrars,
//! arriving down a pipe instead of off a disk.
//!
//! **What it takes is declared, not guessed.** [`needs`] is the list a coordinator reads to know
//! what to send; anything else sent is refused by name rather than ignored, because a setting
//! that silently does nothing is the worst kind of typo.
//!
//! # What is trusted
//!
//! Whoever started this process. The chunk arrives on argv or stdin, never on the socket: a
//! socket that runs Lua is remote code execution, and the spawn link is where the trust already
//! is — a parent that can send this could have passed the same file as configuration anyway.

use crate::LuaError;
use crate::config::Config;
use crate::engine::Engine;
use serde::{Deserialize, Serialize};

/// What kind of value a setting takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A string.
    Text,
    /// A number.
    Number,
    /// True or false.
    Flag,
    /// A table — a list or a map, and `about` says which.
    Table,
}

/// One thing balthasar wants to be told.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Need {
    /// What to set, as the config names it.
    pub name: String,
    /// What sort of value it takes.
    pub kind: Kind,
    /// One line, for a person reading the list.
    pub about: String,
    /// Whether balthasar cannot work without it.
    #[serde(default)]
    pub required: bool,
    /// What it does when nothing is said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
}

/// What a `configure` call did.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Applied {
    /// Settings that took effect, by name.
    #[serde(default)]
    pub set: Vec<String>,
    /// Settings that were sent and not taken, with why.
    #[serde(default)]
    pub refused: Vec<Refused>,
}

impl Applied {
    /// Whether everything sent was taken.
    #[must_use]
    pub fn whole(&self) -> bool {
        self.refused.is_empty()
    }
}

/// One setting balthasar would not take.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Refused {
    /// What was sent.
    pub name: String,
    /// Why it was not taken.
    pub why: String,
}

/// The settings a coordinator may set.
///
/// The thresholds and decay rates `init.lua` documents. A registrar — `    std::fs::write(path, source).map_err(|source| LuaError::Io {`, `section`,
/// `extractor`, `tool` — is called rather than assigned, so it is declared in [`needs`] but not
/// listed here.
const SETTINGS: &[&str] = &[
    "promote_floor",
    "hold_floor",
    "inject_floor",
    "live_floor",
    "decay",
    "witness",
    "tool",
    "scope",
];

/// What balthasar wants to be told.
///
/// Everything has a default. A coordinator must be able to start one by saying nothing at all.
#[must_use]
pub fn needs() -> Vec<Need> {
    vec![
        Need {
            name: "promote_floor".to_owned(),
            kind: Kind::Number,
            about: "confidence a memory needs before it crosses into the project's own".to_owned(),
            required: false,
            default: Some(serde_json::json!(0.5)),
        },
        Need {
            name: "hold_floor".to_owned(),
            kind: Kind::Number,
            about: "confidence below which a memory is let go".to_owned(),
            required: false,
            default: Some(serde_json::json!(0.3)),
        },
        Need {
            name: "inject_floor".to_owned(),
            kind: Kind::Number,
            about: "confidence a memory needs before it is put in front of a model".to_owned(),
            required: false,
            default: Some(serde_json::json!(0.35)),
        },
        Need {
            name: "live_floor".to_owned(),
            kind: Kind::Number,
            about: "confidence a memory needs to stay in a live session's context".to_owned(),
            required: false,
            default: Some(serde_json::json!(0.10)),
        },
        Need {
            name: "decay".to_owned(),
            kind: Kind::Table,
            about: "how fast confidence fades: normal, high, inertia".to_owned(),
            required: false,
            default: None,
        },
        Need {
            name: "witness".to_owned(),
            kind: Kind::Table,
            about: "what each kind of evidence is worth".to_owned(),
            required: false,
            default: None,
        },
        Need {
            name: "tool".to_owned(),
            kind: Kind::Text,
            about: "whose memory this is, when the caller is not named by the kernel".to_owned(),
            required: false,
            default: None,
        },
        Need {
            name: "scope".to_owned(),
            kind: Kind::Text,
            about: "which memory to work in: global, project, or a path".to_owned(),
            required: false,
            default: Some(serde_json::json!("project")),
        },
        Need {
            name: "source".to_owned(),
            kind: Kind::Table,
            about: "a source adapter, as `balthasar.source(name, { … })`".to_owned(),
            required: false,
            default: None,
        },
        Need {
            name: "section".to_owned(),
            kind: Kind::Table,
            about: "a section of what is remembered, as `balthasar.section(name, { … })`"
                .to_owned(),
            required: false,
            default: None,
        },
    ]
}

/// Run a chunk of config Lua and say what it did.
///
/// # Errors
/// When the chunk will not compile or raises. Fatal rather than partial: a description that did
/// not run has expressed no intention, and applying half of one is worse than applying none.
pub fn configure(source: &str) -> Result<Applied, LuaError> {
    configure_into(&given(), source)
}

/// The same, kept somewhere named.
///
/// The path is a parameter so a test can own one. It was a process-wide constant, and tests
/// running together deleted each other's -- which is the same shape as two coordinators sharing
/// a balthasar, and neither should.
///
/// # Errors
/// As [`configure`].
pub fn configure_into(path: &std::path::Path, source: &str) -> Result<Applied, LuaError> {
    let mut engine = Engine::new();
    // What the VM holds before the chunk runs. The module carries entries of its own, and a
    // hardcoded list of those would be a list to keep in step with the VM; the difference is
    // what the coordinator said.
    engine.harvest();
    let before = held(&engine.config());

    engine.run(source, "configure")?;
    engine.harvest();

    let config = engine.config();
    let mut applied = Applied::default();
    for name in SETTINGS {
        // Changed, not merely present. The VM installs tables of its own -- `decay`, `witness`
        // -- so "is there" would report every one of them as something the coordinator said.
        if config.get(name).is_some() && config.get(name) != before.get(*name) {
            applied.set.push((*name).to_owned());
        }
    }
    for registrar in crate::engine::REGISTRARS {
        let declared = config.all(registrar).len();
        if declared > 0 {
            applied.set.push(format!("{registrar} ({declared})"));
        }
    }
    for name in config.settings.keys() {
        if !before.contains_key(name) && !SETTINGS.contains(&name.as_str()) {
            applied.refused.push(Refused {
                name: name.clone(),
                why: "balthasar takes no setting by that name; `needs` lists what it takes"
                    .to_owned(),
            });
        }
    }

    if applied.whole() {
        remember(path, source)?;
    }
    Ok(applied)
}

/// Every setting a config left behind, with its value.
///
/// Values rather than names, because the question is what *changed*: a VM that installs its own
/// tables would otherwise report each of them as something a coordinator set.
fn held(config: &Config) -> serde_json::Map<String, serde_json::Value> {
    config.settings.clone()
}

/// Where configuration sent by a coordinator is kept.
///
/// The runtime directory rather than the config directory: this is what a coordinator said for
/// as long as it is running, not something a person edits or should find later.
#[must_use]
pub fn given() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("balthasar").join("given.lua")
}

fn remember(path: &std::path::Path, source: &str) -> Result<(), LuaError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| LuaError::Io {
            file: path.display().to_string(),
            source,
        })?;
    }
    std::fs::write(path, source).map_err(|source| LuaError::Io {
        file: path.display().to_string(),
        source,
    })
}

/// Forget what a coordinator said, so a restart reads the files again.
pub fn forget() {
    let _ = std::fs::remove_file(given());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A place of this test's own, so tests running together do not delete each other's.
    fn mine(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("balthasar-setup-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("given.lua")
    }

    #[test]
    fn everything_it_needs_has_a_default_or_is_optional() {
        for need in needs() {
            assert!(!need.required, "{} is required", need.name);
        }
    }

    #[test]
    fn what_it_declares_is_what_it_takes() {
        let declared: Vec<String> = needs().into_iter().map(|need| need.name).collect();
        for name in SETTINGS {
            assert!(declared.contains(&(*name).to_owned()), "{name} undeclared");
        }
    }

    #[test]
    fn a_setting_it_takes_is_applied_and_named() {
        let path = mine("takes");
        let applied = configure_into(&path, "balthasar.promote_floor = 0.8").expect("runs");
        assert_eq!(applied.set, vec!["promote_floor".to_owned()], "{applied:?}");
        assert!(applied.whole());
    }

    #[test]
    fn a_setting_it_does_not_take_is_refused_by_name() {
        let path = mine("refused");
        let applied = configure_into(&path, r#"balthasar.colour = "green""#).expect("runs");
        assert!(!applied.whole(), "{applied:?}");
        assert_eq!(applied.refused[0].name, "colour");
    }

    #[test]
    fn a_chunk_that_will_not_run_leaves_nothing_behind() {
        let path = mine("broken");
        configure_into(&path, "this is not lua at all !!").expect_err("must fail");
        assert!(!path.exists());
    }

    #[test]
    fn what_was_given_outlives_the_call() {
        let path = mine("outlives");
        configure_into(&path, "balthasar.hold_floor = 0.4").expect("runs");
        let held = std::fs::read_to_string(&path).expect("kept");
        assert!(held.contains("hold_floor"), "{held}");
        let _ = std::fs::remove_file(&path);
        assert!(!path.exists());
    }
}
