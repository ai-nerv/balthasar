//! Whether session k+1 is less annoying than session k.
//!
//! LoCoMo and LongMemEval measure conversational recall, and they measure it well. Neither asks
//! the question a person using a coding agent has: *did it stop rediscovering things*. That one
//! is ours to invent, and it is the only number here that would make somebody change what they
//! run.

mod run;
mod scenario;
mod suite;

pub use run::{Ran, Score, run};
pub use scenario::{Lesson, Scenario, Session};
pub use suite::{Act, Case, Category, Expect, Failure, Probe, Report, Verdict, corpus, run_suite};
