//! The number that would make somebody change what they run.

use aeon_testkit::{Scenario, Score, run};

const START: aeon_model::Timestamp = 1_756_000_000;

#[test]
fn without_memory_every_session_rediscovers_everything() {
    // The baseline every harness ships today. Ten sessions, one lesson, learned ten times.
    let scenario = Scenario::one_lesson(10, START);
    let score = run(&scenario, false);
    assert_eq!(score.recalls, 0);
    assert_eq!(score.rediscoveries, 10);
    assert_eq!(score.hit_rate(), 0.0);
}

#[test]
fn with_memory_a_lesson_is_learned_once() {
    // The whole claim, as a number. The first session has nothing to know from; every one
    // after it should start knowing.
    let scenario = Scenario::one_lesson(10, START);
    let score = run(&scenario, true);

    assert_eq!(
        score.sessions[0].rediscovered.len(),
        1,
        "the first session had nothing to go on"
    );
    assert!(
        score.hit_rate() > 0.5,
        "hit rate {:.2}, ceiling {:.2}",
        score.hit_rate(),
        Score::ceiling(&scenario)
    );
}

#[test]
fn memory_beats_no_memory_on_the_same_scenario() {
    let scenario = Scenario::one_lesson(10, START);
    let with = run(&scenario, true);
    let without = run(&scenario, false);
    assert!(
        with.hit_rate() > without.hit_rate(),
        "{:.2} vs {:.2}",
        with.hit_rate(),
        without.hit_rate()
    );
}

#[test]
fn the_second_session_already_knows() {
    // The canonical demo, asserted. A session that discovers `make test` after `cargo test`
    // fails should have the next one start with it.
    let scenario = Scenario::one_lesson(3, START);
    let score = run(&scenario, true);
    assert!(
        score.sessions[1].knew.contains(&"run the tests".to_owned()),
        "session 2 rediscovered: {:?}",
        score.sessions[1]
    );
}

#[test]
fn several_lessons_met_in_different_orders_are_all_kept() {
    // A memory layer that remembered the *last* thing rather than the *relevant* thing scores
    // well on one lesson and badly here.
    let scenario = Scenario::several_lessons(9, START);
    let score = run(&scenario, true);
    assert!(
        score.hit_rate() > 0.5,
        "hit rate {:.2}, ceiling {:.2}",
        score.hit_rate(),
        Score::ceiling(&scenario)
    );
}

#[test]
fn the_ceiling_is_reported_beside_the_score() {
    // A hit rate of 0.8 means nothing until you know whether 0.83 was the best available.
    let scenario = Scenario::one_lesson(10, START);
    let ceiling = Score::ceiling(&scenario);
    assert!((ceiling - 0.9).abs() < 0.001, "{ceiling}");
    assert!(run(&scenario, true).hit_rate() <= ceiling + 1e-9);
}

#[test]
fn a_scenario_of_one_session_has_nothing_to_measure() {
    // The first session of a project has nothing to know from, so a perfect memory layer
    // scores zero here. Worth asserting so nobody reads it as a failure.
    let scenario = Scenario::one_lesson(1, START);
    assert_eq!(Score::ceiling(&scenario), 0.0);
    assert_eq!(run(&scenario, true).hit_rate(), 0.0);
}

#[test]
fn a_score_is_reproducible() {
    // Nothing here depends on a clock, a model, a network or an embedder, so a number from
    // this suite means the same thing on every machine that runs it.
    let scenario = Scenario::several_lessons(6, START);
    assert_eq!(run(&scenario, true), run(&scenario, true));
}
