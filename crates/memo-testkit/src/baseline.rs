//! The measured baseline: what a run scored, what it cost, and what produced it.
//!
//! A benchmark number without its provenance cannot be compared against a later one. This
//! records the schema, the binary, the configuration, and whether a model or an embedder was
//! involved, so a change in the number can be attributed to a change in the system rather than
//! to a change in the machine it ran on.
//!
//! Two rules hold here.
//!
//! **Logical results are deterministic.** Everything except timing is a function of the
//! scenario and the code. [`Baseline::logical`] is the part two identical runs must agree on
//! byte for byte, and the part a regression check compares.
//!
//! **Nothing private is reported.** The report carries counts, hashes and stable local names.
//! It never carries a query, a transcript excerpt, or the text of a memory.

use crate::{Scenario, Score, measure};

/// The recall latency budget, declared rather than observed.
///
/// Five milliseconds at the tail. A memory layer sits on the turn path, so this is the number
/// that decides whether it belongs there at all — and declaring it is what makes a regression a
/// failure rather than a slightly larger number nobody looks at.
///
/// Generous on purpose: the point is to catch an algorithmic change that turns a bounded query
/// into an unbounded one, not to police a few hundred microseconds on a busy machine.
pub const RECALL_P95_BUDGET_MS: f64 = 5.0;

/// Everything one evaluation says about itself.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Baseline {
    /// A stable name for this configuration of this run. Identical inputs give identical ids.
    pub run_id: String,
    /// The checkout, when the build was told.
    pub git_revision: String,
    /// The binary.
    pub binary_version: String,
    /// What store shape produced this.
    pub schema_version: u32,
    /// A digest of the settings that were in force.
    pub config_fingerprint: String,
    /// Whether anything but rules produced candidates.
    pub extractor_mode: String,
    /// Whether vectors were available.
    pub embedder_mode: String,
    /// Which scenario.
    pub scenario: String,
    /// What fixed the scenario's shape and clock.
    pub seed: i64,
    /// The fraction of encounters the agent already knew.
    pub task_success: f64,
    /// The best this scenario allows.
    pub ceiling: f64,
    /// The same history in the window, with no memory system at all.
    ///
    /// The control that can lose, and the one the field is emphatic about: a memory system that
    /// does not beat simply carrying the text forward has not earned its latency, its storage or
    /// its failure modes. Published comparisons routinely report the arm memo did not run —
    /// Mem0 at 66.88% against full context's 72.90% on its own benchmark.
    pub in_window: f64,
    /// How many sessions in memo first matches or beats the window.
    ///
    /// `None` while it has not. This is the honest form of the claim: a window wins until the
    /// history outruns it, and the number worth stating is where that stops being true.
    pub crossover: Option<usize>,
    /// The same run with memory switched off.
    pub without_memory: f64,
    /// Rediscoveries that memory could have prevented and did not.
    pub avoidable_failures: usize,
    /// Of what was asserted, how much was right.
    pub recall_precision: f64,
    /// How often anything relevant was findable at all.
    pub recall_relevance: f64,
    /// Of what was asserted, how much was not superseded.
    pub assertion_accuracy: f64,
    /// Estimated tokens handed to a model across the whole run.
    pub injected_tokens: usize,
    /// What the store grew to.
    pub store_bytes: u64,
    /// Median recall.
    pub recall_p50_ms: f64,
    /// The tail that decides whether it is usable on a turn path.
    pub recall_p95_ms: f64,
}

/// The shortest history at which memory catches the window.
///
/// Walked prefix by prefix rather than solved, because the two arms fail for different reasons
/// and the point where they cross is a measurement rather than an inequality. `None` means the
/// window was still ahead at the end of the scenario, which is a result and not an omission.
fn crossover(scenario: &Scenario) -> Option<usize> {
    // Walking every prefix is quadratic in sessions, and a benchmark slow enough to skip is a
    // benchmark nobody runs. If the crossing has not happened in this many sessions it is not
    // the interesting kind of crossing.
    const LOOKED: usize = 16;
    for n in 2..=scenario.sessions.len().min(LOOKED) {
        let prefix = Scenario {
            sessions: scenario.sessions[..n].to_vec(),
            ..scenario.clone()
        };
        let with = crate::run_arm(&prefix, crate::Arm::Memory).hit_rate();
        let window = crate::run_arm(&prefix, crate::Arm::InWindow(crate::WINDOW)).hit_rate();
        if with >= window {
            return Some(n);
        }
    }
    None
}

