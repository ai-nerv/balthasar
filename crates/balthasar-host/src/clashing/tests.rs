use super::*;
use balthasar_model::{
    AgentId, Body, ScopeId, Tier, Timestamp, Witness, WitnessId, WitnessKind, floor,
};
use balthasar_store::{Store, Transcript};

/// Hooks that withhold nothing, which is what the shipped configuration does.
struct Plain;
impl crate::Hooks for Plain {}

const NOW: Timestamp = 1_756_000_000;
const SCOPE: &str = "/w/thing";
/// The shipped cadence, which is what these pin.
const EVERY: u32 = 8;

struct Held {
    store: Store,
    scrollback: Transcript,
}

impl Held {
    fn new() -> Self {
        Self {
            store: Store::ephemeral().expect("store"),
            scrollback: Transcript::ephemeral().expect("scrollback"),
        }
    }

    fn at(&mut self) -> Answering<'_> {
        self.at_time(NOW)
    }

    fn at_time(&mut self, now: Timestamp) -> Answering<'_> {
        Answering {
            store: &mut self.store,
            scrollback: Some(&mut self.scrollback),
            scratch: None,
            scope: ScopeId::new(SCOPE),
            agent: AgentId::main(),
            now,
            inject_floor: floor::INJECT,
            live_floor: floor::LIVE,
            capture: false,
        }
    }

    /// A believed claim, stated once by a person.
    fn claim(&mut self, slot: &str, value: &str) -> MemoryId {
        self.claim_by(slot, value, WitnessKind::Imperative)
    }

    fn claim_by(&mut self, slot: &str, value: &str, kind: WitnessKind) -> MemoryId {
        let memory = Memory::new(
            MemoryId::minted(NOW as u64, rand()),
            Tier::Fact,
            ScopeId::new(SCOPE),
            Body::fact("project", slot, value),
            NOW,
        );
        let witness = Witness::new(
            WitnessId::new(format!("w-{slot}-{value}")),
            kind,
            SessionId::new("01RUN"),
            ScopeId::new(SCOPE),
            NOW,
        );
        self.store
            .remember(memory, witness, NOW)
            .expect("remember")
            .id()
            .clone()
    }

    fn confidence(&self, id: &MemoryId) -> f64 {
        self.store
            .get(id)
            .expect("read")
            .expect("memory")
            .confidence
    }
}

/// Distinct ids without a clock that moves.
fn rand() -> u128 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    u128::from(NEXT.fetch_add(1, Ordering::Relaxed))
}

fn pairs(said: &[(&MemoryId, &MemoryId)]) -> String {
    let rows: Vec<Value> = said
        .iter()
        .map(
            |(a, b)| json!({ "a": a.to_string(), "b": b.to_string(), "why": "a says X, b says Y" }),
        )
        .collect();
    json!({ "pairs": rows }).to_string()
}

/// Two claims that disagree without sharing a slot — which is the whole case for this. Two
/// values of one slot supersede each other on the way in and never reach a model.
fn disagreeing(held: &mut Held) -> (MemoryId, MemoryId) {
    (
        held.claim("how_tests_run", "on CI"),
        held.claim("test_hardware", "a GPU every developer lacks"),
    )
}

#[test]
fn a_found_pair_makes_both_sides_less_sure() {
    let mut held = Held::new();
    let (a, b) = disagreeing(&mut held);
    let (was_a, was_b) = (held.confidence(&a), held.confidence(&b));
    assert!(was_a > 0.0 && was_b > 0.0);
    let said = pairs(&[(&a, &b)]);
    assert!(settle(&mut held.at(), &said).expect("settle"));
    assert!(
        held.confidence(&a) < was_a,
        "{} should have fallen from {was_a}",
        held.confidence(&a)
    );
    assert!(held.confidence(&b) < was_b);
}

#[test]
fn the_edge_is_drawn_both_ways() {
    let mut held = Held::new();
    let (a, b) = disagreeing(&mut held);
    settle(&mut held.at(), &pairs(&[(&a, &b)])).expect("settle");
    // Each one's own rescore has to find the other pulling against it, which only works when
    // the link was written in both directions.
    for id in [&a, &b] {
        let before = held.confidence(id);
        let after = held.store.rescore(id, NOW).expect("rescore");
        assert!((after - before).abs() < 1e-9, "{id}: {before} then {after}");
        assert!(after < 1.0);
    }
}

