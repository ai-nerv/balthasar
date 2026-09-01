//! Reading somebody else's transcripts.
//!
//! A source adapter is Lua and lives in `config/sources/`. This walks whatever it names,
//! converts each line through it, runs the extractors, and offers what survives to the store.
//!
//! Re-running is safe and cheap. Every file is stamped with the extractor version that read it,
//! so a second pass skips what has not changed — and a *better* extractor, being a new version,
//! reads everything again without anybody having to remember to say so.

use crate::{Candidate, DistilError, Extracted, Meta, Observation, Verdict, extract, weigh};
use memo_lua::{Engine, Settings};
use memo_model::{Memory, ScopeId, SessionId, Timestamp, Witness, WitnessId};
use memo_store::{Landing, Store, mint};

/// The extractor version stamped against every file this build reads.
///
/// Bumping it is how a better rule gets applied to old material: the stamp no longer matches,
/// so the next ingest reads everything again.
pub const EXTRACTOR_VERSION: i64 = 1;

/// What to read, and how much of it.
#[derive(Debug, Clone)]
pub struct Ingest {
    /// Which registered source.
    pub source: String,
    /// Which memory to write into.
    pub scope: ScopeId,
    /// Only sessions that started after this, when given.
    pub since: Option<Timestamp>,
    /// Say what would happen without writing anything.
    pub dry_run: bool,
    /// The moment to score against.
    pub now: Timestamp,
}

/// One session a source named.
#[derive(Debug, Clone)]
pub struct Source {
    /// The file, as the adapter named it.
    pub file: String,
    /// What the adapter said about it.
    pub meta: Meta,
}

/// What an ingest did.
#[derive(Debug, Default, Clone)]
pub struct Report {
    /// Sessions the source named.
    pub sessions: usize,
    /// Sessions skipped because their stamp already matched.
    pub already_read: usize,
    /// Turns the adapter converted.
    pub observations: usize,
    /// Candidates the extractors proposed.
    pub proposed: usize,
    /// Candidates that crossed the gate.
    pub promoted: usize,
    /// Candidates that reinforced something already held.
    pub reinforced: usize,
    /// Candidates that replaced something.
    pub superseded: usize,
    /// Candidates that waited for a second witness.
    pub held: usize,
    /// Candidates a floor or a configuration refused, with why.
    pub refused: Vec<(String, String)>,
    /// Whether anything was written.
    pub dry_run: bool,
}

/// Read what a source names, and offer what it teaches.
pub fn ingest(
    engine: &mut Engine,
    store: &mut Store,
    settings: &Settings,
    ask: &Ingest,
) -> Result<Report, DistilError> {
    let mut report = Report {
        dry_run: ask.dry_run,
        ..Report::default()
    };

    if !engine.offers("source", &ask.source, "sessions") {
        return Err(if engine.offers("source", &ask.source, "line") {
            DistilError::Incomplete(ask.source.clone(), "sessions")
        } else {
            DistilError::NoSource(ask.source.clone())
        });
    }

    for file in files(engine, &ask.source) {
        let Some(source) = describe(engine, &ask.source, &file)? else {
            continue;
        };
        if ask.since.is_some_and(|floor| source.meta.opened < floor) {
            continue;
        }
        report.sessions += 1;

        if !ask.dry_run && store.already_read(&ask.source, &file, EXTRACTOR_VERSION)? {
            report.already_read += 1;
            continue;
        }

        // The session is recorded before anything it taught, so every memory can name the run
        // it came from and a person can ask what one run left behind.
        store.open_session(
            &SessionId::new(&source.meta.id),
            &ask.scope,
            &source.meta.cwd,
            &ask.source,
            source.meta.opened,
        )?;

        let turns = read(engine, &ask.source, &file, &mut report)?;
        if let Some(first) = turns.iter().find(|t| t.role == crate::Role::User) {
            store.title_session(&SessionId::new(&source.meta.id), &first.text)?;
        }
        let found = extract(&turns, &settings.imperatives);
        report.proposed += found.candidates.len();
        land(store, engine, settings, ask, &source, &found, &mut report)?;

        if !ask.dry_run {
            store.stamp(&ask.source, &file, EXTRACTOR_VERSION, ask.now)?;
            // A transcript that has been read to the end is a run that has ended.
            store.close_session(&SessionId::new(&source.meta.id), ask.now)?;
        }
    }
    Ok(report)
}

/// Every file the adapter names.
fn files(engine: &mut Engine, source: &str) -> Vec<String> {
    engine
        .call("source", source, "sessions", &[])
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect()
}

/// What the adapter says about one file, from its first line.
fn describe(engine: &mut Engine, source: &str, file: &str) -> Result<Option<Source>, DistilError> {
    let text = std::fs::read_to_string(file).map_err(|e| DistilError::Io(file.to_owned(), e))?;
    let first = text.lines().next().unwrap_or_default();

    let meta = engine
        .call("source", source, "meta", &[serde_json::json!(first)])
        .and_then(|v| serde_json::from_value::<Meta>(v).ok());

    // A file the adapter will not describe is one it does not recognise. Skipping is right:
    // a sessions() that globs a directory will find things that are not transcripts.
    Ok(meta.map(|meta| Source {
        file: file.to_owned(),
        meta,
    }))
}

