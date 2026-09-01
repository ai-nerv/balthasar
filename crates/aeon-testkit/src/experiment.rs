//! An experiment, and what it takes to adopt one.
//!
//! A benchmark number on its own is an anecdote. What makes it evidence is that somebody wrote
//! down what they expected *before* running it, named the metrics that would make them stop, and
//! recorded enough about the machinery that a later run means the same thing.
//!
//! The guardrails are the part that matters. Almost any retrieval change can be made to win on
//! one number — usually by injecting more — so a manifest that named only a primary metric would
//! reward exactly the changes that should be rejected.

use crate::Baseline;

/// What a comparison is testing, recorded before it runs.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Manifest {
    /// What it is called.
    pub name: String,
    /// What is expected, in a sentence, written first.
    pub hypothesis: String,
    /// The number this is trying to move.
    pub primary: Metric,
    /// The numbers that must not get worse while it moves.
    pub guardrails: Vec<Metric>,
    /// What it is being compared against.
    pub baseline_policy: String,
    /// What is being tried.
    pub policy: String,
    /// Which scenario, and how many sessions.
    pub scenario: String,
    /// What fixed the clock.
    pub seed: i64,
    /// The settings in force.
    pub config: String,
    /// The store shape.
    pub schema: u32,
    /// The checkout.
    pub revision: String,
    /// What would make this stop.
    pub stop_when: String,
}

/// One thing being measured, and which direction is better.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    /// The fraction of encounters the agent already knew. Higher is better.
    TaskSuccess,
    /// Of what was asserted, how much was right. Higher is better.
    RecallPrecision,
    /// How often anything relevant was findable. Higher is better.
    RecallRelevance,
    /// Of what was asserted, how much was not superseded. Higher is better.
    AssertionAccuracy,
    /// Rediscoveries memory could have prevented. Lower is better.
    AvoidableFailures,
    /// Tokens handed to a model. Lower is better.
    InjectedTokens,
    /// What the store grew to. Lower is better.
    StoreBytes,
    /// The tail latency that decides whether this belongs on a turn path. Lower is better.
    RecallP95,
    /// The share of attacks that reached assertion. Lower is better, and the only acceptable
    /// value is zero.
    AttackSuccess,
}

impl Metric {
    /// Whether a larger number is a better one.
    #[must_use]
    pub fn higher_is_better(self) -> bool {
        matches!(
            self,
            Self::TaskSuccess
                | Self::RecallPrecision
                | Self::RecallRelevance
                | Self::AssertionAccuracy
        )
    }

    /// Read this metric off a baseline.
    #[must_use]
    pub fn of(self, held: &Baseline, attack_success: f64) -> f64 {
        match self {
            Self::TaskSuccess => held.task_success,
            Self::RecallPrecision => held.recall_precision,
            Self::RecallRelevance => held.recall_relevance,
            Self::AssertionAccuracy => held.assertion_accuracy,
            Self::AvoidableFailures => held.avoidable_failures as f64,
            Self::InjectedTokens => held.injected_tokens as f64,
            Self::StoreBytes => held.store_bytes as f64,
            Self::RecallP95 => held.recall_p95_ms,
            Self::AttackSuccess => attack_success,
        }
    }

    /// The word this is spelled with.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TaskSuccess => "task-success",
            Self::RecallPrecision => "recall-precision",
            Self::RecallRelevance => "recall-relevance",
            Self::AssertionAccuracy => "assertion-accuracy",
            Self::AvoidableFailures => "avoidable-failures",
            Self::InjectedTokens => "injected-tokens",
            Self::StoreBytes => "store-bytes",
            Self::RecallP95 => "recall-p95",
            Self::AttackSuccess => "attack-success",
        }
    }
}

/// What to do about an experiment, having run it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// It moved the primary metric and broke no guardrail.
    Adopt,
    /// It moved the primary metric and cost something. Worth another look, not a merge.
    Revise,
    /// It did not move the primary metric, or it broke a guardrail that cannot be traded.
    Reject,
}

/// How one metric came out.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Moved {
    /// Which.
    pub metric: Metric,
    /// Before.
    pub was: f64,
    /// After.
    pub now: f64,
}

impl Moved {
    /// Whether this went the way it should.
    #[must_use]
    pub fn improved(&self) -> bool {
        if self.metric.higher_is_better() {
            self.now > self.was
        } else {
            self.now < self.was
        }
    }

