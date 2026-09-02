//! Running the scenarios, and saying plainly what broke.

use super::{Act, Case, Category, Expect, Probe, corpus};
use balthasar_distil::{Observation, Role, consolidate, extract};
use balthasar_lua::Settings;
use balthasar_model::{Memory, ScopeId, SessionId, Tier, Timestamp, Witness, WitnessId, floor};
use balthasar_store::{Recall, Store, mint};

/// What one probe did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// It behaved as it should.
    Held,
    /// It did not.
    Broke,
}

/// One probe that did not hold.
#[derive(Debug, Clone)]
pub struct Failure {
    /// Which scenario.
    pub case: &'static str,
    /// Which axis.
    pub category: Category,
    /// What was asked.
    pub asks: &'static str,
    /// What should have been true.
    pub why: &'static str,
    /// What actually came back.
    pub got: String,
}

/// How the whole suite went.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// Probes that held, by category.
    pub held: Vec<(Category, usize)>,
    /// Probes that did not, by category.
    pub broke: Vec<(Category, usize)>,
    /// Every failure, in order.
    pub failures: Vec<Failure>,
    /// How many probes ran.
    pub probes: usize,
}

impl Report {
    /// The share of probes that held.
    #[must_use]
    pub fn rate(&self) -> f64 {
        if self.probes == 0 {
            return 0.0;
        }
        let broke: usize = self.broke.iter().map(|(_, n)| n).sum();
        (self.probes - broke) as f64 / self.probes as f64
    }

    /// How one category went, as (held, broke).
    #[must_use]
    pub fn of(&self, category: Category) -> (usize, usize) {
        let get = |list: &[(Category, usize)]| {
            list.iter()
                .find(|(c, _)| *c == category)
                .map_or(0, |(_, n)| *n)
        };
        (get(&self.held), get(&self.broke))
    }
}

/// Run every scenario.
#[must_use]
pub fn run_suite() -> Report {
    let mut report = Report::default();
    for case in corpus() {
        run_case(&case, &mut report);
    }
    report
}

/// Run one.
fn run_case(case: &Case, report: &mut Report) {
    let mut store = Store::ephemeral().expect("a store");
    let settings = Settings::default();
    let scope = ScopeId::new(case.project);

    // What each run has already tried. The store no longer keeps a window ledger to read this
    // back out of, and it never should have: a scenario's own history is the scenario's.
    let mut ran: Vec<Ran> = Vec::new();
    for act in &case.script {
        perform(&mut store, &settings, &scope, act, &mut ran);
    }

    for probe in &case.probes {
        let verdict = check(&store, &scope, probe);
        let bucket = if verdict.0 == Verdict::Held {
            &mut report.held
        } else {
            &mut report.broke
        };
        match bucket.iter_mut().find(|(c, _)| *c == case.category) {
            Some((_, n)) => *n += 1,
            None => bucket.push((case.category, 1)),
        }
        report.probes += 1;
        if verdict.0 == Verdict::Broke {
            report.failures.push(Failure {
                case: case.name,
                category: case.category,
                asks: probe.asks,
                why: probe.why,
                got: verdict.1,
            });
        }
    }
}

/// One command a scenario has already run.
struct Ran {
    session: String,
    command: String,
    ok: bool,
}

/// What a scenario is running against: the store, its settings, and the history so far.
struct Running<'a> {
    store: &'a mut Store,
    settings: &'a Settings,
    scope: &'a ScopeId,
    ran: &'a mut Vec<Ran>,
}

