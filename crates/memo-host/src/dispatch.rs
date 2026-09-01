//! Turning a request into an answer.

use crate::{Door, verbs};
use memo_ipc::{Reply, Request};
use memo_model::{
    Body, Memory, MemoryId, NoteKind, ScopeId, SessionId, Tier, Timestamp, Witness, WitnessId,
    WitnessKind,
};
use memo_store::{Recall, Store, mint};

/// What the dispatcher needs to answer anything.
pub struct Answering<'a> {
    /// The memory being worked in.
    pub store: &'a mut Store,
    /// The scrollback, when this host keeps one.
    ///
    /// Optional in the type and mandatory in practice for a harness that keeps no journal of
    /// its own: without it there is nowhere for a turn to be, and `replay` has nothing to
    /// answer with. A host that serves such a harness and passes `None` here has quietly
    /// removed the only copy.
    pub scrollback: Option<&'a mut memo_store::Transcript>,
    /// Where each run's own memories go, when this host keeps them separately.
    ///
    /// A session's scratch belongs to that run and lives in that run's file, so deleting one
    /// run is deleting one directory. `None` keeps everything in [`Answering::store`], which is
    /// what a test with one ephemeral store wants and what `--store` asks for.
    pub scratch: Option<&'a mut memo_store::Scratchpad>,
    /// Which project.
    pub scope: ScopeId,
    /// The moment.
    pub now: Timestamp,
    /// Where assertion begins.
    pub inject_floor: f64,
    /// Where keeping begins.
    pub live_floor: f64,
    /// Whether the use-and-outcome ledger records anything.
    ///
    /// Off unless a configuration asked. Recording costs writes on the recall path, and a
    /// memory layer that started keeping a trail of what a caller searched for because a new
    /// version shipped is not one anybody should install.
    pub capture: bool,
}

impl Answering<'_> {
    /// The store a run's own memories belong in.
    ///
    /// The run's own file when this host keeps one, and the project's store otherwise. Callers
    /// do not branch on which: a scratch memory is written the same way either way, and the
    /// only difference is which file the write lands in.
    pub(crate) fn run(
        &mut self,
        session: &SessionId,
    ) -> Result<&mut Store, memo_store::StoreError> {
        match self.scratch.as_mut() {
            Some(pad) => pad.of(session),
            None => Ok(self.store),
        }
    }
}

/// Answer one call, with no way to describe a masked turn.
///
/// `plan` still works: a turn nobody can describe is left alone, which is the right answer when
/// there is no configuration to ask.
pub fn answer(at: &mut Answering<'_>, door: &Door, request: &Request) -> Reply {
    answer_with(at, door, request, |_| None)
}

