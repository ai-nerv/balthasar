//! The one place a row is removed.
//!
//! `gate-no-delete` greps for `DELETE FROM` outside this file and the ledger's retention. A
//! memory is not one row: evidence, asserted and derived edges, an entity index, a full-text
//! row, an embedding and a ledger trail all go with it, and some can reconstruct what was purged.

use crate::{Store, StoreError};
use balthasar_model::MemoryId;
use rusqlite::params;

/// What a purge would remove, counted before anything goes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Closure {
    pub witnesses: usize,
    /// Asserted links, in either direction.
    pub links: usize,
    /// Derived relationships, in either direction.
    pub relations: usize,
    pub entities: usize,
    pub ledger: usize,
    pub embedded: bool,
    /// Memories distilled or consolidated out of this one, which go with it.
    pub derived: usize,
}

impl Closure {
    /// How many rows in total, beside the memory itself.
    #[must_use]
    pub fn rows(&self) -> usize {
        self.witnesses + self.links + self.relations + self.entities + self.ledger + self.derived
    }
}

/// What removing `id` would take with it.
pub fn closure_of(store: &Store, id: &MemoryId) -> Result<Closure, StoreError> {
    let one = |sql: &str| -> Result<usize, StoreError> {
        let n: i64 = store
            .db()
            .query_row(sql, params![id.as_str()], |r| r.get(0))?;
        Ok(n.max(0) as usize)
    };
    Ok(Closure {
        witnesses: one("SELECT count(*) FROM witness WHERE memory = ?1")?,
        links: one("SELECT count(*) FROM link WHERE src = ?1 OR dst = ?1")?,
        relations: one(
            "SELECT count(*) FROM relation_view WHERE from_memory = ?1 OR to_memory = ?1",
        )?,
        entities: one("SELECT count(*) FROM entity WHERE memory = ?1")?,
        ledger: one(
            "SELECT (SELECT count(*) FROM recall_candidate WHERE memory_id = ?1) \
                  + (SELECT count(*) FROM injection_memory WHERE memory_id = ?1) \
                  + (SELECT count(*) FROM action_memory WHERE memory_id = ?1)",
        )?,
        embedded: one("SELECT count(*) FROM memory WHERE id = ?1 AND embedding IS NOT NULL")? > 0,
        derived: descendants(store, id)?.len(),
    })
}

