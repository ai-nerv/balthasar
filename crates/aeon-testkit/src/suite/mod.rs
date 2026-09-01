//! A scenario suite, written against the task shapes the published benchmarks use.
//!
//! LongMemEval separates single-session, multi-session, temporal reasoning, knowledge update
//! and abstention. LoCoMo separates single-hop, multi-hop, temporal, open-domain and
//! adversarial. Both agree on the interesting axes and disagree on the names, so the categories
//! here are the union, plus the ones that only matter to a store which decays and which can
//! decline to answer.
//!
//! **These are aeon's own scenarios, not either dataset.** Written here so the suite runs with
//! no download, no network and no model, and so a number from it means the same thing on every
//! machine. What is borrowed is the taxonomy — which is the part worth borrowing.
//!
//! The interesting category is **abstention**. Every other system on this shelf answers *which
//! memory is most relevant*; a store with two floors can answer *nothing here is worth telling
//! you*, and that is the one behaviour worth being strict about.

mod cases;
mod check;
mod harder;

pub use cases::{Act, Case, Category, Expect, Probe};

/// Every scenario the suite runs: the original corpus and the harder additions.
///
/// One function rather than two exported lists, so nothing can run half the suite and report a
/// rate. A scenario that exists but is not run is worse than one that does not exist.
#[must_use]
pub fn corpus() -> Vec<Case> {
    let mut all = cases::corpus();
    all.extend(harder::harder());
    all
}
pub use check::{Failure, Report, Verdict, run_suite};