    /// Whether this got materially worse.
    ///
    /// A tolerance, because every measurement moves a little and a guardrail that fired on
    /// noise would reject everything. Five per cent of the baseline, except for safety.
    #[must_use]
    pub fn regressed(&self) -> bool {
        if self.metric == Metric::AttackSuccess {
            // No tolerance at all. An attack that got through is not noise.
            return self.now > self.was;
        }
        let tolerance = self.was.abs() * 0.05;
        if self.metric.higher_is_better() {
            self.now < self.was - tolerance
        } else {
            self.now > self.was + tolerance
        }
    }
}

/// A finished comparison.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Outcome {
    /// What was being tested.
    pub manifest: Manifest,
    /// How the primary metric moved.
    pub primary: Moved,
    /// How each guardrail moved.
    pub guardrails: Vec<Moved>,
    /// What to do about it.
    pub decision: Decision,
}

impl Outcome {
    /// Judge a comparison against its own manifest.
    ///
    /// Deliberately mechanical. The judgment is made from what was written down beforehand,
    /// which is the only arrangement in which a disappointing result cannot be reinterpreted
    /// into a success afterwards.
    #[must_use]
    pub fn judge(manifest: Manifest, primary: Moved, guardrails: Vec<Moved>) -> Self {
        let broke_safety = guardrails
            .iter()
            .any(|g| g.metric == Metric::AttackSuccess && g.regressed());
        let broke_something = guardrails.iter().any(Moved::regressed);

        let decision = if broke_safety || !primary.improved() {
            // Safety is not tradeable and a primary that did not move is not a result.
            Decision::Reject
        } else if broke_something {
            Decision::Revise
        } else {
            Decision::Adopt
        };

        Self {
            manifest,
            primary,
            guardrails,
            decision,
        }
    }
}

/// The four metric groups, as one report.
///
/// Correctness, agent outcomes, efficiency and safety together, because each of them can be
/// won at the expense of the others: a system that asserts nothing has perfect safety, one
/// that asserts everything has perfect recall, and one that injects the whole store has the
/// best task success anybody has ever measured.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Full {
    /// What produced this, and what it scored.
    pub baseline: Baseline,
    /// How many attacks reached assertion. Zero, or the rest does not matter.
    pub attack_success_rate: f64,
    /// How many attacks were run.
    pub attacks: usize,
    /// Every scenario probe, and how many passed.
    pub scenarios_passed: usize,
    /// Out of how many.
    pub scenarios: usize,
}

impl Full {
    /// Run everything.
    #[must_use]
    pub fn measure(scenario: &crate::Scenario, name: &str, seed: i64) -> Self {
        let settings = aeon_lua::Settings::default();
        let attacks = crate::run_attacks();
        let suite = crate::run_suite();
        Self {
            baseline: Baseline::of(scenario, name, seed, &settings),
            attack_success_rate: attacks.attack_success_rate(),
            attacks: attacks.verdicts.len(),
            scenarios_passed: suite.probes - suite.broke.iter().map(|(_, n)| n).sum::<usize>(),
            scenarios: suite.probes,
        }
    }