impl Store {
    /// Every memory a run owns. Owning is not having seen: a memory another run wrote and this
    /// one merely witnessed belongs to the other.
    pub fn owned_by(
        &self,
        session: &balthasar_model::SessionId,
    ) -> Result<Vec<MemoryId>, StoreError> {
        let mut statement = self
            .db()
            .prepare("SELECT id FROM memory WHERE session = ?1")?;
        let found = statement
            .query_map(params![session.as_str()], |r| {
                Ok(MemoryId::new(r.get::<_, String>(0)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(found)
    }
}

/// Remove one run's own scratch, permanently. The run's directory, not one agent's: a run may
/// have several agents, and each has its own scratch file under the run.
pub fn purge_scratch(
    pad: &mut crate::Scratchpad,
    session: &balthasar_model::SessionId,
) -> Result<bool, StoreError> {
    let dir = crate::run_dir_in(pad.home(), session);
    if !dir.is_dir() {
        return Ok(false);
    }
    // Before removing the files, not after: an open connection to a file that has stopped
    // existing is a store that answers questions out of a deleted inode.
    pad.close(session);
    std::fs::remove_dir_all(dir).map_err(|why| StoreError::Foreign(why.to_string()))?;
    Ok(true)
}

/// Remove one run's turns from the scrollback, permanently. The scrollback is a separate file,
/// so `purge_session` cannot reach it and a caller answering "forget that session" does both.
pub fn purge_run(
    scrollback: &crate::Transcript,
    session: &balthasar_model::SessionId,
) -> Result<usize, StoreError> {
    let tx = rusqlite::Transaction::new_unchecked(
        scrollback.db(),
        rusqlite::TransactionBehavior::Immediate,
    )?;
    let transcripts = {
        let mut query =
            tx.prepare("SELECT session FROM transcript_run WHERE run = ?1 UNION SELECT ?1")?;
        query
            .query_map([session.as_str()], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
    };
    let maintenance = maintenance(scrollback)?;
    if let Some(maintenance) = &maintenance {
        for transcript in &transcripts {
            if !maintenance.available(transcript)? {
                return Err(StoreError::Foreign(
                    "a transcript in this run has a writer".into(),
                ));
            }
        }
    }
    let mut gone = 0;
    for transcript in transcripts {
        gone += purge_transcript(scrollback, &balthasar_model::SessionId::new(transcript))?;
    }
    tx.commit()?;
    optimize(scrollback.db(), "turn_fts")?;
    Ok(gone)
}

pub(crate) fn maintenance(
    scrollback: &crate::Transcript,
) -> Result<Option<crate::writer::Maintenance>, StoreError> {
    if !scrollback.is_persistent() {
        return Ok(None);
    }
    scrollback
        .maintenance()?
        .map(Some)
        .ok_or_else(|| StoreError::Foreign("transcript reservation admission is busy".into()))
}

fn purge_transcript(
    scrollback: &crate::Transcript,
    session: &balthasar_model::SessionId,
) -> Result<usize, StoreError> {
    // The search index holds a second copy of every word, so it goes before the turn.
    scrollback.db().execute(
        "DELETE FROM turn_fts WHERE session = ?1",
        params![session.as_str()],
    )?;
    let gone = scrollback.db().execute(
        "DELETE FROM turn WHERE session = ?1",
        params![session.as_str()],
    )?;
    // Summaries, job inputs, proposed layouts, earlier revisions, traces, generations and note
    // changes quote the turns, so they go with them.
    for table in [
        "turn_revision",
        "trace",
        "generation",
        "chunk",
        "note_change",
        "extraction_progress",
        "extraction_ack",
        "layout_effect",
        "layout",
        "summary",
        "prompt",
        "job",
        "counter",
        "run",
        "run_model",
        "transcript_run",
    ] {
        scrollback.db().execute(
            &format!("DELETE FROM {table} WHERE session = ?1"),
            params![session.as_str()],
        )?;
    }
    forget_cited(scrollback.db(), session.as_str())?;
    Ok(gone)
}

/// A note made only of `session`'s turns goes, with its changes; one citing other runs too
/// keeps its text and loses this run's citations.
fn forget_cited(db: &rusqlite::Connection, session: &str) -> Result<usize, StoreError> {
    let held: Vec<(i64, String)> = {
        let mut statement = db.prepare("SELECT n, evidence FROM note")?;
        statement
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?
    };
    let mut gone = 0;
    for (n, evidence) in held {
        let Ok(mut evidence) = serde_json::from_str::<serde_json::Value>(&evidence) else {
            continue;
        };
        let Some(sources) = evidence
            .get_mut("sources")
            .and_then(serde_json::Value::as_array_mut)
        else {
            continue;
        };
        let before = sources.len();
        sources.retain(|s| s.get("session").and_then(serde_json::Value::as_str) != Some(session));
        if sources.len() == before {
            continue;
        }
        if sources.is_empty() {
            db.execute(
                "DELETE FROM note_change WHERE note = ?1",
                params![format!("N-{n}")],
            )?;
            gone += db.execute("DELETE FROM note WHERE n = ?1", params![n])?;
        } else {
            db.execute(
                "UPDATE note SET evidence = ?2 WHERE n = ?1",
                params![n, evidence.to_string()],
            )?;
        }
    }
    Ok(gone)
}

/// Remove a memory and everything that points at it, permanently.
///
/// Order matters: everything referring to the memory goes before the memory, because the foreign
/// keys point that way. The embedding is a column and goes with the row. Derivations go too, and
/// descendants are removed first, so no edge is ever left pointing at nothing.
pub fn purge(store: &mut Store, id: &MemoryId) -> Result<usize, StoreError> {
    let gone = purge_tree(store, id)?;
    optimize(store.db(), "memory_fts")?;
    Ok(gone)
}

/// [`purge`], leaving the index's merge to the caller.
fn purge_tree(store: &mut Store, id: &MemoryId) -> Result<usize, StoreError> {
    let mut gone = 0;
    for derived in descendants(store, id)? {
        gone += purge_one(store, &derived)?;
    }
    Ok(gone + purge_one(store, id)?)
}

/// Merge a full-text index's segments, so a deleted row's tokens leave its posting lists.
fn optimize(db: &rusqlite::Connection, index: &str) -> Result<(), StoreError> {
    db.execute(
        &format!("INSERT INTO {index}({index}) VALUES ('optimize')"),
        [],
    )?;
    Ok(())
}

/// Everything made out of this memory, deepest first. Breadth-first with a seen set, so a cycle
/// terminates; reversed on the way out, so a descendant is purged before its source.
fn descendants(store: &Store, root: &MemoryId) -> Result<Vec<MemoryId>, StoreError> {
    let mut seen = vec![root.clone()];
    let mut order = Vec::new();
    let mut queue = vec![root.clone()];

    while let Some(here) = queue.pop() {
        let mut statement = store
            .db()
            .prepare("SELECT src FROM link WHERE dst = ?1 AND rel = 'derived_from'")?;
        let made: Vec<MemoryId> = statement
            .query_map(params![here.as_str()], |r| {
                Ok(MemoryId::new(r.get::<_, String>(0)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for id in made {
            if !seen.contains(&id) {
                seen.push(id.clone());
                order.push(id.clone());
                queue.push(id);
            }
        }
    }
    order.reverse();
    Ok(order)
}

/// One memory, and everything that points at it.
fn purge_one(store: &mut Store, id: &MemoryId) -> Result<usize, StoreError> {
    let tx = store.db_mut().transaction()?;
    let gone = remove(&tx, id)?;
    tx.commit()?;
    Ok(gone)
}

/// The rows of [`purge_one`], inside a transaction the caller holds.
fn remove(tx: &rusqlite::Connection, id: &MemoryId) -> Result<usize, StoreError> {
    // Asserted edges, in both directions.
    tx.execute(
        "DELETE FROM link WHERE src = ?1 OR dst = ?1",
        params![id.as_str()],
    )?;
    // Derived edges. No foreign key holds these, so nothing would complain about a dangling row.
    tx.execute(
        "DELETE FROM relation_view WHERE from_memory = ?1 OR to_memory = ?1",
        params![id.as_str()],
    )?;
    // The entity index. It has a foreign key, so leaving it out fails the purge outright.
    tx.execute("DELETE FROM entity WHERE memory = ?1", params![id.as_str()])?;
    // The ledger's trail. These carry no content, but they name the id.
    tx.execute(
        "DELETE FROM recall_candidate WHERE memory_id = ?1",
        params![id.as_str()],
    )?;
    tx.execute(
        "DELETE FROM injection_memory WHERE memory_id = ?1",
        params![id.as_str()],
    )?;
    tx.execute(
        "DELETE FROM action_memory WHERE memory_id = ?1",
        params![id.as_str()],
    )?;
    tx.execute(
        "DELETE FROM witness WHERE memory = ?1",
        params![id.as_str()],
    )?;
    tx.execute("DELETE FROM memory_fts WHERE id = ?1", params![id.as_str()])?;

    Ok(tx.execute("DELETE FROM memory WHERE id = ?1", params![id.as_str()])?)
}

/// Everything one run left behind. Returns how many memories went. Memories a run merely
/// *witnessed* are not removed: its witness from this run goes, and the fact stays with the
/// evidence that remains, scored again at `now`.
pub fn purge_session(
    store: &mut Store,
    session: &balthasar_model::SessionId,
    now: balthasar_model::Timestamp,
) -> Result<usize, StoreError> {
    let owned = store.owned_by(session)?;

    let mut gone = 0;
    for id in &owned {
        gone += purge_tree(store, id)?;
    }
    let witnessed: Vec<MemoryId> = {
        let mut statement = store.db().prepare(
            "SELECT DISTINCT w.memory FROM witness w JOIN memory m ON m.id = w.memory \
                 WHERE w.session = ?1 ORDER BY w.memory",
        )?;
        statement
            .query_map(params![session.as_str()], |r| {
                Ok(MemoryId::new(r.get::<_, String>(0)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };

    // The run's own evidence for things it did not own, and its bookkeeping.
    let tx = store.db_mut().transaction()?;
    tx.execute(
        "DELETE FROM witness WHERE session = ?1",
        params![session.as_str()],
    )?;
    tx.execute(
        "DELETE FROM episode_segment WHERE session = ?1",
        params![session.as_str()],
    )?;
    // The ledger's rows for the run, children before what they point at; one another run's
    // action still points at stays.
    for statement in [
        "DELETE FROM outcome WHERE action_id IN (SELECT id FROM action_use WHERE session = ?1)",
        "DELETE FROM action_memory WHERE action_id IN \
         (SELECT id FROM action_use WHERE session = ?1)",
        "DELETE FROM action_use WHERE session = ?1",
        "DELETE FROM injection_memory WHERE injection_id IN \
         (SELECT id FROM injection WHERE session = ?1 \
          AND id NOT IN (SELECT injection_id FROM action_use WHERE injection_id IS NOT NULL))",
        "DELETE FROM injection WHERE session = ?1 \
         AND id NOT IN (SELECT injection_id FROM action_use WHERE injection_id IS NOT NULL)",
        "DELETE FROM recall_candidate WHERE recall_id IN \
         (SELECT id FROM recall_run WHERE session = ?1 \
          AND id NOT IN (SELECT recall_id FROM injection WHERE recall_id IS NOT NULL))",
        "DELETE FROM recall_run WHERE session = ?1 \
         AND id NOT IN (SELECT recall_id FROM injection WHERE recall_id IS NOT NULL)",
    ] {
        tx.execute(statement, params![session.as_str()])?;
    }
    tx.execute(
        "DELETE FROM session WHERE id = ?1",
        params![session.as_str()],
    )?;
    tx.commit()?;
    for id in &witnessed {
        store.rescore(id, now)?;
    }
    optimize(store.db(), "memory_fts")?;
    Ok(gone)
}

/// Everything that came from one source: every memory whose evidence comes only from `domain`. A
/// memory with evidence from elsewhere as well is kept, with the tainted witness removed, so one
/// poisoned source cannot take honest memories with it; it is scored again at `now`.
pub fn purge_domain(
    store: &mut Store,
    domain: &balthasar_model::Domain,
    now: balthasar_model::Timestamp,
) -> Result<usize, StoreError> {
    let touched: Vec<MemoryId> = {
        let mut statement = store
            .db()
            .prepare("SELECT DISTINCT memory FROM witness WHERE domain = ?1")?;
        statement
            .query_map(params![domain.as_str()], |r| {
                Ok(MemoryId::new(r.get::<_, String>(0)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };

    let mut gone = 0;
    for id in &touched {
        let total: i64 = store.db().query_row(
            "SELECT count(*) FROM witness WHERE memory = ?1",
            params![id.as_str()],
            |r| r.get(0),
        )?;
        let tainted: i64 = store.db().query_row(
            "SELECT count(*) FROM witness WHERE memory = ?1 AND domain = ?2",
            params![id.as_str(), domain.as_str()],
            |r| r.get(0),
        )?;

        if tainted >= total {
            gone += purge_tree(store, id)?;
        } else {
            store.db().execute(
                "DELETE FROM witness WHERE memory = ?1 AND domain = ?2",
                params![id.as_str(), domain.as_str()],
            )?;
            // Confidence is derived, so it has to be recomputed now that the evidence changed.
            store.rescore(id, now)?;
        }
    }
    optimize(store.db(), "memory_fts")?;
    Ok(gone)
}

/// How long a layout is kept once a provider has taken it: thirty days.
pub const LAYOUT_KEPT: balthasar_model::Timestamp = 30 * 24 * 60 * 60;

/// Remove layouts applied before `before`, except each run's newest and any whose effects are
/// still waiting to be carried out. Answers how many went.
pub fn prune_layouts(
    scrollback: &crate::Transcript,
    before: balthasar_model::Timestamp,
) -> Result<usize, StoreError> {
    Ok(scrollback.db().execute(
        "DELETE FROM layout WHERE applied IS NOT NULL AND applied < ?1 \
         AND n NOT IN (SELECT max(n) FROM layout GROUP BY session) \
         AND ('L-' || n) NOT IN (SELECT layout FROM layout_effect WHERE done = 0)",
        params![before],
    )?)
}

/// Seconds in a day, the unit turn retention is counted in.
pub const DAY: balthasar_model::Timestamp = 24 * 60 * 60;

/// A session a retention pass would empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expired {
    pub session: balthasar_model::SessionId,
    /// Turns it holds, of which `cited` are a note's evidence and stay.
    pub turns: u64,
    pub cited: u64,
    /// When anything last happened in it.
    pub last: balthasar_model::Timestamp,
}

/// What a retention pass removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pruned {
    pub sessions: usize,
    pub turns: usize,
    /// Scratch copies of what was said, in the runs' files and the project's store.
    pub mirrors: usize,
}

/// When anything last happened in each session idle since `?1`.
const IDLE: &str = "SELECT session, max(at) FROM ( \
     SELECT session, at FROM turn UNION ALL SELECT session, at FROM trace \
     UNION ALL SELECT session, max(at, COALESCE(applied, at)) FROM layout \
     UNION ALL SELECT session, max(created, COALESCE(settled, created)) FROM job \
     UNION ALL SELECT session, opened FROM run) \
   GROUP BY session HAVING max(at) < ?1";

/// Rows of the sessions named in the JSON list `?1`.
const EXPIRED: &str = "session IN (SELECT j.value FROM json_each(?1) j)";

/// Rows of those sessions at a cursor no `[session, cursor]` pair in `?2` names.
const UNCITED: &str = "session IN (SELECT j.value FROM json_each(?1) j) AND (session, cursor) \
     NOT IN (SELECT json_extract(j.value, '$[0]'), json_extract(j.value, '$[1]') \
             FROM json_each(?2) j)";

/// What a person or the model said, the turns a scratch copy is kept of.
const SAID: &str = "role IN ('user', 'assistant') AND kind NOT IN ('tool', 'tool_result') \
     AND text != ''";

/// The sessions [`prune_turns`] would empty: idle since `before`, not in `live`, and holding a
/// trace or a turn no note cites.
pub fn expired(
    scrollback: &crate::Transcript,
    before: balthasar_model::Timestamp,
    live: &[balthasar_model::SessionId],
) -> Result<Vec<Expired>, StoreError> {
    expired_in(scrollback.db(), before, live)
}

fn expired_in(
    db: &rusqlite::Connection,
    before: balthasar_model::Timestamp,
    live: &[balthasar_model::SessionId],
) -> Result<Vec<Expired>, StoreError> {
    let cited = cited(db)?;
    let idle = {
        let mut statement = db.prepare(IDLE)?;
        statement
            .query_map(params![before], |r| Ok((r.get::<_, String>(0)?, r.get(1)?)))?
            .collect::<Result<Vec<(String, i64)>, _>>()?
    };
    let mut out = Vec::new();
    for (session, last) in idle {
        if live.iter().any(|held| held.as_str() == session) {
            continue;
        }
        let mine: Vec<u64> = cited
            .iter()
            .filter(|(at, _)| *at == session)
            .map(|(_, cursor)| *cursor)
            .collect();
        let (turns, kept, traces): (i64, i64, i64) = db.query_row(
            "SELECT count(*), COALESCE(sum(cursor IN (SELECT j.value FROM json_each(?2) j)), 0), \
                    (SELECT count(*) FROM trace WHERE session = ?1) \
             FROM turn WHERE session = ?1",
            params![session, serde_json::to_string(&mine)?],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        if turns > kept || traces > 0 {
            out.push(Expired {
                session: balthasar_model::SessionId::new(session),
                turns: turns.max(0) as u64,
                cited: kept.max(0) as u64,
                last,
            });
        }
    }
    Ok(out)
}

/// Every turn a note or a change to one cites as evidence, as `(session, cursor)`.
fn cited(db: &rusqlite::Connection) -> Result<Vec<(String, u64)>, StoreError> {
    let mut statement =
        db.prepare("SELECT evidence FROM note UNION ALL SELECT evidence FROM note_change")?;
    let held = statement
        .query_map([], |r| r.get::<_, Option<String>>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut out = Vec::new();
    for evidence in held.iter().flatten() {
        let Ok(evidence) = serde_json::from_str::<serde_json::Value>(evidence) else {
            continue;
        };
        let sources = evidence
            .get("sources")
            .and_then(serde_json::Value::as_array);
        for source in sources.into_iter().flatten() {
            let session = source.get("session").and_then(serde_json::Value::as_str);
            let cursor = source.get("cursor").and_then(serde_json::Value::as_u64);
            if let (Some(session), Some(cursor)) = (session, cursor) {
                out.push((session.to_owned(), cursor));
            }
        }
    }
    Ok(out)
}

/// Remove every turn of each session idle since `before` and not in `live`, with every copy of
/// it: its search entry, revisions, chunks and traces, the layouts, summaries, generations and
/// jobs made from it, and the scratch copies in `scratch` and `project` that no remaining turn of
/// the run still says. A turn a note cites stays, and so does its session's record. One
/// `IMMEDIATE` transaction on the scrollback, held while the scratch copies go.
pub fn prune_turns(
    scrollback: &crate::Transcript,
    mut scratch: Option<&mut crate::Scratchpad>,
    mut project: Option<&mut Store>,
    before: balthasar_model::Timestamp,
    live: &[balthasar_model::SessionId],
) -> Result<Pruned, StoreError> {
    let tx = rusqlite::Transaction::new_unchecked(
        scrollback.db(),
        rusqlite::TransactionBehavior::Immediate,
    )?;
    let mut expired = expired_in(&tx, before, live)?;
    let maintenance = maintenance(scrollback)?;
    if let Some(maintenance) = &maintenance {
        let mut available = Vec::new();
        for entry in expired {
            if maintenance.available(entry.session.as_str())? {
                available.push(entry);
            }
        }
        expired = available;
    }
    if expired.is_empty() {
        return Ok(Pruned::default());
    }
    let sessions: Vec<&str> = expired.iter().map(|e| e.session.as_str()).collect();
    let sessions = serde_json::to_string(&sessions)?;
    let cited = serde_json::to_string(&cited(&tx)?)?;
    let said = said_by_run(&tx, &sessions, &cited)?;

    let mut pruned = Pruned {
        sessions: expired.len(),
        ..Pruned::default()
    };
    for table in ["turn_fts", "turn_revision", "chunk", "turn"] {
        let gone = tx.execute(
            &format!("DELETE FROM {table} WHERE {UNCITED}"),
            params![sessions, cited],
        )?;
        if table == "turn" {
            pruned.turns = gone;
        }
    }
    for table in [
        "trace",
        "layout_effect",
        "layout",
        "summary",
        "generation",
        "job",
    ] {
        tx.execute(
            &format!("DELETE FROM {table} WHERE {EXPIRED}"),
            params![sessions],
        )?;
    }
    // A session left holding a cited turn keeps its record.
    for table in [
        "extraction_progress",
        "extraction_ack",
        "prompt",
        "counter",
        "run_model",
        "run",
        "transcript_run",
    ] {
        tx.execute(
            &format!(
                "DELETE FROM {table} WHERE {EXPIRED} \
                 AND NOT EXISTS (SELECT 1 FROM turn WHERE turn.session = {table}.session)"
            ),
            params![sessions],
        )?;
    }

    for (run, mut hashes) in said {
        let still = said_in_run(&tx, &run)?;
        hashes.retain(|hash| !still.contains(hash));
        if hashes.is_empty() {
            continue;
        }
        if let Some(pad) = scratch.as_deref_mut() {
            for agent in pad.agents_of(&balthasar_model::SessionId::new(&run)) {
                let store = pad.of(&balthasar_model::SessionId::new(&run), &agent)?;
                pruned.mirrors += forget_mirrors(store, &run, &hashes)?;
            }
        }
        if let Some(store) = project.as_deref_mut() {
            pruned.mirrors += forget_mirrors(store, &run, &hashes)?;
        }
    }
    tx.commit()?;
    if pruned.turns > 0 {
        optimize(scrollback.db(), "turn_fts")?;
    }
    Ok(pruned)
}

/// The content hash of each uncited said turn and revision of the sessions in `sessions`, by
/// the run its scratch copy was kept under.
fn said_by_run(
    db: &rusqlite::Connection,
    sessions: &str,
    cited: &str,
) -> Result<std::collections::BTreeMap<String, Vec<String>>, StoreError> {
    let mut statement = db.prepare(&format!(
        "SELECT COALESCE((SELECT run FROM transcript_run b WHERE b.session = t.session), \
                t.session), t.text \
         FROM (SELECT session, cursor, role, kind, text FROM turn UNION ALL \
               SELECT session, cursor, role, kind, text FROM turn_revision) t \
         WHERE {UNCITED} AND {SAID}"
    ))?;
    let rows = statement
        .query_map(params![sessions, cited], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut out: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for (run, text) in rows {
        let hashes = out.entry(run).or_default();
        let hash = balthasar_model::content_hash(&text);
        if !hashes.contains(&hash) {
            hashes.push(hash);
        }
    }
    Ok(out)
}

/// The content hash of everything still said in `run`, by any session bound to it.
fn said_in_run(
    db: &rusqlite::Connection,
    run: &str,
) -> Result<std::collections::HashSet<String>, StoreError> {
    let mut statement = db.prepare(&format!(
        "SELECT text FROM (SELECT session, role, kind, text FROM turn UNION ALL \
                           SELECT session, role, kind, text FROM turn_revision) \
         WHERE session IN (SELECT session FROM transcript_run WHERE run = ?1 UNION SELECT ?1) \
         AND {SAID}"
    ))?;
    let texts = statement
        .query_map(params![run], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(texts
        .iter()
        .map(|text| balthasar_model::content_hash(text))
        .collect())
}

/// Remove the scratch memories `run` kept of what it said, each with its closure.
fn forget_mirrors(store: &mut Store, run: &str, hashes: &[String]) -> Result<usize, StoreError> {
    let tx = store.db_mut().transaction()?;
    let mut gone = 0;
    for hash in hashes {
        let ids = {
            let mut statement = tx.prepare_cached(
                "SELECT id FROM memory WHERE session = ?1 AND tier = 'scratch' \
                 AND content_hash = ?2",
            )?;
            statement
                .query_map(params![run, hash], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        for id in ids {
            gone += remove(&tx, &MemoryId::new(id))?;
        }
    }
    tx.commit()?;
    Ok(gone)
}
