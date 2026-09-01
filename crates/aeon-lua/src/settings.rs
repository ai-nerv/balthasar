//! What a configuration's numbers mean.
//!
//! The bridge between "the config said something" and "aeon behaves differently". Every knob
//! has a default that is the plan's number, so an empty configuration and no configuration at
//! all behave identically — which is what makes a fresh install work before anybody has written
//! a line of Lua.

use crate::Config;
use aeon_model::{Importance, WitnessKind, floor};

/// The thresholds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Floors {
    /// Confidence at which a memory is asserted to a model.
    pub inject: f64,
    /// Confidence at which a memory stays in the live set.
    pub live: f64,
    /// Score a candidate must reach to leave its session.
    pub promote: f64,
    /// Score at which a candidate waits for a second witness.
    pub hold: f64,
}

impl Default for Floors {
    fn default() -> Self {
        Self {
            inject: floor::INJECT,
            live: floor::LIVE,
            promote: floor::PROMOTE,
            hold: floor::HOLD,
        }
    }
}

/// How fast each class fades.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Decay {
    /// Per-day constants, in the order of [`Importance`].
    pub critical: f64,
    /// See [`Decay::critical`].
    pub high: f64,
    /// See [`Decay::critical`].
    pub normal: f64,
    /// See [`Decay::critical`].
    pub low: f64,
    /// Whether what is needed often resists fading.
    pub inertia: bool,
}

impl Default for Decay {
    fn default() -> Self {
        Self {
            critical: Importance::Critical.rate(),
            high: Importance::High.rate(),
            normal: Importance::Normal.rate(),
            low: Importance::Low.rate(),
            inertia: true,
        }
    }
}

/// How much each retrieval signal counts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weights {
    /// Cosine similarity, when embeddings exist.
    pub semantic: f64,
    /// Full-text ranking. The floor, always present.
    pub lexical: f64,
    /// What the query and the memory are both about, rarity-weighted.
    pub entity: f64,
    /// How often and how recently a memory has been needed.
    pub frecency: f64,
    /// How sure.
    pub confidence: f64,
    /// How faded.
    pub strength: f64,
    /// Whether the project outranks the global store.
    pub scope: f64,
}

impl Default for Weights {
    fn default() -> Self {
        Self {
            semantic: 0.26,
            lexical: 0.22,
            entity: 0.14,
            frecency: 0.13,
            confidence: 0.13,
            strength: 0.08,
            scope: 0.04,
        }
    }
}

/// What a single recall may spend.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Budget {
    /// Hard cap on candidates gathered across all phases.
    pub candidates: usize,
    /// Soft wall-clock cap; expansion stops once it is spent.
    pub milliseconds: u64,
    /// Cap on terms query expansion may add.
    pub expansion_terms: usize,
    /// How far multi-hop expansion may walk.
    pub hops: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            candidates: 500,
            milliseconds: 50,
            expansion_terms: 32,
            hops: 2,
        }
    }
}

/// Whether and how long the use-and-outcome ledger is kept.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ledger {
    /// Whether searches, injections and outcomes are recorded at all.
    ///
    /// Off by default. The ledger is instrumentation: it costs writes on the recall path, and a
    /// memory layer that silently starts recording what a person searches for because a new
    /// version shipped is not one anybody should install.
    pub capture: bool,
    /// How long a ledger row lives before retention drops it.
    pub retention_days: u32,
}

impl Default for Ledger {
    fn default() -> Self {
        Self {
            capture: false,
            retention_days: 90,
        }
    }
}

/// Everything a configuration decided, in the shapes aeon uses.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// The thresholds.
    pub floors: Floors,
    /// How fast things fade.
    pub decay: Decay,
    /// How retrieval ranks.
    pub weights: Weights,
    /// Whether the use-and-outcome ledger records anything.
    pub ledger: Ledger,
    /// What a recall may spend.
    pub budget: Budget,
    /// What each kind of evidence is worth.
    pub witness: Vec<(WitnessKind, f64)>,
    /// Directories whose own `.aeon.lua` may declare.
    pub trusted: Vec<String>,
    /// Which tool this configuration's memories belong to, when the binary name is not it.
    pub tool: Option<String>,
    /// Words that mark a user turn as an instruction to remember.
    pub imperatives: Vec<String>,
}

impl Default for Settings {
    /// What aeon does when nothing has been configured.
    ///
    /// Deliberately built through [`Settings::from`] rather than derived. A derived default
    /// would leave the witness weights and the imperative list empty, so "no configuration"
    /// and "an empty configuration" would behave differently — which is a difference nobody
    /// would ever think to look for.
    fn default() -> Self {
        Self::from(&Config::default())
    }
}