impl Baseline {
    /// Run `scenario` both ways and report everything about it.
    pub fn of(scenario: &Scenario, name: &str, seed: i64, settings: &memo_lua::Settings) -> Self {
        let (with, cost) = measure(scenario, true);
        let (without, _) = measure(scenario, false);
        let (windowed, _) = crate::measure_arm(scenario, crate::Arm::InWindow(crate::WINDOW));
        // Only worth walking when there is a crossing to find. If the window is still level or
        // ahead over the whole scenario then memory never overtook it here, and searching the
        // prefixes is dozens of runs spent confirming it.
        let crossover = (with.hit_rate() > windowed.hit_rate())
            .then(|| crossover(scenario))
            .flatten();
        let ceiling = Score::ceiling(scenario);

        // What memory could have prevented and did not: every rediscovery after the first
        // encounter of that lesson. The first is nobody's fault — there was nothing to know.
        let unavoidable = scenario.lessons().len();
        let avoidable_failures = with.rediscoveries.saturating_sub(unavoidable);

        let config_fingerprint = fingerprint(settings);
        let schema_version = memo_store::SCHEMA_VERSION;
        let run_id = memo_model::content_hash(&format!(
            "{name}/{seed}/{schema_version}/{config_fingerprint}/{}",
            scenario.sessions.len()
        ))[..16]
            .to_owned();

        Self {
            run_id,
            git_revision: option_env!("MEMO_GIT_REV").unwrap_or("unknown").to_owned(),
            binary_version: env!("CARGO_PKG_VERSION").to_owned(),
            schema_version,
            config_fingerprint,
            // Both are the floor and not a claim about what is installed: the benchmark runs
            // deterministically on purpose, so that a number from it means the same thing on
            // every machine. See the module note in `eval`.
            extractor_mode: "rules".to_owned(),
            embedder_mode: "none".to_owned(),
            scenario: name.to_owned(),
            seed,
            task_success: with.hit_rate(),
            ceiling,
            without_memory: without.hit_rate(),
            in_window: windowed.hit_rate(),
            crossover,
            avoidable_failures,
            recall_precision: cost.recall_precision(),
            recall_relevance: cost.recall_relevance(),
            assertion_accuracy: cost.assertion_accuracy(),
            injected_tokens: cost.injected_tokens,
            store_bytes: cost.store_bytes,
            recall_p50_ms: cost.recall_ms(0.50),
            recall_p95_ms: cost.recall_ms(0.95),
        }
    }

    /// The part of this report that two identical runs must agree on exactly.
    ///
    /// Timing and store size are excluded: one is the machine and the other moves with SQLite's
    /// page allocation. Everything else is a function of the scenario and the code, and a
    /// difference in any of it is a real change rather than noise.
    #[must_use]
    pub fn logical(&self) -> String {
        format!(
            "{}/{}/{}/{}/{:.6}/{:.6}/{:.6}/{}/{:.6}/{:.6}/{:.6}/{}",
            self.run_id,
            self.schema_version,
            self.config_fingerprint,
            self.scenario,
            self.task_success,
            self.ceiling,
            self.without_memory,
            self.avoidable_failures,
            self.recall_precision,
            self.recall_relevance,
            self.assertion_accuracy,
            self.injected_tokens,
        )
    }
}

