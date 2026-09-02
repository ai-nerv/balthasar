//! Running a scenario, with memory and without, and comparing.
//!
//! The measurement is not "does recall find the thing". It is **does session k+1 avoid the
//! mistake session k already made** — which is the only question a person using a coding agent
//! actually has, and the one no published benchmark asks.

use crate::{Lesson, Scenario};
use memo_distil::{Observation, Role, consolidate, extract};
use memo_lua::Settings;
use memo_model::{
    Body, Memory, NoteKind, ScopeId, SessionId, Tier, Timestamp, Witness, WitnessId, floor,
};
use memo_store::{Store, mint};

/// What one session did.
#[derive(Debug, Clone, PartialEq)]
pub struct Ran {
    /// Which run.
    pub session: String,
    /// Lessons it had to learn the hard way, having no memory of them.
    pub rediscovered: Vec<String>,
    /// Lessons it started already knowing.
    pub knew: Vec<String>,
}

/// How a whole scenario went.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Score {
    /// Every session, in order.
    pub sessions: Vec<Ran>,
    /// How many lessons were learned the hard way, across every session.
    pub rediscoveries: usize,
    /// How many were already known.
    pub recalls: usize,
}

impl Score {
    /// The fraction of encounters where the agent already knew.
    ///
    /// The number the whole system is for. Zero is a stateless agent; one is impossible,
    /// because the first session of a project has nothing to know from.
    #[must_use]
    pub fn hit_rate(&self) -> f64 {
        let total = self.rediscoveries + self.recalls;
        if total == 0 {
            return 0.0;
        }
        self.recalls as f64 / total as f64
    }

    /// The best any memory layer could do on this scenario.
    ///
    /// Every encounter after the first of each lesson. Reported beside the score, because a
    /// hit rate of 0.8 means nothing until you know whether 0.83 was the ceiling.
    #[must_use]
    pub fn ceiling(scenario: &Scenario) -> f64 {
        let total: usize = scenario.sessions.iter().map(|s| s.lessons.len()).sum();
        let first_time = scenario.lessons().len();
        if total == 0 {
            return 0.0;
        }
        (total - first_time) as f64 / total as f64
    }
}

/// What a run cost and how well it retrieved, beside whether it succeeded.
///
/// Separate from [`Score`] because they answer different questions. `Score` is whether the
/// agent stopped rediscovering things; this is what that took — how much was injected, how
/// precise retrieval was, how long it took, how big the store got. A benchmark that reports
/// only the first can be won by injecting everything.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Measured {
    /// Memories that crossed the injection floor and were the lesson being asked about.
    pub asserted_right: usize,
    /// Memories that crossed the injection floor and were not.
    pub asserted_wrong: usize,
    /// Encounters where something relevant was findable, whether or not it was asserted.
    pub relevant_found: usize,
    /// Assertions naming the command that had already failed, without naming its replacement.
    pub asserted_stale: usize,
    /// Encounters in total.
    pub encounters: usize,
    /// Estimated tokens a harness would have been handed across the whole run.
    pub injected_tokens: usize,
    /// How long each recall took, in microseconds, in the order they happened.
    pub recall_us: Vec<u64>,
    /// What the store grew to.
    pub store_bytes: u64,
}

impl Measured {
    /// Of what was asserted, how much was right.
    ///
    /// Zero assertions is not perfect precision — it is no answer, and reported as zero so a
    /// system that asserts nothing cannot win on this axis.
    #[must_use]
    pub fn recall_precision(&self) -> f64 {
        let asserted = self.asserted_right + self.asserted_wrong;
        if asserted == 0 {
            return 0.0;
        }
        self.asserted_right as f64 / asserted as f64
    }

    /// How often anything relevant was findable at all, asserted or not.
    ///
    /// The gap between this and precision is the cost of the assertion floor: memories that
    /// were there and were right, but had not earned the right to be stated.
    #[must_use]
    pub fn recall_relevance(&self) -> f64 {
        if self.encounters == 0 {
            return 0.0;
        }
        self.relevant_found as f64 / self.encounters as f64
    }

    /// Of what was asserted, how much was not superseded.
    ///
    /// Distinct from precision: a memory can be about the right subject and still name the
    /// command that has since been corrected. That is the failure mode a memory layer adds.
    #[must_use]
    pub fn assertion_accuracy(&self) -> f64 {
        let asserted = self.asserted_right + self.asserted_wrong;
        if asserted == 0 {
            return 1.0;
        }
        (asserted - self.asserted_stale) as f64 / asserted as f64
    }

