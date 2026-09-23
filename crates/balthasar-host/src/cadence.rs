//! When a maintenance job is due, measured in what has changed rather than in how often it was
//! asked, so polling raises nothing.

use balthasar_model::{SessionId, Timestamp};
use balthasar_store::{StoreError, Transcript};

/// The least time between two jobs of a kind, however fast the source moves.
pub(crate) const APART: Timestamp = 60;

/// What has changed since a job of this kind last went out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Since {
    pub progress: u64,
    pub seconds: Timestamp,
}

/// Whether one is due. Progress is the whole rule where there is a threshold: a poll writes no
/// rows and holds no claims. [`APART`] is only for `every` of 0, which asks for no threshold.
pub(crate) fn due(since: Since, every: u32) -> bool {
    if every == 0 {
        return since.seconds >= APART;
    }
    since.progress >= u64::from(every)
}

/// What has changed since the mark named `name`, and how long ago it was set.
pub(crate) fn since(
    scrollback: &Transcript,
    session: &SessionId,
    name: &str,
    progress: u64,
    now: Timestamp,
) -> Result<Since, StoreError> {
    let was = scrollback.counter(session, name)?.unwrap_or(0);
    let at = scrollback.counter(session, &format!("{name}_at"))?;
    Ok(Since {
        progress: progress.saturating_sub(was),
        seconds: at.map_or(Timestamp::MAX, |at| now.saturating_sub(at as Timestamp)),
    })
}

/// Record that one has just gone out at `progress`.
pub(crate) fn mark(
    scrollback: &Transcript,
    session: &SessionId,
    name: &str,
    progress: u64,
    now: Timestamp,
) -> Result<(), StoreError> {
    scrollback.set_counter(session, name, progress)?;
    scrollback.set_counter(session, &format!("{name}_at"), now.max(0) as u64)
}

#[cfg(test)]
#[path = "cadence/tests.rs"]
mod tests;