impl Settings {
    /// How retrieval ranks, as this configuration set it.
    #[must_use]
    pub fn weights(&self) -> &Weights {
        &self.weights
    }

    /// The thresholds, as this configuration set them.
    #[must_use]
    pub fn floors(&self) -> &Floors {
        &self.floors
    }

    /// Read a configuration, falling back to the shipped number for anything it did not say.
    ///
    /// Every knob is optional and every default is the plan's, so an empty configuration and no
    /// configuration behave identically.
    #[must_use]
    pub fn from(config: &Config) -> Self {
        let floors = Floors {
            inject: config.number("inject_floor").unwrap_or(floor::INJECT),
            live: config.number("live_floor").unwrap_or(floor::LIVE),
            promote: config.number("promote_floor").unwrap_or(floor::PROMOTE),
            hold: config.number("hold_floor").unwrap_or(floor::HOLD),
        };
        let fallback = Decay::default();
        let decay = Decay {
            critical: config
                .nested("decay", "critical")
                .unwrap_or(fallback.critical),
            high: config.nested("decay", "high").unwrap_or(fallback.high),
            normal: config.nested("decay", "normal").unwrap_or(fallback.normal),
            low: config.nested("decay", "low").unwrap_or(fallback.low),
            inertia: config
                .get("decay")
                .and_then(|d| d.get("inertia"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(fallback.inertia),
        };
        let fallback = Weights::default();
        let weights = Weights {
            semantic: config
                .nested("weights", "semantic")
                .unwrap_or(fallback.semantic),
            lexical: config
                .nested("weights", "lexical")
                .unwrap_or(fallback.lexical),
            entity: config
                .nested("weights", "entity")
                .unwrap_or(fallback.entity),
            frecency: config
                .nested("weights", "frecency")
                .unwrap_or(fallback.frecency),
            confidence: config
                .nested("weights", "confidence")
                .unwrap_or(fallback.confidence),
            strength: config
                .nested("weights", "strength")
                .unwrap_or(fallback.strength),
            scope: config.nested("weights", "scope").unwrap_or(fallback.scope),
        };
        let fallback = Budget::default();
        let budget = Budget {
            candidates: count(config, "budget", "candidates", fallback.candidates),
            milliseconds: count(
                config,
                "budget",
                "milliseconds",
                fallback.milliseconds as usize,
            ) as u64,
            expansion_terms: count(
                config,
                "budget",
                "expansion_terms",
                fallback.expansion_terms,
            ),
            hops: count(config, "budget", "hops", fallback.hops),
        };

        let fallback_ledger = Ledger::default();

        Self {
            floors,
            decay,
            weights,
            budget,
            ledger: Ledger {
                // One table, `aeon.outcome`, rather than a flat flag beside a nested number.
                // Two shapes for one concern is how a configuration surface becomes a thing
                // people have to look up.
                capture: config
                    .get("outcome")
                    .and_then(|held| held.get("capture"))
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(fallback_ledger.capture),
                retention_days: count(
                    config,
                    "outcome",
                    "retention_days",
                    fallback_ledger.retention_days as usize,
                ) as u32,
            },
            witness: witness_weights(config),
            tool: config.string("tool").map(str::to_owned),
            trusted: strings(config, "trusted"),
            imperatives: imperatives(config),
        }
    }

    /// A digest of every setting that changes what aeon does.
    ///
    /// Written out field by field rather than derived from a serialization, so that adding a
    /// field to `Settings` cannot silently change every recorded fingerprint. When a new
    /// setting starts mattering it is added here deliberately, and the fingerprint changing is
    /// the point.
    ///
    /// Recorded beside every measurement and every ledger row: two numbers produced under
    /// different weights are not the same experiment, and without this nothing would say so.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        aeon_model::content_hash(&format!(
            "floors:{:.4},{:.4},{:.4},{:.4}|weights:{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4}|\
             decay:{:.6},{:.6},{:.6},{:.6},{}|budget:{},{},{},{}|imperatives:{}",
            self.floors.inject,
            self.floors.live,
            self.floors.promote,
            self.floors.hold,
            self.weights.semantic,
            self.weights.lexical,
            self.weights.entity,
            self.weights.frecency,
            self.weights.confidence,
            self.weights.strength,
            self.weights.scope,
            self.decay.critical,
            self.decay.high,
            self.decay.normal,
            self.decay.low,
            self.decay.inertia,
            self.budget.candidates,
            self.budget.milliseconds,
            self.budget.expansion_terms,
            self.budget.hops,
            self.imperatives.len(),
        ))[..16]
            .to_owned()
    }

    /// Whether the ledger records, and for how long.
    #[must_use]
    pub fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    /// Which tool this configuration names, if it names one.
    ///
    /// A wrapper script or a dev build under another name is the case this exists for: the
    /// kernel would call it something nobody recognises, and the memories would land in a
    /// directory named after the wrapper.
    #[must_use]
    pub fn tool(&self) -> Option<&str> {
        self.tool.as_deref()
    }

    /// The decay constant for a class, as this configuration set it.
    #[must_use]
    pub fn rate(&self, importance: Importance) -> f64 {
        match importance {
            Importance::Critical => self.decay.critical,
            Importance::High => self.decay.high,
            Importance::Normal => self.decay.normal,
            Importance::Low => self.decay.low,
        }
    }

    /// What one witness of a kind is worth, as this configuration set it.
    #[must_use]
    pub fn weight(&self, kind: WitnessKind) -> f64 {
        self.witness
            .iter()
            .find(|(k, _)| *k == kind)
            .map_or_else(|| kind.weight(), |(_, w)| *w)
    }
}