/// Answer one call, asking `describe` what a masked turn should say.
pub fn answer_with(
    at: &mut Answering<'_>,
    door: &Door,
    request: &Request,
    describe: impl FnMut(&memo_store::Entry) -> Option<String>,
) -> Reply {
    let Some(verb) = verbs::known(&request.call) else {
        // Naming what is available beats "unknown verb": the usual cause is a sibling built
        // against a newer surface, and saying so is what makes that diagnosable.
        return Reply::refused(format!(
            "memo does not answer '{}' — it answers: {}",
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
        "status" => status(at),
        "recall" => recall(at, request),
        "why" => why(at, request),
        "sessions" => sessions(at),
        "observe" => crate::window::observe(at, request),
        "amend" => crate::window::amend(at, request),
        "replay" => crate::window::replay(at, request),
        "scroll" => crate::window::scroll(at, request),
        "resume" => crate::window::resume(at, request),
        "plan" => crate::window::plan(at, request, describe),
        "used" => crate::outcome::used(at, door, request),
        "outcome" => crate::outcome::outcome(at, door, request),
        "trace" => crate::outcome::trace(at, request),
        "utility" => crate::outcome::utility(at, request),
        "remember" => remember(at, door, request),
        "forget" => forget(at, door, request),
        // `context` needs the configuration's sections, which the caller assembles and hands
        // in. Answering it here without them would produce an injection nobody declared.
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
    // A peer asking for memory is a peer about to send it somewhere. The remote boundary is
    // the safe assumption unless it says otherwise.
    ask.remote = opts
        .and_then(|o| o.get("remote"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);

    let mut found = match at.store.recall(&ask) {
        Ok(found) => found,
        Err(why) => return Reply::refused(why.to_string()),
    };
    // A run searches its own scratch too. Its memories live in its own file now, so without
    // this a session could not find what it had just been told.
    if let Some(run) = opts
        .and_then(|o| o.get("session"))
        .and_then(serde_json::Value::as_str)
        .map(SessionId::new)
    {
        let own = match at.scratch.as_mut() {
            Some(pad) => pad.peek(&run).and_then(|held| match held {
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
    // Names are resolved once for the whole result set. A caller handed a bare identity has to
    // ask a second question to find out which run it means, and "which session" is one of the
    // two questions every answer here has to be able to settle.
    let names = session_names(at);
    let described: Vec<serde_json::Value> = found
        .iter()
        .map(|hit| describe(&hit.memory, at.inject_floor, at.now, &names))
        .collect();

    // Handing memories to a caller that is about to put them in a model's context *is* an
    // injection, and pretending otherwise would mean the ledger only ever saw the injections
    // somebody remembered to declare. The id goes back with the results so the caller can say
    // what it then did — which is the only way an outcome ever becomes attributable.
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
/// After the answer is decided, never before it: the ledger is instrumentation, and there is no
/// path from here back into what was returned.
fn note_served(
    at: &mut Answering<'_>,
    found: &[memo_store::Scored],
    opts: Option<&serde_json::Value>,
) -> Result<String, memo_store::StoreError> {
    let session = opts
        .and_then(|o| o.get("session"))
        .and_then(serde_json::Value::as_str)
        .map(SessionId::new);
    let query_hash = memo_model::content_hash(
        opts.and_then(|o| o.get("query"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default(),
    )[..16]
        .to_owned();
    let stamp = format!("{}-{}", at.now, &query_hash[..8]);

    at.store.note_recall(
        &memo_store::RecallRun {
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
            .map(|(rank, hit)| memo_store::Candidate {
                memory: hit.memory.id.clone(),
                rank,
                selected: true,
                score: hit.score,
                signals: memo_store::Signals {
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
        &memo_store::Injection {
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
                    memo_model::Presentation::Asserted
                } else {
                    memo_model::Presentation::Evidence
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
                // What the witnesses actually saw, when the scrollback still has it. The
                // difference between naming a cursor and showing the turn.
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

    // Which file this lands in. A session's own memory goes in that run's store; a durable
    // one goes in the project's, which is the same store when this host keeps no scratchpad.
    let landing_in = session.clone();

    // The ceiling, applied here rather than trusted to the caller: a peer proposes at the
    // weight a peer proposes at, whatever it asked for.
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
        memory.provenance = memo_model::Provenance::peer(who);
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
                    memo_store::Landing::Added(_) => "added",
                    memo_store::Landing::Reinforced(_) => "reinforced",
                    memo_store::Landing::Superseded { .. } => "superseded",
                },
                "id": id.to_string(),
                "witness": kind.as_str(),
            }))
        }
        Err(why) => Reply::refused(why.to_string()),
    }
}

/// Stop asserting something.
fn forget(at: &mut Answering<'_>, door: &Door, request: &Request) -> Reply {
    let Some(id) = request.args.first().and_then(|v| v.as_str()) else {
        return Reply::refused("forget needs an id");
    };
    let id = MemoryId::new(id);

    let held = match at.store.get(&id) {
        Ok(Some(memory)) => memory,
        Ok(None) => return Reply::refused(format!("no memory called '{id}'")),
        Err(why) => return Reply::refused(why.to_string()),
    };

    // A peer may retract what its own session put in. Anything else is the owner's to forget,
    // because a process that could archive a project's memory could quietly empty it.
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
        // Both: the identity a caller stores, and the name it shows a person. A durable
        // memory belongs to the project; the session says which run of it learned the thing.
        "session": memory.session.as_ref().map(ToString::to_string),
        "session_name": memory.session.as_ref()
            .and_then(|id| names.get(id.as_str()))
            .cloned(),
        "confidence": memory.confidence,
        // The distinction the whole design turns on, handed over rather than left for the
        // caller to work out from a number and a threshold it would have to be told.
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