    /// Whether every group is where it should be.
    ///
    /// Not a score. Four separate questions, and the answer is no if any of them is no —
    /// which is the whole reason they are not averaged into one number.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.attack_success_rate == 0.0
            && self.scenarios_passed == self.scenarios
            && self.baseline.avoidable_failures == 0
            && self.baseline.assertion_accuracy >= 1.0
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        Manifest {
            name: "entity-traversal".to_owned(),
            hypothesis: "walking entity edges finds things search alone misses".to_owned(),
            primary: Metric::TaskSuccess,
            guardrails: vec![
                Metric::InjectedTokens,
                Metric::RecallP95,
                Metric::AttackSuccess,
            ],
            baseline_policy: "lexical-only".to_owned(),
            policy: "entity".to_owned(),
            scenario: "one-lesson/10".to_owned(),
            seed: 1_756_000_000,
            config: "cfg".to_owned(),
            schema: 5,
            revision: "abc1234".to_owned(),
            stop_when: "injected tokens grow more than 5%".to_owned(),
        }
    }

    fn moved(metric: Metric, was: f64, now: f64) -> Moved {
        Moved { metric, was, now }
    }

    #[test]
    fn the_four_groups_are_reported_together() {
        // Each of them can be won at the expense of the others, so a report showing one is a
        // report that can be gamed.
        let held = Full::measure(
            &crate::Scenario::one_lesson(10, 1_756_000_000),
            "one-lesson",
            1_756_000_000,
        );
        assert!(held.attacks > 0, "safety ran");
        assert!(held.scenarios > 0, "correctness ran");
        assert!(held.baseline.injected_tokens > 0, "efficiency was measured");
        assert!(held.baseline.task_success > 0.0, "outcomes were measured");
    }

    #[test]
    fn everything_is_currently_clean() {
        // The one assertion that would catch a regression in any of the four at once.
        let held = Full::measure(
            &crate::Scenario::one_lesson(10, 1_756_000_000),
            "one-lesson",
            1_756_000_000,
        );
        assert!(
            held.is_clean(),
            "attacks {:.2}, scenarios {}/{}, avoidable {}, accuracy {:.2}",
            held.attack_success_rate,
            held.scenarios_passed,
            held.scenarios,
            held.baseline.avoidable_failures,
            held.baseline.assertion_accuracy
        );
    }

    #[test]
    fn no_group_alone_makes_a_report_clean() {
        // A system that asserts nothing has perfect safety. The conjunction is what stops that
        // reading as success.
        let mut held = Full::measure(
            &crate::Scenario::one_lesson(10, 1_756_000_000),
            "one-lesson",
            1_756_000_000,
        );
        assert!(held.is_clean());
        held.attack_success_rate = 0.1;
        assert!(!held.is_clean(), "safety alone was ignored");
    }

    #[test]
    fn a_clean_win_is_adopted() {
        let held = Outcome::judge(
            manifest(),
            moved(Metric::TaskSuccess, 0.7, 0.9),
            vec![
                moved(Metric::InjectedTokens, 120.0, 122.0),
                moved(Metric::AttackSuccess, 0.0, 0.0),
            ],
        );
        assert_eq!(held.decision, Decision::Adopt);
    }

    #[test]
    fn a_win_bought_with_context_is_not_adopted() {
        // The rejected shortcut, mechanised: almost any retrieval change can win on one number
        // by injecting more, so the token count is a guardrail rather than a footnote.
        let held = Outcome::judge(
            manifest(),
            moved(Metric::TaskSuccess, 0.7, 0.95),
            vec![moved(Metric::InjectedTokens, 120.0, 900.0)],
        );
        assert_eq!(held.decision, Decision::Revise);
    }

    #[test]
    fn safety_is_not_tradeable_for_anything() {
        // A change that lets one attack through is rejected however well it scores, and there
        // is no tolerance band — an attack getting through is not measurement noise.
        let held = Outcome::judge(
            manifest(),
            moved(Metric::TaskSuccess, 0.7, 0.99),
            vec![moved(Metric::AttackSuccess, 0.0, 0.1)],
        );
        assert_eq!(held.decision, Decision::Reject);
    }

    #[test]
    fn a_primary_that_did_not_move_is_not_a_result() {
        let held = Outcome::judge(
            manifest(),
            moved(Metric::TaskSuccess, 0.9, 0.9),
            vec![moved(Metric::InjectedTokens, 120.0, 100.0)],
        );
        assert_eq!(held.decision, Decision::Reject);
    }

    #[test]
    fn small_movements_are_noise_rather_than_regressions() {
        // A guardrail that fired on every wobble would reject everything, and a project that
        // rejects everything stops measuring.
        assert!(!moved(Metric::InjectedTokens, 100.0, 103.0).regressed());
        assert!(moved(Metric::InjectedTokens, 100.0, 140.0).regressed());
    }

    #[test]
    fn direction_is_a_property_of_the_metric() {
        assert!(Metric::TaskSuccess.higher_is_better());
        assert!(!Metric::InjectedTokens.higher_is_better());
        assert!(!Metric::AttackSuccess.higher_is_better());
        assert!(moved(Metric::AvoidableFailures, 4.0, 1.0).improved());
        assert!(moved(Metric::TaskSuccess, 0.5, 0.8).improved());
    }

    #[test]
    fn a_manifest_records_what_would_make_it_comparable_later() {
        // Schema, revision, config and seed. A result missing any of them cannot be set beside
        // a later one, which makes it an anecdote.
        let held = manifest();
        let json = serde_json::to_string(&held).expect("serialize");
        for field in [
            "schema",
            "revision",
            "config",
            "seed",
            "hypothesis",
            "stop_when",
        ] {
            assert!(json.contains(field), "{field} is missing from {json}");
        }
    }

    #[test]
    fn every_metric_can_be_read_off_a_baseline() {
        let baseline = Baseline::of(
            &crate::Scenario::one_lesson(4, 1_756_000_000),
            "one-lesson",
            1_756_000_000,
            &aeon_lua::Settings::default(),
        );
        for metric in [
            Metric::TaskSuccess,
            Metric::RecallPrecision,
            Metric::RecallRelevance,
            Metric::AssertionAccuracy,
            Metric::AvoidableFailures,
            Metric::InjectedTokens,
            Metric::StoreBytes,
            Metric::RecallP95,
            Metric::AttackSuccess,
        ] {
            let value = metric.of(&baseline, 0.0);
            assert!(value.is_finite(), "{metric:?} was not a number");
        }
    }
}