/// Every turn the adapter converts, in order.
fn read(
    engine: &mut Engine,
    source: &str,
    file: &str,
    report: &mut Report,
) -> Result<Vec<Observation>, DistilError> {
    let text = std::fs::read_to_string(file).map_err(|e| DistilError::Io(file.to_owned(), e))?;
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        // A line the adapter skips or raises on costs that line and nothing else. A source
        // walks somebody else's file and one bad record must not end the ingest.
        let Some(value) = engine.call("source", source, "line", &[serde_json::json!(line)]) else {
            continue;
        };
        for one in spread(value) {
            if let Ok(observation) = serde_json::from_value::<Observation>(one) {
                out.push(observation);
                report.observations += 1;
            }
        }
    }
    Ok(out)
}

/// An adapter may answer with one turn or several.
fn spread(value: serde_json::Value) -> Vec<serde_json::Value> {
    match value {
        serde_json::Value::Array(items) => items,
        other => vec![other],
    }
}

/// Offer every candidate to the gate, then to the store.
fn land(
    store: &mut Store,
    engine: &mut Engine,
    settings: &Settings,
    ask: &Ingest,
    source: &Source,
    found: &Extracted,
    report: &mut Report,
) -> Result<(), DistilError> {
    for candidate in &found.candidates {
        let score = candidate.score(|kind| settings.weight(kind));
        let verdict = decide(engine, settings, candidate, score);

        match verdict {
            Verdict::Refuse { reason } => report.refused.push((candidate.text(), reason)),
            Verdict::Hold => report.held += 1,
            Verdict::Promote { importance, pinned } => {
                if ask.dry_run {
                    report.promoted += 1;
                    continue;
                }
                let landing = write(store, ask, source, candidate, importance, pinned)?;
                match landing {
                    Landing::Added(_) => report.promoted += 1,
                    Landing::Reinforced(_) => report.reinforced += 1,
                    Landing::Superseded { .. } => report.superseded += 1,
                }
            }
        }
    }
    Ok(())
}

/// The floors first, then whatever the configuration wants to say about it.
///
/// A handler may promote something the floors would have held, refuse something they would
/// have promoted, or amend how fast it fades. What it must not do is go unheard, which is why
/// this runs on every candidate rather than only on the borderline ones.
fn decide(engine: &mut Engine, settings: &Settings, candidate: &Candidate, score: f64) -> Verdict {
    let floors = settings.floors();
    let mut verdict = weigh(score, floors.promote, floors.hold);
    // Somebody insisting is not asking for a memory that decays. Applied before the
    // configuration sees it, so a gate can still overrule.
    if candidate.pinned
        && let Verdict::Promote { importance, pinned } = &mut verdict
    {
        *importance = candidate.importance;
        *pinned = true;
    }

    let Some(said) = engine.ask("promote", &[candidate.as_json()]) else {
        return verdict;
    };

    if said.get("promote").and_then(serde_json::Value::as_bool) == Some(false) {
        return Verdict::Refuse {
            reason: said
                .get("reason")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("a gate refused it")
                .to_owned(),
        };
    }
    if said.get("promote").and_then(serde_json::Value::as_bool) == Some(true) {
        verdict = Verdict::Promote {
            importance: candidate.importance,
            pinned: false,
        };
    }
    if let Verdict::Promote { importance, pinned } = &mut verdict {
        if let Some(said) = said.get("importance").and_then(serde_json::Value::as_str)
            && let Ok(parsed) = said.parse()
        {
            *importance = parsed;
        }
        if let Some(said) = said.get("pinned").and_then(serde_json::Value::as_bool) {
            *pinned = said;
        }
    }
    verdict
}

/// Write one candidate, with the evidence that earned it.
fn write(
    store: &mut Store,
    ask: &Ingest,
    source: &Source,
    candidate: &Candidate,
    importance: memo_model::Importance,
    pinned: bool,
) -> Result<Landing, DistilError> {
    let session = SessionId::new(&source.meta.id);
    // The claim dates from when it happened, not from when the backfill ran. Six months of
    // transcripts read this afternoon did not all become true this afternoon.
    let happened = source.meta.opened.max(0);
    let mut memory = Memory::new(
        mint(ask.now),
        candidate.tier,
        ask.scope.clone(),
        candidate.body.clone(),
        ask.now,
    );
    memory.temporal = memo_model::Temporal::recalled(ask.now, happened);
    memory.session = Some(session.clone());
    memory.strength.importance = importance;
    memory.strength.pinned = pinned;
    memory.provenance = memo_model::Provenance {
        through: memo_model::Through::Ingest,
        who: Some(ask.source.clone()),
    };

    let witness = Witness::new(
        WitnessId::new(format!(
            "{}:{}:{}",
            candidate.from,
            candidate.cursor.unwrap_or(0),
            &memory.content_hash[..8]
        )),
        candidate.witness,
        session,
        ask.scope.clone(),
        happened,
    )
    .noted(format!("{} (rules, not a model)", candidate.from));
    let witness = match candidate.cursor {
        Some(cursor) => witness.at_cursor(cursor),
        None => witness,
    };

    Ok(store.remember(memory, witness, ask.now)?)
}
