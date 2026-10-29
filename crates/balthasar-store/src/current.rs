//! What may be put in front of a model as memory.

use crate::{Recall, Scored, Store, StoreError};
use balthasar_model::{Memory, Tier};

/// A recalled memory that is open: built only here, from a search of `live_memory`.
#[derive(Debug, Clone)]
pub struct Current(Scored);

impl Current {
    /// The memory.
    #[must_use]
    pub fn memory(&self) -> &Memory {
        &self.0.memory
    }

    /// The memory with how it scored.
    #[must_use]
    pub fn into_scored(self) -> Scored {
        self.0
    }
}

impl Store {
    /// A search that finds only open memories, whatever `ask` says about history.
    pub fn recall_current(&self, ask: &Recall) -> Result<Vec<Current>, StoreError> {
        let mut open = ask.clone();
        open.history = false;
        open.include_archived = false;
        Ok(self
            .recall(&open)?
            .into_iter()
            .filter(|hit| is_open(&hit.memory))
            .map(Current)
            .collect())
    }
}

fn is_open(memory: &Memory) -> bool {
    memory.temporal.valid_to.is_none()
        && memory.archived_at.is_none()
        && memory.tier != Tier::Archive
}
