//! Running a scenario, with memory and without, and comparing.
//!
//! The measurement is not "does recall find the thing". It is **does session k+1 avoid the
//! mistake session k already made** — which is the only question a person using a coding agent
//! actually has, and the one no published benchmark asks.

use crate::{Lesson, Scenario};
use aeon_distil::{Observation, Role, consolidate, extract};
use aeon_lua::Settings;
use aeon_model::{
    Body, Memory, NoteKind, ScopeId, SessionId, Tier, Timestamp, Witness, WitnessId, floor,
};
use aeon_store::{Store, mint};

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

/// Run a scenario against a store, consolidating between sessions.
///
/// `with_memory` is the switch the whole thing turns on: with it off, nothing is written and
/// every session starts blank, which is the baseline every harness ships today.
pub fn run(scenario: &Scenario, with_memory: bool) -> Score {
    let mut store = Store::ephemeral().expect("a store");
    let settings = Settings::default();
    let scope = ScopeId::new(&scenario.project);
    let mut score = Score::default();

    for session in &scenario.sessions {
        let mut ran = Ran {
            session: session.id.clone(),
            rediscovered: Vec::new(),
            knew: Vec::new(),
        };

        for lesson in &session.lessons {
            let known = with_memory && already_knows(&store, &scope, lesson, session.at);
            if known {
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
                observe(&mut store, &scope, session, lesson, known);
            }
        }

        if with_memory {
            // Between sessions, not during. Free compute, and the point at which what
            // recurred in unrelated runs becomes the project's.
            consolidate(&mut store, None, &settings, &scope, session.at + 3600, false)
                .expect("consolidate");
        }
        score.sessions.push(ran);
    }
    score
}

/// Whether the project already holds something that would have saved this session the trouble.
///
/// Asked the way a harness would ask it: what would be *asserted* for this turn. Not "is it in
/// the store" — a memory below the injection floor is one the model is never told, and a
/// benchmark that counted it would measure the store rather than the agent.
fn already_knows(store: &Store, scope: &ScopeId, lesson: &Lesson, at: Timestamp) -> bool {
    let mut ask = aeon_store::Recall::of(&lesson.intent, at);
    ask.limit = 10;
    ask.floor = floor::INJECT;
    ask.near = true;
    let Ok(found) = store.recall(&ask) else {
        return false;
    };
    found.iter().any(|hit| {
        hit.memory.is_assertable(floor::INJECT, at, false)
            && hit.memory.text().contains(&lesson.right)
            && hit.memory.scope == *scope
    })
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
        scratch.temporal = aeon_model::Temporal::recalled(session.at, session.at);
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
        memory.temporal = aeon_model::Temporal::recalled(session.at, session.at);
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