    /// The nth percentile recall, in milliseconds.
    #[must_use]
    pub fn recall_ms(&self, percentile: f64) -> f64 {
        if self.recall_us.is_empty() {
            return 0.0;
        }
        let mut sorted = self.recall_us.clone();
        sorted.sort_unstable();
        let at = ((sorted.len() - 1) as f64 * percentile.clamp(0.0, 1.0)).round() as usize;
        sorted[at] as f64 / 1000.0
    }
}

/// Which arm of the benchmark is being run.
///
/// Three, and the third is the one that matters. `Nothing` is the baseline every harness ships
/// today and it cannot win — it starts blank every session and scores zero by construction.
/// `InWindow` is the arm the field insists on and the one memo has never run against itself:
/// the same history, carried forward in the window, with no memory system at all. It is the
/// control that can lose, and in the published comparisons it frequently wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arm {
    /// memo, as it ships.
    Memory,
    /// Nothing carried between sessions.
    Nothing,
    /// Every earlier session's text carried forward, truncated to a token budget.
    InWindow(usize),
}

/// How much window the in-window arm gets.
///
/// A budget rather than everything, because "put the whole history in" stops being an option at
/// some length and the point of the arm is to find where. Small enough that a long scenario
/// overflows it, which is where memo should start winning.
pub const WINDOW: usize = 900;

/// Run a scenario against a store, consolidating between sessions.
///
/// `with_memory` is the switch the whole thing turns on: with it off, nothing is written and
/// every session starts blank, which is the baseline every harness ships today.
pub fn run(scenario: &Scenario, with_memory: bool) -> Score {
    measure(scenario, with_memory).0
}

/// Run one arm.
pub fn run_arm(scenario: &Scenario, arm: Arm) -> Score {
    measure_arm(scenario, arm).0
}

/// The same run, with what it cost.
///
/// One implementation rather than two, so a measured run and a scored run can never disagree
/// about what happened.
pub fn measure(scenario: &Scenario, with_memory: bool) -> (Score, Measured) {
    measure_arm(
        scenario,
        if with_memory {
            Arm::Memory
        } else {
            Arm::Nothing
        },
    )
}

/// The same, for one named arm.
pub fn measure_arm(scenario: &Scenario, arm: Arm) -> (Score, Measured) {
    let with_memory = arm == Arm::Memory;
    // What an agent with no memory system would still have in front of it: the text of what has
    // happened, oldest dropped first when it stops fitting.
    let mut window: Vec<String> = Vec::new();
    let mut store = Store::ephemeral().expect("a store");
    let settings = Settings::default();
    let scope = ScopeId::new(&scenario.project);
    let mut score = Score::default();
    let mut cost = Measured::default();

    for session in &scenario.sessions {
        let mut ran = Ran {
            session: session.id.clone(),
            rediscovered: Vec::new(),
            knew: Vec::new(),
        };

        for lesson in &session.lessons {
            cost.encounters += 1;
            let looked = match arm {
                Arm::Memory => look(&store, &scope, lesson, session.at, &mut cost),
                Arm::Nothing => false,
                // No retrieval, no ranking: the agent simply has the earlier text in front of
                // it. Knowing means the answer is still in the window and has not been pushed
                // out by everything said since.
                Arm::InWindow(_) => window.iter().any(|held| held.contains(&lesson.right)),
            };
            if looked {
                ran.knew.push(lesson.intent.clone());
                score.recalls += 1;
            } else {
                ran.rediscovered.push(lesson.intent.clone());
                score.rediscoveries += 1;
            }

            if with_memory {
                // What the session did, as a harness would have streamed it. A lesson already
                // known is still worked through — the agent uses the right command — and that
                // is itself another session agreeing.
                observe(&mut store, &scope, session, lesson, looked);
            }
            if let Arm::InWindow(budget) = arm {
                window.push(format!(
                    "{}: tried {}, which failed; {} worked",
                    lesson.intent, lesson.wrong, lesson.right
                ));
                // Oldest first out. This is the whole mechanism the arm exists to model — a
                // window is not a memory, and what falls off the front is gone.
                while window.iter().map(|l| l.len().div_ceil(4)).sum::<usize>() > budget {
                    window.remove(0);
                }
            }
        }

        if with_memory {
            // Between sessions, not during. Free compute, and the point at which what
            // recurred in unrelated runs becomes the project's.
            consolidate(
                &mut store,
                None,
                &settings,
                &scope,
                session.at + 3600,
                false,
            )
            .expect("consolidate");
        }
        score.sessions.push(ran);
    }
    cost.store_bytes = store.bytes().unwrap_or(0);
    (score, cost)
}

