//! Turning a request into an answer.

use crate::{Door, verbs};
use balthasar_ipc::{Reply, Request};
use balthasar_model::{
    AgentId, Body, Memory, MemoryId, NoteKind, ScopeId, SessionId, Tier, Timestamp, Witness,
    WitnessId, WitnessKind,
};
use balthasar_store::{Recall, Store, mint};

/// What the dispatcher needs to answer anything.
pub struct Answering<'a> {
    /// The memory being worked in.
    pub store: &'a mut Store,
    /// The scrollback, when this host keeps one.
    ///
    /// Without it there is nowhere for a turn to be and `replay` has nothing to answer with.
    pub scrollback: Option<&'a mut balthasar_store::Transcript>,
    /// Where each run's own memories go, when this host keeps them separately.
    ///
    /// A session's scratch belongs to that run and lives in that run's file.
    pub scratch: Option<&'a mut balthasar_store::Scratchpad>,
    /// Which project.
    pub scope: ScopeId,
    /// Which agent inside a run this connection belongs to.
    ///
    /// Pinned when the connection is accepted and never a call argument.
    pub agent: AgentId,
    /// The moment.
    pub now: Timestamp,
    /// Where assertion begins.
    pub inject_floor: f64,
    /// Where keeping begins.
    pub live_floor: f64,
    /// Whether the use-and-outcome ledger records anything.
    ///
    /// Off unless a configuration asked.
    pub capture: bool,
}

impl Answering<'_> {
    /// The store a run's own memories belong in.
    ///
    /// The run's own file when this host keeps one, and the project's store otherwise.
    pub(crate) fn run(
        &mut self,
        session: &SessionId,
    ) -> Result<&mut Store, balthasar_store::StoreError> {
        // The pinned agent, taken from this connection rather than from the call.
        let agent = self.agent.clone();
        match self.scratch.as_mut() {
            Some(pad) => pad.of(session, &agent),
            None => Ok(self.store),
        }
    }
}

/// Answer one call, with no way to describe a masked turn.
///
/// `plan` still works: a turn nobody can describe is left alone.
pub fn answer(at: &mut Answering<'_>, door: &Door, request: &Request) -> Reply {
    answer_with(at, door, request, |_| None)
}