/// A digest of every setting that changes what a run does.
fn fingerprint(settings: &memo_lua::Settings) -> String {
    settings.fingerprint()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: i64 = 1_756_000_000;

    fn baseline() -> Baseline {
        let scenario = Scenario::one_lesson(6, SEED);
        Baseline::of(
            &scenario,
            "one-lesson",
            SEED,
            &memo_lua::Settings::default(),
        )
    }

    #[test]
    fn two_identical_runs_agree_on_everything_but_the_clock() {
        // F0's whole point. A benchmark whose number moves between two runs of the same code
        // cannot be used to judge a change to the code.
        assert_eq!(baseline().logical(), baseline().logical());
    }

    #[test]
    fn the_report_says_what_produced_it() {
        let held = baseline();
        assert_eq!(held.schema_version, memo_store::SCHEMA_VERSION);
        assert!(!held.config_fingerprint.is_empty());
        assert!(!held.run_id.is_empty());
        assert_eq!(held.extractor_mode, "rules");
        assert_eq!(held.embedder_mode, "none");
    }

    #[test]
    fn a_different_configuration_is_a_different_fingerprint() {
        // Otherwise two runs under different settings would compare as though they were the
        // same experiment.
        let mut changed = memo_lua::Settings::default();
        changed.floors.inject += 0.1;
        assert_ne!(
            fingerprint(&memo_lua::Settings::default()),
            fingerprint(&changed)
        );
    }

    #[test]
    fn a_different_configuration_is_a_different_run() {
        let scenario = Scenario::one_lesson(6, SEED);
        let mut changed = memo_lua::Settings::default();
        changed.floors.inject += 0.1;
        let one = Baseline::of(
            &scenario,
            "one-lesson",
            SEED,
            &memo_lua::Settings::default(),
        );
        let two = Baseline::of(&scenario, "one-lesson", SEED, &changed);
        assert_ne!(one.run_id, two.run_id);
    }

    #[test]
    fn the_report_carries_no_text_from_the_run() {
        // The privacy rule, checked rather than asserted: nothing a session said may appear.
        let scenario = Scenario::one_lesson(6, SEED);
        let held = Baseline::of(
            &scenario,
            "one-lesson",
            SEED,
            &memo_lua::Settings::default(),
        );
        let json = serde_json::to_string(&held).expect("serialize");
        for lesson in scenario.lessons() {
            assert!(!json.contains(&lesson.right), "a command leaked: {json}");
            assert!(!json.contains(&lesson.wrong), "a command leaked: {json}");
        }
        for session in &scenario.sessions {
            assert!(!json.contains(&session.asked), "a question leaked: {json}");
        }
    }

    #[test]
    fn memory_costs_tokens_and_the_report_says_how_many() {
        // A benchmark that reports success without cost can be won by injecting everything.
        let held = baseline();
        assert!(held.injected_tokens > 0, "something was injected");
        assert!(held.store_bytes > 0, "and it took room");
    }

    #[test]
    fn recall_stays_inside_its_declared_budget() {
        // §8.9's last acceptance bullet. Traversal, relationship views and the ledger have all
        // been added to this path since the budget was set; a change that made any of them
        // unbounded would show up here rather than as a slow afternoon.
        let held = baseline();
        assert!(
            held.recall_p95_ms < RECALL_P95_BUDGET_MS,
            "p95 {:.2}ms is over the declared {RECALL_P95_BUDGET_MS}ms budget",
            held.recall_p95_ms
        );
    }

    #[test]
    fn nothing_superseded_is_asserted() {
        // The failure a memory layer adds that a stateless agent cannot have: confidently
        // stating the command that was already corrected. Locked at one so a regression that
        // starts asserting superseded facts shows up as a number rather than as a bug report.
        assert!(
            (baseline().assertion_accuracy - 1.0).abs() < f64::EPSILON,
            "a corrected command was asserted without its correction"
        );
    }

    #[test]
    fn success_is_not_bought_with_an_ever_growing_prompt() {
        // The rejected shortcut, as a test: a benchmark gain purchased by injecting more every
        // session is not a gain. Ten sessions of one lesson must not cost ten times four.
        let short = Baseline::of(
            &Scenario::one_lesson(4, SEED),
            "one-lesson",
            SEED,
            &memo_lua::Settings::default(),
        );
        let long = Baseline::of(
            &Scenario::one_lesson(16, SEED),
            "one-lesson",
            SEED,
            &memo_lua::Settings::default(),
        );
        let per_session_short = short.injected_tokens as f64 / 4.0;
        let per_session_long = long.injected_tokens as f64 / 16.0;
        assert!(
            per_session_long <= per_session_short * 1.5,
            "injection grew per session: {per_session_short:.1} -> {per_session_long:.1}"
        );
    }

    #[test]
    fn what_could_not_have_been_known_is_not_counted_against_memory() {
        // The first encounter of a lesson has nothing to recall from. Counting it as an
        // avoidable failure would make the ceiling unreachable and the metric meaningless.
        let scenario = Scenario::one_lesson(6, SEED);
        let held = Baseline::of(
            &scenario,
            "one-lesson",
            SEED,
            &memo_lua::Settings::default(),
        );
        assert_eq!(held.avoidable_failures, 0, "it learned on the first try");
    }
}