/// Whether the project already holds something that would have saved this session the trouble,
/// and what asking cost.
///
/// Asked the way a harness would ask it: what would be *asserted* for this turn. Not "is it in
/// the store" — a memory below the injection floor is one the model is never told, and a
/// benchmark that counted it would measure the store rather than the agent.
///
/// The findable set is read at the retrieval floor and the asserted set at the injection floor,
/// from one query, because the gap between them is the thing the two floors exist to create.
fn look(
    store: &Store,
    scope: &ScopeId,
    lesson: &Lesson,
    at: Timestamp,
    cost: &mut Measured,
) -> bool {
    let mut ask = memo_store::Recall::of(&lesson.intent, at);
    ask.limit = 10;
    ask.floor = floor::LIVE;
    ask.near = true;

    let started = std::time::Instant::now();
    let found = store.recall(&ask);
    cost.recall_us.push(started.elapsed().as_micros() as u64);

    let Ok(found) = found else { return false };
    let mine: Vec<_> = found.iter().filter(|h| h.memory.scope == *scope).collect();

    if mine.iter().any(|h| h.memory.text().contains(&lesson.right)) {
        cost.relevant_found += 1;
    }

    let mut knew = false;
    for hit in mine {
        if !hit.memory.is_assertable(floor::INJECT, at, false) {
            continue;
        }
        // What a harness would actually have been handed. Counted for everything asserted,
        // right or wrong, because a wrong assertion costs the same tokens as a right one.
        cost.injected_tokens += hit.memory.text().len().div_ceil(4);
        if hit.memory.text().contains(&lesson.right) {
            cost.asserted_right += 1;
            knew = true;
        } else {
            cost.asserted_wrong += 1;
        }
        // The failure a memory layer adds: asserting the command that was corrected, with no
        // mention of the correction. A repair names both — "`make test` rather than
        // `cargo test`" is the memory working, not a stale one — so naming the old command is
        // only stale when the new one is absent.
        if hit.memory.text().contains(&lesson.wrong) && !hit.memory.text().contains(&lesson.right) {
            cost.asserted_stale += 1;
        }
    }
    knew
}
/// Record what a session did, as a harness streaming its turns would.
fn observe(
    store: &mut Store,
    scope: &ScopeId,
    session: &crate::Session,
    lesson: &Lesson,
    knew: bool,
) {
    let id = SessionId::new(&session.id);
    store
        .open_session(&id, scope, &scenario_cwd(scope), "bench", session.at)
        .expect("open");

    // The turns. A session that did not know reaches for the wrong thing first and repairs;
    // one that did goes straight to the right thing.
    let mut turns = vec![Observation {
        cursor: Some(1),
        role: Role::User,
        text: session.asked.clone(),
        ..Observation::default()
    }];
    if !knew {
        turns.push(tool(2, &lesson.wrong, false));
    }
    turns.push(tool(3, &lesson.right, true));

    for turn in &turns {
        if turn.text.is_empty() {
            continue;
        }
        let mut scratch = Memory::new(
            mint(session.at),
            Tier::Scratch,
            scope.clone(),
            Body::note(&turn.text, NoteKind::Observation),
            session.at,
        );
        scratch.session = Some(id.clone());
        scratch.temporal = memo_model::Temporal::recalled(session.at, session.at);
        store.keep_scratch(scratch).expect("scratch");
    }

    // The repair, if there was one, is what SCAR turns into a habit — the path that makes a
    // single session's hard-won lesson worth keeping without waiting for a second one.
    for candidate in extract(&turns, &Settings::default().imperatives).candidates {
        let mut memory = Memory::new(
            mint(session.at),
            candidate.tier,
            scope.clone(),
            candidate.body.clone(),
            session.at,
        );
        memory.session = Some(id.clone());
        memory.strength.importance = candidate.importance;
        memory.temporal = memo_model::Temporal::recalled(session.at, session.at);
        let witness = Witness::new(
            WitnessId::new(format!(
                "{}-{}",
                candidate.from,
                candidate.cursor.unwrap_or(0)
            )),
            candidate.witness,
            id.clone(),
            scope.clone(),
            session.at,
        );
        store
            .remember(memory, witness, session.at)
            .expect("remember");
    }
}

/// One tool call, as a harness records it.
fn tool(cursor: u64, command: &str, ok: bool) -> Observation {
    Observation {
        cursor: Some(cursor),
        role: Role::Tool,
        tool: Some("shell".to_owned()),
        args: Some(serde_json::json!({ "command": command })),
        ok: Some(ok),
        text: if ok { "ok" } else { "no such command" }.to_owned(),
        ..Observation::default()
    }
}

/// Where a scenario's sessions ran.
fn scenario_cwd(scope: &ScopeId) -> String {
    scope.to_string()
}
