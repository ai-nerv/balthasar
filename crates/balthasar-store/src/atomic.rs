//! Immediate transactions over one transcript database.

use crate::{StoreError, Transcript};

impl Transcript {
    /// Commit these synchronous store operations together, or roll them all back on error or panic.
    /// Nested transactions are refused before their callback runs.
    pub fn atomic<T>(
        &self,
        operation: impl FnOnce(&Self) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let transaction = rusqlite::Transaction::new_unchecked(
            self.db(),
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let result = operation(self)?;
        transaction.commit()?;
        Ok(result)
    }
}