/// The whole reason a model may draw these edges at all. It does not say how much the edge
/// weighs — the claim on the other end of it does, out of its own evidence. So a guess pulling
/// against something a person stated barely moves it, and could never take it below the floor
/// at which it stops being asserted.
#[test]
fn a_guess_pulls_far_less_than_a_person_does() {
    let mut held = Held::new();
    let stated = held.claim("how_tests_run", "on CI");
    let was = held.confidence(&stated);
    let guess = held.claim_by("test_hardware", "a GPU", WitnessKind::Inferred);
    assert!(held.confidence(&guess) < was, "a guess is worth less");
    settle(&mut held.at(), &pairs(&[(&stated, &guess)])).expect("settle");
    let by_a_guess = was - held.confidence(&stated);

    let mut held = Held::new();
    let stated = held.claim("how_tests_run", "on CI");
    let said = held.claim("test_hardware", "a GPU");
    settle(&mut held.at(), &pairs(&[(&stated, &said)])).expect("settle");
    let by_a_person = was - held.confidence(&stated);

    assert!(
        by_a_guess < by_a_person,
        "a guess moved {by_a_guess} and a person {by_a_person}"
    );
    assert!(by_a_guess > 0.0, "it is still evidence");
}

/// The person's word is the end of it. A model that goes on proposing a pair somebody has
/// already looked at and dismissed is overruled every time, and says nothing about it.
#[test]
fn a_pair_a_person_has_settled_is_not_raised_again() {
    let mut held = Held::new();
    let (a, b) = disagreeing(&mut held);
    let said = pairs(&[(&a, &b)]);
    settle(&mut held.at(), &said).expect("settle");
    held.store.reconcile(&a, &b, NOW).expect("reconcile");
    let (was_a, was_b) = (held.confidence(&a), held.confidence(&b));

    settle(&mut held.at(), &said).expect("settle again");

    assert!((held.confidence(&a) - was_a).abs() < 1e-9, "a moved");
    assert!((held.confidence(&b) - was_b).abs() < 1e-9, "b moved");
    let held_a = held.store.get(&a).expect("read").expect("memory");
    assert!(
        held_a
            .links
            .iter()
            .any(|link| link.rel == LinkRelation::Reconciled),
        "the person's word was overwritten"
    );
}

#[test]
fn an_id_the_model_invented_is_ignored() {
    let mut held = Held::new();
    let a = held.claim("runner", "CI");
    let was = held.confidence(&a);
    let made_up = MemoryId::new("M-NOTHING");
    assert!(settle(&mut held.at(), &pairs(&[(&a, &made_up)])).expect("settle"));
    assert!((held.confidence(&a) - was).abs() < 1e-9, "nothing pulled");
}

#[test]
fn a_claim_cannot_contradict_itself() {
    let mut held = Held::new();
    let a = held.claim("runner", "CI");
    let was = held.confidence(&a);
    settle(&mut held.at(), &pairs(&[(&a, &a)])).expect("settle");
    assert!((held.confidence(&a) - was).abs() < 1e-9);
}

#[test]
fn another_projects_claim_is_out_of_reach() {
    let mut held = Held::new();
    let a = held.claim("runner", "CI");
    let was = held.confidence(&a);
    let elsewhere = Memory::new(
        MemoryId::minted(NOW as u64, rand()),
        Tier::Fact,
        ScopeId::new("/w/other"),
        Body::fact("project", "runner", "a GPU"),
        NOW,
    );
    let id = elsewhere.id.clone();
    held.store
        .remember(
            elsewhere,
            Witness::new(
                WitnessId::new("w-elsewhere"),
                WitnessKind::Imperative,
                SessionId::new("01RUN"),
                ScopeId::new("/w/other"),
                NOW,
            ),
            NOW,
        )
        .expect("remember");
    settle(&mut held.at(), &pairs(&[(&a, &id)])).expect("settle");
    assert!((held.confidence(&a) - was).abs() < 1e-9);
}

