use balthasar_buffer::Rules;
use balthasar_model::Memory;
use balthasar_store::Turn;
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

    fn window(&self) -> Rules {
        Rules::default()
    }

    fn memory(&self) -> crate::Keeping {
        crate::Keeping::default()
    }
}

/// A stub closure, as hooks that say nothing else.
pub struct Describing<F>(pub F);

impl<F: FnMut(&Turn) -> Option<String>> Hooks for Describing<F> {
    fn stub(&mut self, turn: &Turn) -> Option<String> {
        (self.0)(turn)
    }
}
