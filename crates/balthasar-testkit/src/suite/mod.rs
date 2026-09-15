//! A scenario suite of balthasar's own scenarios, written against the task shapes the published
//! benchmarks use. It runs with no download, no network and no model.

mod cases;
mod check;
mod harder;

pub use cases::{Act, Case, Category, Expect, Probe};

/// Every scenario the suite runs: the original corpus and the harder additions.
#[must_use]
pub fn corpus() -> Vec<Case> {
    let mut all = cases::corpus();
    all.extend(harder::harder());
    all
}
pub use check::{Failure, Report, Verdict, run_suite};