/// Do one thing, through the real pipeline.
fn perform(store: &mut Store, settings: &Settings, scope: &ScopeId, act: &Act, ran: &mut Vec<Ran>) {
    match act {
        Act::Said { session, at, text } => {
            said(store, settings, scope, session, *at, text);
        }
        Act::Elsewhere {
            project,
            session,
            at,
            text,
        } => {
            said(store, settings, &ScopeId::new(*project), session, *at, text);
        }
        Act::Tool {
            session,
            at,
            command,
            ok,
        } => {
            tool(
                Running {
                    store,
                    settings,
                    scope,
                    ran,
                },
                session,
                *at,
                command,
                *ok,
            );
        }
        Act::Consolidate { at } => {
            consolidate(store, None, settings, scope, *at, false).expect("consolidate");
        }
        Act::Decay { at } => {
            store.decay(*at).expect("decay");
        }
        Act::Read {
            session,
            at,
            origin,
            text,
        } => {
            read(store, scope, session, *at, origin, text);
        }
        Act::Purged { at, matching } => {
            purge_matching(store, scope, *at, matching);
        }
        Act::Used { at, query } => {
            // Reaching for something is what resists decay. Done through recall so the test
            // exercises the path a harness would, rather than touching a row directly.
            let mut ask = Recall::of(*query, *at);
            ask.floor = 0.0;
            ask.scope_name = scope.to_string();
            let found = store.recall(&ask).expect("recall");
            for hit in found.iter().take(1) {
                store.touch(&hit.memory.id, *at).expect("touch");
            }
        }
    }
}

/// A person saying something, and whatever the ladder makes of it.
fn said(
    store: &mut Store,
    settings: &Settings,
    scope: &ScopeId,
    session: &str,
    at: Timestamp,
    text: &str,
) {
    let id = SessionId::new(session);
    store
        .open_session(&id, scope, &scope.to_string(), "suite", at)
        .expect("open");

    // Kept as scratch whatever happens: it is the session's own, and the ladder is what
    // carries it further.
    let mut scratch = Memory::new(
        mint(at),
        Tier::Scratch,
        scope.clone(),
        balthasar_model::Body::note(text, balthasar_model::NoteKind::Observation),
        at,
    );
    scratch.session = Some(id.clone());
    scratch.temporal = balthasar_model::Temporal::recalled(at, at);
    store.keep_scratch(scratch).expect("scratch");

    let turn = Observation {
        cursor: Some(1),
        role: Role::User,
        text: text.to_owned(),
        at: Some(at),
        ..Observation::default()
    };
    land(store, settings, scope, &id, at, &[turn]);
}

/// A tool running, and whatever the ladder makes of it.
fn tool(on: Running<'_>, session: &str, at: Timestamp, command: &str, ok: bool) {
    let Running {
        store,
        settings,
        scope,
        ran,
    } = on;
    let id = SessionId::new(session);
    store
        .open_session(&id, scope, &scope.to_string(), "suite", at)
        .expect("open");

    // The repair path needs the failure that came before it, so the whole run is replayed.
    let mut turns = Vec::new();
    for (index, past) in ran.iter().filter(|r| r.session == session).enumerate() {
        turns.push(Observation {
            cursor: Some(index as u64 + 1),
            role: Role::Tool,
            tool: Some("shell".to_owned()),
            args: Some(serde_json::json!({ "command": past.command })),
            ok: Some(past.ok),
            ..Observation::default()
        });
    }
    let cursor = turns.len() as u64 + 1;
    turns.push(Observation {
        cursor: Some(cursor),
        role: Role::Tool,
        tool: Some("shell".to_owned()),
        args: Some(serde_json::json!({ "command": command })),
        ok: Some(ok),
        text: if ok { "ok" } else { "failed" }.to_owned(),
        at: Some(at),
        ..Observation::default()
    });

    ran.push(Ran {
        session: session.to_owned(),
        command: command.to_owned(),
        ok,
    });

    land(store, settings, scope, &id, at, &turns);
}

/// Offer whatever the extractors found to the store.
fn land(
    store: &mut Store,
    settings: &Settings,
    scope: &ScopeId,
    session: &SessionId,
    at: Timestamp,
    turns: &[Observation],
) {
    for candidate in extract(turns, &settings.imperatives).candidates {
        let score = candidate.score(|kind| settings.weight(kind));
        if score < settings.floors().promote {
            continue;
        }
        let mut memory = Memory::new(
            mint(at),
            candidate.tier,
            scope.clone(),
            candidate.body.clone(),
            at,
        );
        memory.session = Some(session.clone());
        memory.strength.importance = candidate.importance;
        memory.strength.pinned = candidate.pinned;
        memory.temporal = balthasar_model::Temporal::recalled(at, at);

        let witness = Witness::new(
            WitnessId::new(format!(
                "{}-{}-{}",
                candidate.from,
                candidate.cursor.unwrap_or(0),
                &memory.content_hash[..8]
            )),
            candidate.witness,
            session.clone(),
            scope.clone(),
            at,
        );
        store.remember(memory, witness, at).expect("remember");
    }
}