/// Answer one call, asking `describe` what a masked turn should say.
pub fn answer_with(
    at: &mut Answering<'_>,
    door: &Door,
    request: &Request,
    describe: impl FnMut(&balthasar_store::Turn) -> Option<String>,
) -> Reply {
    let Some(verb) = verbs::known(&request.call) else {
        // Naming what is available beats "unknown verb".
        return Reply::refused(format!(
            "balthasar does not answer '{}' — it answers: {}",
            request.call,
            verbs::SURFACE
                .iter()
                .map(|v| v.name)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    };

    match verb.name {
        "verbs" => Reply::one(serde_json::json!(
            verbs::SURFACE
                .iter()
                .map(
                    |v| serde_json::json!({ "name": v.name, "writes": v.writes, "about": v.about })
                )
                .collect::<Vec<_>>()
        )),
        // Shipped in the binary and identical for every session, so it answers like `verbs`.
        "client" => Reply::one(serde_json::json!(balthasar_lua::CLIENT)),
        "status" => status(at),
        "recall" => recall(at, request),
        "why" => why(at, request),
        "sessions" => sessions(at),
        "observe" => crate::window::observe(at, request),
        "amend" => crate::window::amend(at, request),
        "replay" => crate::window::replay(at, request),
        "scroll" => crate::window::scroll(at, request),
        "resume" => crate::window::resume(at, request),
        "model" => crate::window::model(at, request),
        "plan" => crate::window::plan(at, request, describe),
        "used" => crate::outcome::used(at, door, request),
        "outcome" => crate::outcome::outcome(at, door, request),
        "trace" => crate::outcome::trace(at, request),
        "utility" => crate::outcome::utility(at, request),
        "remember" => remember(at, door, request),
        "forget" => forget(at, door, request),
        // `context` needs the configuration's sections, which the caller assembles and hands in.
        "context" => Reply::refused("context is served by the host that holds the configuration"),
        other => Reply::refused(format!("'{other}' is named but not wired")),
    }
}

/// What this memory holds.
fn status(at: &mut Answering<'_>) -> Reply {
    let census = match at.store.census() {
        Ok(census) => census,
        Err(why) => return Reply::refused(why.to_string()),
    };
    Reply::one(serde_json::json!({
        "scope": at.scope.to_string(),
        "path": at.store.path().to_string_lossy(),
        "tiers": census.into_iter()
            .map(|(tier, count)| (tier, serde_json::json!(count)))
            .collect::<serde_json::Map<String, serde_json::Value>>(),
        "inject_floor": at.inject_floor,
    }))
}

/// Search.
fn recall(at: &mut Answering<'_>, request: &Request) -> Reply {
    let query = request.args.first().and_then(|v| v.as_str()).unwrap_or("");
    let opts = request.args.get(1);
    let limit = opts
        .and_then(|o| o.get("limit"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(10)
        .min(100) as usize;

    let mut ask = Recall::of(query, at.now);
    ask.limit = limit;
    ask.floor = at.live_floor;
    ask.near = true;
    // A peer asking for memory is about to send it somewhere, so the remote boundary is assumed.
    ask.remote = opts
        .and_then(|o| o.get("remote"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);

    let mut found = match at.store.recall(&ask) {
        Ok(found) => found,
        Err(why) => return Reply::refused(why.to_string()),
    };
    // A run searches its own scratch too, which now lives in its own file.
    if let Some(run) = opts
        .and_then(|o| o.get("session"))
        .and_then(serde_json::Value::as_str)
        .map(SessionId::new)
    {
        // This agent's own scratch and no sibling's.
        let agent = at.agent.clone();
        let own = match at.scratch.as_mut() {
            Some(pad) => pad.peek(&run, &agent).and_then(|held| match held {
                Some(store) => store.recall(&ask),
                None => Ok(Vec::new()),
            }),
            None => Ok(Vec::new()),
        };
        match own {
            Ok(mine) => found.extend(mine),
            Err(why) => return Reply::refused(why.to_string()),
        }
        found.sort_by(|a, b| b.score.total_cmp(&a.score));
        found.truncate(limit);
    }
    // Names are resolved once for the whole result set.
    let names = session_names(at);
    let described: Vec<serde_json::Value> = found
        .iter()
        .map(|hit| describe(&hit.memory, at.inject_floor, at.now, &names))
        .collect();

    // Handing memories to a caller about to put them in a context is an injection.
    let injection = if at.capture {
        match note_served(at, &found, opts) {
            Ok(id) => Some(id),
            Err(why) => return Reply::refused(why.to_string()),
        }
    } else {
        None
    };

    match injection {
        None => Reply::one(serde_json::json!(described)),
        Some(id) => Reply::one(serde_json::json!({
            "injection": id,
            "memories": described,
        })),
    }
}

/// Record that a search happened and that its results were handed over.
///
/// After the answer is decided, never before it: the ledger is instrumentation.
fn note_served(
    at: &mut Answering<'_>,
    found: &[balthasar_store::Scored],
    opts: Option<&serde_json::Value>,
) -> Result<String, balthasar_store::StoreError> {
    let session = opts
        .and_then(|o| o.get("session"))
        .and_then(serde_json::Value::as_str)
        .map(SessionId::new);
    let query_hash = balthasar_model::content_hash(
        opts.and_then(|o| o.get("query"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default(),
    )[..16]
        .to_owned();
    let stamp = format!("{}-{}", at.now, &query_hash[..8]);

    at.store.note_recall(
        &balthasar_store::RecallRun {
            id: format!("recall-{stamp}"),
            scope: at.scope.clone(),
            session: session.clone(),
            query_hash,
            requested_at: at.now,
            config_fingerprint: String::new(),
            vector_available: false,
            result_limit: found.len(),
            latency_us: 0,
        },
        &found
            .iter()
            .enumerate()
            .map(|(rank, hit)| balthasar_store::Candidate {
                memory: hit.memory.id.clone(),
                rank,
                selected: true,
                score: hit.score,
                signals: balthasar_store::Signals {
                    semantic: hit.semantic.unwrap_or(0.0),
                    lexical: hit.lexical,
                    entity: hit.entity,
                    frecency: hit.frecency,
                    confidence: hit.confidence,
                    strength: hit.strength,
                    scope: f64::from(u8::from(hit.near)),
                },
            })
            .collect::<Vec<_>>(),
    )?;

    let injection = format!("inject-{stamp}");
    let tokens: usize = found
        .iter()
        .map(|h| h.memory.text().len().div_ceil(4))
        .sum();
    at.store.note_injection(
        &balthasar_store::Injection {
            id: injection.clone(),
            recall: Some(format!("recall-{stamp}")),
            session,
            created_at: at.now,
            token_count: tokens,
            remote: true,
            policy: "balanced".to_owned(),
        },
        &found
            .iter()
            .map(|hit| {
                let mode = if hit.memory.is_assertable(at.inject_floor, at.now, true) {
                    balthasar_model::Presentation::Asserted
                } else {
                    balthasar_model::Presentation::Evidence
                };
                (hit.memory.id.clone(), mode)
            })
            .collect::<Vec<_>>(),
    )?;
    Ok(injection)
}

/// The evidence for one memory.
fn why(at: &mut Answering<'_>, request: &Request) -> Reply {
    let Some(id) = request.args.first().and_then(|v| v.as_str()) else {
        return Reply::refused("why needs an id");
    };
    let held = at.store.get(&MemoryId::new(id));
    match held {
        Ok(Some(memory)) => {
            let names = session_names(at);
            let quoted: Vec<serde_json::Value> = memory
                .witnesses
                .iter()
                .filter_map(|w| {
                    let cursor = w.cursor?;
                    let turn = at
                        .scrollback
                        .as_ref()?
                        .at(&w.session, cursor)
                        .ok()
                        .flatten()?;
                    Some(serde_json::json!({
                        "session": w.session.to_string(),
                        "cursor": cursor,
                        "role": turn.role,
                        "said": turn.text,
                    }))
                })
                .collect();
            Reply::one(serde_json::json!({
                "id": memory.id.to_string(),
                "text": memory.text(),
                "confidence": memory.confidence,
                "sessions": memory.distinct_sessions(),
                "witnesses": memory.witnesses.iter().map(|w| serde_json::json!({
                    "kind": w.kind.as_str(),
                    "session": names.get(w.session.as_str()).cloned()
                        .unwrap_or_else(|| w.session.to_string()),
                    "at": w.at,
                    "cursor": w.cursor,
                    "worth": w.value(at.now),
                    "note": w.note,
                })).collect::<Vec<_>>(),
                // What the witnesses actually saw, when the scrollback still has it.
                "quoted": quoted,
            }))
        }
        Ok(None) => Reply::refused(format!("no memory called '{id}'")),
        Err(why) => Reply::refused(why.to_string()),
    }
}

/// The runs this project has had.
fn sessions(at: &mut Answering<'_>) -> Reply {
    match at.store.sessions(50) {
        Ok(found) => Reply::one(serde_json::json!(
            found
                .into_iter()
                .map(|s| serde_json::json!({
                    "id": s.id.to_string(),
                    "name": s.name,
                    "title": s.title,
                    "project": s.scope.to_string(),
                    "harness": s.harness,
                    "opened": s.opened,
                    "open": s.is_open(),
                }))
                .collect::<Vec<_>>()
        )),
        Err(why) => Reply::refused(why.to_string()),
    }
}

/// Propose something worth keeping.
fn remember(at: &mut Answering<'_>, door: &Door, request: &Request) -> Reply {
    let Some(text) = request.args.first().and_then(|v| v.as_str()) else {
        return Reply::refused("remember needs something to remember");
    };
    let text = text.trim();
    if text.is_empty() {
        return Reply::refused("remember needs something to remember");
    }
    let opts = request.args.get(1);

    let asked_scope = opts
        .and_then(|o| o.get("scope"))
        .and_then(serde_json::Value::as_str)
        .map_or_else(|| at.scope.clone(), ScopeId::new);
    let scope = door.scope_for(&asked_scope, &at.scope);

    let session = opts
        .and_then(|o| o.get("session"))
        .and_then(serde_json::Value::as_str)
        .map(SessionId::new);

    // Which file this lands in. A session's own memory goes in that run's store.
    let landing_in = session.clone();

    // The ceiling, applied here rather than trusted to the caller.
    let kind = door.witness_for(WitnessKind::Imperative);
    let pinned = door.may_pin()
        && opts
            .and_then(|o| o.get("pin"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

    let tier = if session.is_some() {
        Tier::Scratch
    } else {
        Tier::Fact
    };
    let mut memory = Memory::new(
        mint(at.now),
        tier,
        scope.clone(),
        Body::note(text, NoteKind::Claim),
        at.now,
    );
    memory.session.clone_from(&session);
    memory.strength.pinned = pinned;
    if let Some(who) = door.who() {
        memory.provenance = balthasar_model::Provenance::peer(who);
    }

    let witness = Witness::new(
        WitnessId::new(format!("peer-{}-{}", at.now, &memory.content_hash[..8])),
        kind,
        session.unwrap_or_else(|| SessionId::new("peer")),
        scope,
        at.now,
    );
    let witness = match door.who() {
        Some(who) => witness.noted(who),
        None => witness.noted("typed at the command line"),
    };

    let now = at.now;
    let landed = match landing_in {
        Some(run) => match at.run(&run) {
            Ok(store) => store.remember(memory, witness, now),
            Err(why) => return Reply::refused(why.to_string()),
        },
        None => at.store.remember(memory, witness, now),
    };
    match landed {
        Ok(landing) => {
            let id = landing.id().clone();
            Reply::one(serde_json::json!({
                "landing": match landing {
                    balthasar_store::Landing::Added(_) => "added",
                    balthasar_store::Landing::Reinforced(_) => "reinforced",
                    balthasar_store::Landing::Superseded { .. } => "superseded",
                },
                "id": id.to_string(),
                "witness": kind.as_str(),
            }))
        }
        Err(why) => Reply::refused(why.to_string()),
    }
}

/// Stop asserting something, or a whole run of them.
fn forget(at: &mut Answering<'_>, door: &Door, request: &Request) -> Reply {
    let Some(id) = request.args.first().and_then(|v| v.as_str()) else {
        return Reply::refused("forget needs an id");
    };
    let said = request.args.get(1);
    let asked = |name: &str| {
        said.and_then(|s| s.get(name))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    };
    if asked("session") {
        return forget_run(at, door, id, asked("purge"));
    }
    let id = MemoryId::new(id);

    let held = match at.store.get(&id) {
        Ok(Some(memory)) => memory,
        Ok(None) => return Reply::refused(format!("no memory called '{id}'")),
        Err(why) => return Reply::refused(why.to_string()),
    };

    // A peer may retract what its own session put in. Anything else is the owner's to forget.
    if !matches!(door, Door::Owner) {
        let own = held.tier == Tier::Scratch;
        if !own {
            return Reply::refused(
                "a peer may forget only what its own session wrote — ask the owner",
            );
        }
    }

    match at.store.archive(&id, at.now) {
        Ok(()) => Reply::one(serde_json::json!({ "archived": id.to_string() })),
        Err(why) => Reply::refused(why.to_string()),
    }
}

/// Stop asserting a whole run, or remove it.
///
/// A run is in three files: purging reaches all three, archiving touches memories alone.
fn forget_run(at: &mut Answering<'_>, door: &Door, handle: &str, purge: bool) -> Reply {
    let session = match at.store.session(handle) {
        Ok(Some(found)) => found.id,
        Ok(None) => SessionId::new(handle),
        Err(why) => return Reply::refused(why.to_string()),
    };

    let turns = match at.scrollback.as_ref() {
        Some(held) => held.replay(&session).map(|t| t.len()).unwrap_or(0),
        None => 0,
    };
    let promoted = match at.store.owned_by(&session) {
        Ok(ids) => ids,
        Err(why) => return Reply::refused(why.to_string()),
    };
    // A handle naming no run is a typo, and must not answer as a successful purge of nothing.
    let known = turns > 0
        || !promoted.is_empty()
        // Any agent of it, because a run one subagent wrote scratch for is a run that happened.
        || at
            .scratch
            .as_ref()
            .is_some_and(|pad| !pad.agents_of(&session).is_empty());
    if !known {
        return Reply::refused(format!("no run called '{handle}'"));
    }

    if !purge {
        return archive_run(at, door, &session, &promoted);
    }
    // The existing ceiling, used for the first time.
    if !door.may_purge() {
        return Reply::refused("removing a run is the owner's — a peer may archive it");
    }

    let memories = match balthasar_store::purge_session(at.store, &session) {
        Ok(n) => n,
        Err(why) => return Reply::refused(why.to_string()),
    };
    let gone = match at.scrollback.as_ref() {
        Some(held) => match balthasar_store::purge_run(held, &session) {
            Ok(n) => n,
            Err(why) => return Reply::refused(why.to_string()),
        },
        None => 0,
    };
    let scratch = match at.scratch.as_mut() {
        Some(pad) => balthasar_store::purge_scratch(pad, &session).unwrap_or(false),
        None => false,
    };

    Reply::one(serde_json::json!({
        "purged": session.to_string(),
        "memories": memories,
        "turns": gone,
        "scratch": scratch,
    }))
}

/// Stop asserting what a run learned, keeping every word of it.
///
/// A peer reaches only the run's own scratch.
fn archive_run(
    at: &mut Answering<'_>,
    door: &Door,
    session: &SessionId,
    promoted: &[MemoryId],
) -> Reply {
    let now = at.now;
    let mut archived = 0;
    let mut left = 0;

    // Every agent of the run, not only the one asking.
    let agents = at
        .scratch
        .as_ref()
        .map(|pad| pad.agents_of(session))
        .unwrap_or_default();
    for agent in &agents {
        if let Some(Ok(Some(own))) = at.scratch.as_mut().map(|pad| pad.peek(session, agent)) {
            for id in own.owned_by(session).unwrap_or_default() {
                if own.archive(&id, now).is_ok() {
                    archived += 1;
                }
            }
        }
    }

    for id in promoted {
        if !matches!(door, Door::Owner) {
            left += 1;
            continue;
        }
        if at.store.archive(id, now).is_ok() {
            archived += 1;
        }
    }

    Reply::one(serde_json::json!({
        "archived": archived,
        "session": session.to_string(),
        // Named rather than silent, so a peer is not told something untrue by omission.
        "left_to_the_owner": left,
    }))
}

/// One memory, as a peer receives it.
fn describe(
    memory: &Memory,
    inject_floor: f64,
    now: Timestamp,
    names: &std::collections::HashMap<String, String>,
) -> serde_json::Value {
    serde_json::json!({
        "id": memory.id.to_string(),
        "text": memory.text(),
        "tier": memory.tier.as_str(),
        "project": memory.scope.to_string(),
        // Both: the identity a caller stores, and the name it shows a person.
        "session": memory.session.as_ref().map(ToString::to_string),
        "session_name": memory.session.as_ref()
            .and_then(|id| names.get(id.as_str()))
            .cloned(),
        "confidence": memory.confidence,
        // Computed here rather than left to the caller to derive from a number and a threshold.
        "asserted": memory.is_assertable(inject_floor, now, true),
        "since": memory.temporal.valid_from,
        "until": memory.temporal.valid_to,
    })
}

/// Session ids and the names they are printed under.
fn session_names(at: &mut Answering<'_>) -> std::collections::HashMap<String, String> {
    at.store
        .sessions(usize::MAX)
        .unwrap_or_default()
        .into_iter()
        .map(|s| (s.id.to_string(), s.name))
        .collect()
}
