//! Whether session k+1 is less annoying than session k.
//!
//! LoCoMo and LongMemEval measure conversational recall, and they measure it well. Neither asks
//! the question a person using a coding agent has: *did it stop rediscovering things*. That one
//! is ours to invent, and it is the only number here that would make somebody change what they
//! run.

mod adapter;
mod attack;
mod baseline;
mod experiment;
mod run;
mod scenario;
mod suite;
mod train;

pub use adapter::{Dataset, Family, Found, load};
pub use attack::{ATTACKS, Attack, Report as AttackReport, Verdict as AttackVerdict, run_attacks};
pub use baseline::{Baseline, RECALL_P95_BUDGET_MS};
pub use experiment::{Decision, Full, Manifest, Metric, Moved, Outcome};
pub use run::{Measured, Ran, Score, measure, run};
pub use scenario::{Lesson, Scenario, Session};
pub use suite::{Act, Case, Category, Expect, Failure, Probe, Report, Verdict, corpus, run_suite};
pub use train::{Example, Fitted, TrainError, auc, features_of, fit, label};