#[test]
fn only_so_many_pairs_are_taken_from_one_answer() {
    let mut held = Held::new();
    let offered = AT_MOST + 3;
    let made: Vec<(MemoryId, MemoryId)> = (0..offered)
        .map(|n| {
            (
                held.claim(&format!("left{n}"), "value"),
                held.claim(&format!("right{n}"), "another value"),
            )
        })
        .collect();
    let all: Vec<(&MemoryId, &MemoryId)> = made.iter().map(|(a, b)| (a, b)).collect();
    let untouched = held.confidence(&made[0].0);
    settle(&mut held.at(), &pairs(&all)).expect("settle");
    let pulled = made
        .iter()
        .filter(|(a, b)| held.confidence(a) < untouched && held.confidence(b) < untouched)
        .count();
    assert_eq!(pulled, AT_MOST);
    let (a, b) = made.last().expect("last");
    for id in [a, b] {
        assert!(
            (held.confidence(id) - untouched).abs() < 1e-9,
            "the pairs past the limit were left alone"
        );
    }
}

#[test]
fn chatter_without_json_is_not_an_answer() {
    let mut held = Held::new();
    assert!(!settle(&mut held.at(), "I could not find any.").expect("settle"));
    assert!(settle(&mut held.at(), "{\"pairs\": []}").expect("settle"));
}

#[test]
fn a_harness_that_cannot_run_the_role_is_asked_for_nothing() {
    let mut held = Held::new();
    for n in 0..EVERY {
        held.claim(&format!("slot{n}"), "value");
    }
    let session = SessionId::new("01RUN");
    let at = held.at();
    for _ in 0..(EVERY + 2) {
        queue(&at, &session, &["notes".to_owned()], EVERY, &mut Plain).expect("queue");
    }
    assert!(
        at.scrollback
            .as_ref()
            .expect("scrollback")
            .jobs_of(&session)
            .expect("jobs")
            .is_empty()
    );
}

#[test]
fn a_sweep_waits_for_its_turn_and_then_runs_once() {
    let mut held = Held::new();
    for n in 0..EVERY {
        held.claim(&format!("slot{n}"), "value");
    }
    let session = SessionId::new("01RUN");
    let at = held.at();
    let roles = ["contradict".to_owned()];
    let queued = |at: &Answering<'_>| {
        at.scrollback
            .as_ref()
            .expect("scrollback")
            .jobs_of(&session)
            .expect("jobs")
            .len()
    };
    queue(&at, &session, &roles, EVERY, &mut Plain).expect("queue");
    assert_eq!(queued(&at), 1, "six claims and nothing swept yet");
    // Still on its way, and then nothing new to look at.
    for _ in 0..(EVERY * 2) {
        queue(&at, &session, &roles, EVERY, &mut Plain).expect("queue");
    }
    assert_eq!(queued(&at), 1, "one sweep at a time, over a set that stood");
}

#[test]
fn a_set_that_has_not_grown_enough_waits_however_often_it_is_asked() {
    // What used to decide this was how many rounds had gone by, so a harness that laid out or
    // ran helpers more often swept more often over the very same claims.
    let mut held = Held::new();
    for n in 0..EVERY {
        held.claim(&format!("slot{n}"), "value");
    }
    let session = SessionId::new("01RUN");
    let roles = ["contradict".to_owned()];
    {
        let at = held.at();
        queue(&at, &session, &roles, EVERY, &mut Plain).expect("queue");
    }
    held.claim("one-more", "value");
    let later = NOW + crate::cadence::APART;
    let at = held.at_time(later);
    for _ in 0..50 {
        queue(&at, &session, &roles, EVERY, &mut Plain).expect("queue");
    }
    let jobs = at
        .scrollback
        .as_ref()
        .expect("scrollback")
        .jobs_of(&session)
        .expect("jobs");
    assert_eq!(jobs.len(), 1, "one new claim is not four");
}

