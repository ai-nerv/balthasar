//! What a configuration declared, once every file has run.

use std::collections::HashMap;

/// Everything a configuration said.
#[derive(Debug, Default, Clone)]
pub struct Config {
    /// Settings assigned onto the module, as JSON.
    pub settings: serde_json::Map<String, serde_json::Value>,
    /// Everything handed to a registrar, keyed by registrar then by identity.
    pub registered: Registered,
    /// Files `balthasar.load` asked for, in the order it asked.
    ///
    /// Collected rather than run on the spot: running a chunk from inside a chunk is
    /// re-entrancy the VM does not offer, and a queue the host drains gives the same ordering
    /// with none of it. A file already asked for is not queued twice, so a diamond terminates.
    pub loads: Vec<String>,
    /// Anything a handler passed to `balthasar.log`.
    pub log: Vec<String>,
}

impl Config {
    /// A setting, if the config assigned one.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&serde_json::Value> {
        self.settings.get(name)
    }

    /// A setting as a string.
    #[must_use]
    pub fn string(&self, name: &str) -> Option<&str> {
        self.get(name).and_then(serde_json::Value::as_str)
    }

    /// A setting as a boolean.
    #[must_use]
    pub fn boolean(&self, name: &str) -> Option<bool> {
        self.get(name).and_then(serde_json::Value::as_bool)
    }

    /// A setting as a number.
    ///
    /// Lua has one number type, so `2` and `2.0` are the same value written twice and both must
    /// answer here — a config that says `2` and gets nothing would be right to call that a bug.
    #[must_use]
    pub fn number(&self, name: &str) -> Option<f64> {
        self.get(name).and_then(serde_json::Value::as_f64)
    }

    /// A number nested one level down, as `balthasar.decay.normal` is.
    #[must_use]
    pub fn nested(&self, table: &str, name: &str) -> Option<f64> {
        self.get(table)?.get(name)?.as_f64()
    }

    /// Everything handed to one registrar, in declaration order.
    #[must_use]
    pub fn all(&self, registrar: &str) -> Vec<(&str, &serde_json::Value)> {
        self.registered
            .order
            .iter()
            .filter(|(kind, _)| kind == registrar)
            .filter_map(|(kind, id)| {
                self.registered
                    .entries
                    .get(&(kind.clone(), id.clone()))
                    .map(|value| (id.as_str(), value))
            })
            .collect()
    }

    /// One declaration by registrar and identity.
    #[must_use]
    pub fn one(&self, registrar: &str, id: &str) -> Option<&serde_json::Value> {
        self.registered
            .entries
            .get(&(registrar.to_owned(), id.to_owned()))
    }
}

/// Declarations handed to registrars.
///
/// Keyed by `(registrar, identity)`, so re-registering replaces rather than appends — the map
/// form of the rule. A config that loops over a directory and declares one source per file is
/// then idempotent, which matters because configs get re-read.
#[derive(Debug, Default, Clone)]
pub struct Registered {
    entries: HashMap<(String, String), serde_json::Value>,
    order: Vec<(String, String)>,
    /// Every `(registrar, id)` written since this was last cleared.
    ///
    /// What a file *did*, rather than what the result looks like afterwards. A privileged
    /// declaration is mostly callbacks, and callbacks are kept in the VM rather than in
    /// `entries` -- so two sources differing only in what their functions do are byte-identical
    /// here, and comparing before with after cannot see a replacement at all.
    touched: Vec<(String, String)>,
}

impl Registered {
    /// Record a declaration, replacing any earlier one with the same identity.
    pub fn insert(&mut self, registrar: &str, id: &str, value: serde_json::Value) {
        let key = (registrar.to_owned(), id.to_owned());
        self.touched.push(key.clone());
        if !self.entries.contains_key(&key) {
            self.order.push(key.clone());
        }
        self.entries.insert(key, value);
    }

    /// Forget what has been written since the last call, and answer what it was.
    ///
    /// Called around each file so a refusal can name what *that* file declared.
    pub fn take_touched(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.touched)
    }

    /// Every identity registered against one registrar, in declaration order.
    #[must_use]
    pub fn ids(&self, registrar: &str) -> Vec<String> {
        self.order
            .iter()
            .filter(|(kind, _)| kind == registrar)
            .map(|(_, id)| id.clone())
            .collect()
    }

    /// One declaration, by registrar and identity.
    #[must_use]
    pub fn one(&self, registrar: &str, id: &str) -> Option<&serde_json::Value> {
        self.entries.get(&(registrar.to_owned(), id.to_owned()))
    }

    /// How many declarations are held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing has been declared.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registering_twice_replaces_rather_than_appends() {
        // A config that loops over a directory must be safe to re-run, and configs get re-read.
        let mut registered = Registered::default();
        registered.insert("source", "harness", serde_json::json!({ "v": 1 }));
        registered.insert("source", "harness", serde_json::json!({ "v": 2 }));
        assert_eq!(registered.len(), 1);
        assert_eq!(registered.order.len(), 1);
    }

    #[test]
    fn declaration_order_survives() {
        // A section list that reshuffled between runs would move what the model reads first.
        let mut registered = Registered::default();
        for id in ["identity", "project", "recent"] {
            registered.insert("section", id, serde_json::json!({}));
        }
        let config = Config {
            registered,
            ..Config::default()
        };
        let names: Vec<&str> = config
            .all("section")
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(names, ["identity", "project", "recent"]);
    }

    #[test]
    fn a_number_written_as_an_integer_still_reads_as_one() {
        // Lua has one number type. A config that says `2` and gets nothing back would be
        // right to call that a bug.
        let mut settings = serde_json::Map::new();
        settings.insert("limit".into(), serde_json::json!(2));
        let config = Config {
            settings,
            ..Config::default()
        };
        assert_eq!(config.number("limit"), Some(2.0));
    }

    #[test]
    fn a_nested_setting_is_reachable() {
        let mut settings = serde_json::Map::new();
        settings.insert("decay".into(), serde_json::json!({ "normal": 0.05 }));
        let config = Config {
            settings,
            ..Config::default()
        };
        assert_eq!(config.nested("decay", "normal"), Some(0.05));
        assert_eq!(config.nested("decay", "missing"), None);
    }
}
