//! A slice of what a model is told.

use balthasar_model::{Importance, Tier};
use serde::Deserialize;

/// One section, as a configuration declared it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Section {
    /// Its identity, which is also how a later declaration replaces it.
    #[serde(skip)]
    pub id: String,

    /// Its share of the budget, relative to the other sections.
    #[serde(default = "one")]
    pub weight: f64,

    /// Where it appears. Lower comes first.
    #[serde(default)]
    pub order: i64,

    /// Which tiers it draws from. Empty means every durable one.
    #[serde(default)]
    pub tiers: Vec<Tier>,

    /// What it will not take.
    #[serde(default)]
    pub filter: Filter,

    /// How many lines at most, whatever the budget allows.
    #[serde(default)]
    pub limit: Option<usize>,

    /// Keep the order things came in rather than sorting by salience.
    #[serde(default)]
    pub preserve_order: bool,

    /// A confidence floor of this section's own, above the global one.
    #[serde(default)]
    pub min_confidence: Option<f64>,

    /// Score against the turn in hand rather than taking what is most salient overall.
    #[serde(default)]
    pub query: Option<String>,
}

/// What a section will not take.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Filter {
    /// Only these predicates.
    #[serde(default)]
    pub predicate: Vec<String>,
    /// Only this scope: `project` or `global`.
    #[serde(default)]
    pub scope: Option<String>,
    /// Only these importance classes.
    #[serde(default)]
    pub importance: Vec<Importance>,
}

fn one() -> f64 {
    1.0
}

impl Section {
    /// Read one from what a configuration declared.
    ///
    /// A malformed declaration answers `None` rather than raising.
    #[must_use]
    pub fn read(id: &str, spec: &serde_json::Value) -> Option<Self> {
        // `where` is reserved in Rust, so the field is `filter` here and aliased on the way in.
        let mut spec = spec.clone();
        if let Some(object) = spec.as_object_mut()
            && let Some(said) = object.remove("where")
        {
            object.insert("filter".into(), said);
        }
        let mut section: Self = serde_json::from_value(spec).ok()?;
        section.id = id.to_owned();
        (section.weight > 0.0).then_some(section)
    }

    /// Every section a configuration declared, in the order they will be rendered.
    #[must_use]
    pub fn all(config: &balthasar_lua::Config) -> Vec<Self> {
        let mut found: Vec<Self> = config
            .all("section")
            .into_iter()
            .filter_map(|(id, spec)| Self::read(id, spec))
            .collect();
        // Stable on `order`, so two sections that did not say keep the order they were declared in.
        found.sort_by_key(|section| section.order);
        found
    }

    /// Whether a memory belongs in this section.
    #[must_use]
    pub fn takes(&self, memory: &balthasar_model::Memory, project: bool) -> bool {
        if !self.tiers.is_empty() && !self.tiers.contains(&memory.tier) {
            return false;
        }
        if !self.filter.predicate.is_empty() {
            let Some((_, predicate)) = memory.body.slot() else {
                return false;
            };
            if !self.filter.predicate.iter().any(|p| p == predicate) {
                return false;
            }
        }
        match self.filter.scope.as_deref() {
            Some("project") if !project => return false,
            Some("global") if project => return false,
            _ => {}
        }
        if !self.filter.importance.is_empty()
            && !self.filter.importance.contains(&memory.strength.importance)
        {
            return false;
        }
        true
    }

    /// The confidence a memory must reach for this section, given the global floor.
    #[must_use]
    pub fn floor(&self, global: f64) -> f64 {
        self.min_confidence.map_or(global, |mine| mine.max(global))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use balthasar_model::{Body, Memory, MemoryId, ScopeId};

    fn memory(tier: Tier, body: Body) -> Memory {
        Memory::new(MemoryId::new("m"), tier, ScopeId::global(), body, 0)
    }

    #[test]
    fn a_section_reads_the_shape_a_config_writes() {
        let spec = serde_json::json!({
            "weight": 4, "order": 20,
            "tiers": ["habit", "fact"],
            "where": { "scope": "project", "importance": ["high"] },
            "min_confidence": 0.6,
        });
        let section = Section::read("how", &spec).expect("a section");
        assert_eq!(section.weight, 4.0);
        assert_eq!(section.tiers, [Tier::Habit, Tier::Fact]);
        assert_eq!(section.filter.scope.as_deref(), Some("project"));
        assert_eq!(section.min_confidence, Some(0.6));
    }

    #[test]
    fn a_section_that_would_take_no_room_is_not_a_section() {
        assert!(Section::read("x", &serde_json::json!({ "weight": 0 })).is_none());
    }

    #[test]
    fn a_malformed_section_costs_itself_and_nothing_else() {
        assert!(Section::read("x", &serde_json::json!({ "tiers": "not a list" })).is_none());
    }

    #[test]
    fn a_section_with_no_tiers_takes_any() {
        let section = Section::read("x", &serde_json::json!({})).expect("a section");
        assert!(section.takes(
            &memory(
                Tier::Episode,
                Body::note("x", balthasar_model::NoteKind::Claim)
            ),
            true
        ));
    }

    #[test]
    fn a_predicate_filter_excludes_what_has_no_slot() {
        let section = Section::read(
            "x",
            &serde_json::json!({ "where": { "predicate": ["name"] } }),
        )
        .expect("a section");
        assert!(section.takes(&memory(Tier::Fact, Body::fact("you", "name", "Sam")), true));
        assert!(!section.takes(&memory(Tier::Fact, Body::fact("you", "shell", "sh")), true));
        assert!(!section.takes(
            &memory(
                Tier::Fact,
                Body::note("x", balthasar_model::NoteKind::Claim)
            ),
            true
        ));
    }

    #[test]
    fn a_scope_filter_tells_the_two_stores_apart() {
        let section = Section::read("x", &serde_json::json!({ "where": { "scope": "project" } }))
            .expect("a section");
        let m = memory(Tier::Fact, Body::fact("a", "b", "c"));
        assert!(section.takes(&m, true));
        assert!(!section.takes(&m, false));
    }

    #[test]
    fn a_sections_own_floor_can_only_raise_the_global_one() {
        // A section asking for less certainty than the store asserts would route around the floor.
        let section =
            Section::read("x", &serde_json::json!({ "min_confidence": 0.1 })).expect("a section");
        assert_eq!(section.floor(0.35), 0.35);
        let strict =
            Section::read("x", &serde_json::json!({ "min_confidence": 0.6 })).expect("a section");
        assert_eq!(strict.floor(0.35), 0.6);
    }

    #[test]
    fn sections_that_did_not_say_keep_the_order_they_were_declared_in() {
        let mut config = balthasar_lua::Config::default();
        for id in ["a", "b", "c"] {
            config
                .registered
                .insert("section", id, serde_json::json!({ "weight": 1 }));
        }
        let names: Vec<String> = Section::all(&config).into_iter().map(|s| s.id).collect();
        assert_eq!(names, ["a", "b", "c"]);
    }
}
