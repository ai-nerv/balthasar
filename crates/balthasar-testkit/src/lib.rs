//! Whether session k+1 is less annoying than session k.

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
pub use run::{Arm, Measured, Ran, Score, WINDOW, measure, measure_arm, run, run_arm};
pub use scenario::{Lesson, Scenario, Session};
pub use suite::{Act, Case, Category, Expect, Failure, Probe, Report, Verdict, corpus, run_suite};
pub use train::{Example, Fitted, TrainError, auc, features_of, fit, label};
