//! Which background work goes out, in what order, and how much at once. The policy is balthasar’s.

use balthasar_store::{Job, JobState};

/// How many jobs of one session may be out at once, and what a backlog drains to before more go.
pub(crate) const HIGH: usize = 3;
pub(crate) const LOW: usize = 1;

/// Where a kind sits when several are due together.
const ORDER: [&str; 7] = [
    "curate",
    "extract",
    "summarise",
    "working",
    "contradict",
    "retitle",
    "tidy",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Room {
    Full,
    For(usize),
}

impl Room {
    /// How many may go, given how many are already out.
    pub(crate) fn given(running: usize, draining: bool) -> Self {
        if running >= HIGH || (draining && running > LOW) {
            return Self::Full;
        }
        Self::For(HIGH - running)
    }

    pub(crate) fn any(self) -> bool {
        matches!(self, Self::For(n) if n > 0)
    }
}

/// Where `kind` sits. An unknown kind goes last rather than first.
pub(crate) fn rank(kind: &str) -> usize {
    ORDER
        .iter()
        .position(|known| *known == kind)
        .unwrap_or(ORDER.len())
}

/// The queued jobs in the order they should go out, highest first.
pub(crate) fn ordered(jobs: &[Job]) -> Vec<&Job> {
    let mut queued: Vec<&Job> = jobs
        .iter()
        .filter(|job| job.state == JobState::Queued)
        .collect();
    // Stable within a rank, so two of a kind keep the order they were queued in.
    queued.sort_by_key(|job| rank(&job.kind));
    queued
}

/// Whether a job must go whatever the watermarks say: a request is waiting on it.
pub(crate) fn urgent(job: &Job) -> bool {
    job.spec["blocking"].as_bool().unwrap_or(false)
}

#[cfg(test)]
#[path = "scheduling/tests.rs"]
mod tests;