/// Ask one question and judge the answer.
fn check(store: &Store, scope: &ScopeId, probe: &Probe) -> (Verdict, String) {
    let mut ask = Recall::of(probe.asks, probe.at);
    ask.limit = 20;
    ask.floor = 0.0;
    ask.near = true;
    ask.scope_name = scope.to_string();
    // What a harness would actually be told: sure enough to state, and an answer to what was
    // asked. Assertion alone is query-independent by design, so on its own it would call a
    // confidently-held fact about staging an answer about production.
    ask.relevance = 0.55;
    let found = store.recall(&ask).expect("recall");

    let asserted: Vec<String> = found
        .iter()
        .filter(|hit| hit.memory.scope == *scope)
        .filter(|hit| hit.memory.is_assertable(floor::INJECT, probe.at, false))
        .map(|hit| hit.memory.text())
        .collect();

    let holds = |text: &str, list: &[String]| {
        let needle = text.to_lowercase();
        list.iter().any(|t| t.to_lowercase().contains(&needle))
    };

    let verdict = match &probe.expect {
        Expect::Asserted(text) => holds(text, &asserted),
        Expect::NotAsserted(text) => !holds(text, &asserted),
        Expect::Silent => asserted.is_empty(),
        Expect::Findable(text) => {
            let anywhere: Vec<String> = store
                .all()
                .expect("export")
                .into_iter()
                .filter(|m| m.scope == *scope)
                .map(|m| m.text())
                .collect();
            holds(text, &anywhere) && !holds(text, &asserted)
        }
        Expect::Absent(text) => {
            let anywhere: Vec<String> = store
                .all()
                .expect("export")
                .into_iter()
                .map(|m| m.text())
                .collect();
            !holds(text, &anywhere)
        }
    };

    let got = if asserted.is_empty() {
        "nothing asserted".to_owned()
    } else {
        format!("asserted: {}", asserted.join(" | "))
    };
    (
        if verdict {
            Verdict::Held
        } else {
            Verdict::Broke
        },
        got,
    )
}

/// Something arriving from outside.
///
/// The channel is what makes this different from a person saying it, and the domain is what
/// makes several arrivals of the same material one source. Both go on the witness, because the
/// defence lives in the arithmetic rather than in a filter.
fn read(
    store: &mut Store,
    scope: &ScopeId,
    session: &str,
    at: Timestamp,
    origin: &str,
    text: &str,
) {
    let id = SessionId::new(session);
    store
        .open_session(&id, scope, &scope.to_string(), "suite", at)
        .expect("open");

    let mut held = Memory::new(
        mint(at),
        Tier::Fact,
        scope.clone(),
        balthasar_model::Body::note(text, balthasar_model::NoteKind::Claim),
        at,
    );
    held.session = Some(id.clone());
    held.temporal = balthasar_model::Temporal::recalled(at, at);
    held.strength.pinned = false;

    // What a document claims to be does not decide what it mints. A page phrased as an
    // instruction produces a distillation, because phrasing is what an attacker controls.
    let kind = balthasar_model::witness_for(
        balthasar_model::Channel::ExternalContent,
        balthasar_model::WitnessKind::Imperative,
    );
    let witness = Witness::new(
        balthasar_model::WitnessId::new(format!("read-{session}-{at}")),
        kind,
        id,
        scope.clone(),
        at,
    )
    .through(
        balthasar_model::Channel::ExternalContent,
        Some(balthasar_model::Domain::external(origin)),
    );
    store.remember(held, witness, at).expect("read");
}

/// Removing whatever matches, the way `balthasar forget --purge` would.
fn purge_matching(store: &mut Store, scope: &ScopeId, at: Timestamp, matching: &str) {
    let mut ask = Recall::of(matching, at);
    ask.floor = 0.0;
    ask.limit = 50;
    ask.include_archived = true;
    ask.scope_name = scope.to_string();
    let found = store.recall(&ask).expect("recall");
    let doomed: Vec<_> = found
        .into_iter()
        .filter(|hit| hit.memory.text().contains(matching))
        .map(|hit| hit.memory.id)
        .collect();
    for id in doomed {
        balthasar_store::purge(store, &id).expect("purge");
    }
}