/// A nested count, as a `usize`.
fn count(config: &Config, table: &str, name: &str, fallback: usize) -> usize {
    config
        .nested(table, name)
        .filter(|n| *n >= 0.0)
        .map_or(fallback, |n| n as usize)
}

/// A setting that is a list of strings.
fn strings(config: &Config, name: &str) -> Vec<String> {
    config
        .get(name)
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The words that mark a turn as an instruction, defaulting to the shipped list.
fn imperatives(config: &Config) -> Vec<String> {
    let said = strings(config, "imperatives");
    if said.is_empty() {
        return ["remember", "always", "never", "from now on", "note that"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
    }
    said
}

/// Per-kind witness weights, defaulting to the shipped ones.
fn witness_weights(config: &Config) -> Vec<(WitnessKind, f64)> {
    const KINDS: &[WitnessKind] = &[
        WitnessKind::Imperative,
        WitnessKind::Correction,
        WitnessKind::Cost,
        WitnessKind::Repetition,
        WitnessKind::Distillation,
        WitnessKind::Consolidation,
        WitnessKind::Manual,
    ];
    KINDS
        .iter()
        .map(|kind| {
            let said = config.nested("witness", kind.as_str());
            (*kind, said.unwrap_or_else(|| kind.weight()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(json: serde_json::Value) -> Settings {
        let serde_json::Value::Object(settings) = json else {
            panic!("a table");
        };
        Settings::from(&Config {
            settings,
            ..Config::default()
        })
    }

    #[test]
    fn saying_nothing_is_the_same_as_having_no_configuration() {
        // A fresh install has to work before anybody has written a line of Lua.
        assert_eq!(with(serde_json::json!({})), Settings::default());
    }

    #[test]
    fn a_floor_a_config_set_is_the_floor_that_is_used() {
        let settings = with(serde_json::json!({ "inject_floor": 0.6 }));
        assert_eq!(settings.floors.inject, 0.6);
        assert_eq!(settings.floors.live, floor::LIVE, "the rest are untouched");
    }

    #[test]
    fn a_nested_knob_can_be_set_alone() {
        // `aeon.decay.normal = 0.02` must not reset the other three to zero.
        let settings = with(serde_json::json!({ "decay": { "normal": 0.02 } }));
        assert_eq!(settings.decay.normal, 0.02);
        assert_eq!(settings.decay.high, Importance::High.rate());
    }

    #[test]
    fn a_witness_weight_can_be_reset() {
        let settings = with(serde_json::json!({ "witness": { "distillation": 0.1 } }));
        assert_eq!(settings.weight(WitnessKind::Distillation), 0.1);
        assert_eq!(settings.weight(WitnessKind::Imperative), 1.0);
    }

    #[test]
    fn the_shipped_imperatives_stand_until_a_config_replaces_them() {
        assert!(
            with(serde_json::json!({}))
                .imperatives
                .contains(&"remember".to_owned())
        );
        let mine = with(serde_json::json!({ "imperatives": ["merk"] }));
        assert_eq!(mine.imperatives, ["merk"]);
    }

    #[test]
    fn a_negative_count_is_refused_rather_than_wrapped() {
        // `budget.candidates = -1` as a usize is eighteen quintillion, which is not a bound.
        let settings = with(serde_json::json!({ "budget": { "candidates": -1 } }));
        assert_eq!(settings.budget.candidates, Budget::default().candidates);
    }
}