#[test]
fn enough_new_claims_bring_the_next_sweep() {
    let mut held = Held::new();
    for n in 0..EVERY {
        held.claim(&format!("slot{n}"), "value");
    }
    let session = SessionId::new("01RUN");
    let roles = ["contradict".to_owned()];
    {
        let at = held.at();
        queue(&at, &session, &roles, EVERY, &mut Plain).expect("queue");
        let scrollback = at.scrollback.as_ref().expect("scrollback");
        let first = scrollback.jobs_of(&session).expect("jobs").remove(0);
        scrollback
            .settle_job(&first.id, balthasar_store::JobState::Done, None, NOW)
            .expect("settle");
    }
    for n in 0..EVERY {
        held.claim(&format!("later{n}"), "value");
    }
    let later = NOW + crate::cadence::APART;
    let at = held.at_time(later);
    queue(&at, &session, &roles, EVERY, &mut Plain).expect("queue");
    let jobs = at
        .scrollback
        .as_ref()
        .expect("scrollback")
        .jobs_of(&session)
        .expect("jobs");
    assert_eq!(jobs.len(), 2, "four more claims is another sweep");
}

#[test]
fn too_few_claims_are_nothing_to_disagree_about() {
    let mut held = Held::new();
    held.claim("runner", "CI");
    let session = SessionId::new("01RUN");
    let at = held.at();
    for _ in 0..(EVERY + 1) {
        queue(&at, &session, &["contradict".to_owned()], EVERY, &mut Plain).expect("queue");
    }
    assert!(
        at.scrollback
            .as_ref()
            .expect("scrollback")
            .jobs_of(&session)
            .expect("jobs")
            .is_empty()
    );
}

/// Every prompt a helper is given passes the same redaction, the contradiction sweep included.
mod mem_all_helpers_apply_privacy {
    use super::*;

    /// Withholds anything holding the sentinel, as a configuration restricting a tier would.
    struct Withholding;

    const SENTINEL: &str = "SENTINEL-LOCAL-ONLY";

    impl crate::Hooks for Withholding {
        fn redact(&mut self, text: &str, _memory: &balthasar_model::Memory) -> Option<String> {
            (!text.contains(SENTINEL)).then(|| text.to_owned())
        }
    }

    /// The input of the sweep queued for `session`, if one was.
    fn swept(held: &mut Held, session: &SessionId, hooks: &mut dyn crate::Hooks) -> Option<String> {
        let at = held.at();
        let roles = ["contradict".to_owned()];
        queue(&at, session, &roles, EVERY, hooks).expect("queue");
        at.scrollback
            .as_ref()
            .expect("scrollback")
            .jobs_of(session)
            .expect("jobs")
            .into_iter()
            .find(|job| job.kind == "contradict")
            .map(|job| job.spec["input"].as_str().unwrap_or_default().to_owned())
    }

    #[test]
    fn a_withheld_claim_is_left_out_of_the_sweeps_prompt() {
        // Every other way out of here redacts. A sweep that did not was a way round it.
        let mut held = Held::new();
        for n in 0..EVERY {
            held.claim(&format!("slot{n}"), "value");
        }
        held.claim("secret", SENTINEL);
        let input = swept(&mut held, &SessionId::new("01RUN"), &mut Withholding).expect("a sweep");
        assert!(
            !input.contains(SENTINEL),
            "a withheld claim reached the model: {input}"
        );
        assert!(input.contains("slot0"), "the rest still went: {input}");
    }

    #[test]
    fn what_is_not_withheld_still_reaches_it() {
        let mut held = Held::new();
        for n in 0..EVERY {
            held.claim(&format!("slot{n}"), "value");
        }
        let input = swept(&mut held, &SessionId::new("01RUN"), &mut Plain).expect("a sweep");
        for n in 0..6 {
            assert!(input.contains(&format!("slot{n}")), "{input}");
        }
    }

    #[test]
    fn a_redaction_that_empties_a_claim_drops_it_rather_than_naming_it() {
        // An id with nothing against it tells the model a claim exists and nothing more, which
        // is a claim it cannot judge and a leak of the fact.
        struct Emptying;
        impl crate::Hooks for Emptying {
            fn redact(&mut self, _text: &str, _m: &balthasar_model::Memory) -> Option<String> {
                Some(String::new())
            }
        }
        let mut held = Held::new();
        for n in 0..EVERY {
            held.claim(&format!("slot{n}"), "value");
        }
        assert!(
            swept(&mut held, &SessionId::new("01RUN"), &mut Emptying).is_none(),
            "nothing was left to sweep, so nothing should have been queued"
        );
    }
}
