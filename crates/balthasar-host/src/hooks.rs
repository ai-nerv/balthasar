use balthasar_buffer::Rules;
use balthasar_model::Memory;
use balthasar_store::{Turn, Weights};
use serde_json::Value;

/// The configuration's say in what is sent. Every method has an answer that works without one.
pub trait Hooks {
    /// A configured stub for a tool row. `None` leaves it to the tool's own words.
    fn stub(&mut self, turn: &Turn) -> Option<String> {
        let _ = turn;
        None
    }

    /// A configured policy, `(budget, items) -> layout`, replacing the rules when it answers.
    fn policy(&mut self, budget: &Value, items: &Value) -> Option<Value> {
        let _ = (budget, items);
        None
    }

    /// What of a memory may reach a remote model. `None` withholds it.
    fn redact(&mut self, text: &str, memory: &Memory) -> Option<String> {
        let _ = memory;
        Some(text.to_owned())
    }

    /// What of a transcript row may reach a helper, and so anything derived from it.
    fn withhold(&mut self, turn: &Turn) -> Option<String> {
        Some(turn.text.clone())
    }

    fn window(&self) -> Rules {
        Rules::default()
    }

    fn memory(&self) -> crate::Keeping {
        crate::Keeping::default()
    }

    fn embed(&self, text: &str) -> Option<Vec<f32>> {
        let _ = text;
        None
    }

    fn weights(&self, vectors: bool) -> Weights {
        let base = Weights::default();
        if vectors {
            base
        } else {
            base.without_vectors()
        }
    }

    fn budget(&self) -> balthasar_lua::Budget {
        balthasar_lua::Budget::default()
    }

    fn fingerprint(&self) -> String {
        String::new()
    }

    fn sections(&self) -> Vec<balthasar_recall::Section> {
        Vec::new()
    }

    fn backends(&self) -> Option<Value> {
        None
    }

    fn classify_outcome(&mut self, event: &Value) -> Option<String> {
        let _ = event;
        None
    }

    fn note_used(&mut self, action: &Value) {
        let _ = action;
    }

    fn ranker(&self) -> Option<&balthasar_recall::Model> {
        None
    }
}

/// A stub closure, as hooks that say nothing else.
pub struct Describing<F>(pub F);

impl<F: FnMut(&Turn) -> Option<String>> Hooks for Describing<F> {
    fn stub(&mut self, turn: &Turn) -> Option<String> {
        (self.0)(turn)
    }
}
