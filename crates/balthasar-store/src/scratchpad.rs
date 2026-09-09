//! One agent's scratch, in its own file.
//!
//! A session's memories are the session's own until something on the ladder carries them
//! across, so they live in that run's directory rather than as rows in the project's store
//! wearing a `session` column. What that buys is deletion: removing one run is removing one
//! directory, and it takes the scrollback with it.
//!
//! **The agent is the second half of the key.** Several agents work inside one run, and one
//! subagent's working notes are not another's to read — so scratch is `<session>/<agent>` and
//! an agent's identity is pinned to its connection rather than passed per call. What the agents
//! of a run share is the project's store, which is the ladder: scratch is private, a promoted
//! fact is everybody's.
//!
//! What it costs is that promotion crosses a database boundary, which is [`Scratchpad::carry`].
//! There is no transaction spanning the two files and there does not need to be — see the note
//! there.

use crate::{Store, StoreError};
use balthasar_model::{AgentId, SessionId};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Every agent's scratch under one tool's home, opened as it is needed.
///
/// Held open for as long as the process is: a session writes many times, and reopening the file
/// per turn would be paying SQLite's setup cost for nothing.
pub struct Scratchpad {
    home: PathBuf,
    open: HashMap<(SessionId, AgentId), Store>,
}

impl Scratchpad {
    /// Scratch beneath a tool's home — `<project>/balthasar/<tool>`.
    ///
    /// Brings an older tree forward on the way in, which is the only moment anything looks at
    /// the whole of it. Swallowed because a failed move leaves every run whole — see
    /// [`crate::layout`], which undoes a part-moved one — so the scratch is merely invisible
    /// until the next process retries, and that is recoverable where losing it is not.
    #[must_use]
    pub fn at(home: impl Into<PathBuf>) -> Self {
        let home = home.into();
        let _ = crate::layout::bring_forward(&home);
        Self {
            home,
            open: HashMap::new(),
        }
    }

    /// Where this scratchpad keeps its runs.
    #[must_use]
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Where one agent of `session` keeps its scratch.
    #[must_use]
    pub fn path_of(&self, session: &SessionId, agent: &AgentId) -> PathBuf {
        crate::session_dir_in(&self.home, session, agent).join("memory.db")
    }

    /// The store holding this agent's scratch, creating it on its first write.
    ///
    /// Creation is deliberately here and not at session start: a harness that opens a session
    /// and says nothing should leave nothing behind, and an `balthasar/` tree full of empty
    /// directories is what the alternative looks like after a week.
    pub fn of(&mut self, session: &SessionId, agent: &AgentId) -> Result<&mut Store, StoreError> {
        let key = (session.clone(), agent.clone());
        if !self.open.contains_key(&key) {
            let store = Store::open(&self.path_of(session, agent))?;
            self.open.insert(key.clone(), store);
        }
        self.open
            .get_mut(&key)
            .ok_or_else(|| StoreError::Foreign("session".to_owned()))
    }

    /// The store holding this agent's scratch, if it has ever written.
    ///
    /// For readers. A recall must not bring a run's directory into being merely by looking for
    /// it, which is what [`Scratchpad::of`] would do.
    pub fn peek(
        &mut self,
        session: &SessionId,
        agent: &AgentId,
    ) -> Result<Option<&mut Store>, StoreError> {
        let key = (session.clone(), agent.clone());
        if !self.open.contains_key(&key) && !self.path_of(session, agent).is_file() {
            return Ok(None);
        }
        self.of(session, agent).map(Some)
    }

    /// Every agent that has left scratch behind in one run, in a stable order.
    ///
    /// From the directory rather than from a register, so an agent this process has never
    /// spoken to is still found. Names that had to be mangled to be directories come back
    /// mangled — enough to open the file, which is what every caller wants them for.
    #[must_use]
    pub fn agents_of(&self, session: &SessionId) -> Vec<AgentId> {
        let Ok(entries) = std::fs::read_dir(crate::run_dir_in(&self.home, session)) else {
            return Vec::new();
        };
        let mut found: Vec<AgentId> = entries
            .flatten()
            .filter(|e| e.path().join("memory.db").is_file())
            .map(|e| AgentId::new(e.file_name().to_string_lossy().into_owned()))
            .collect();
        found.sort();
        found
    }

    /// Let go of every store a run has open, so its files can be moved or removed.
    ///
    /// Only [`purge`](crate::purge) has reason to call this: an open connection to a file that
    /// is about to stop existing would hand the next caller a store backed by nothing. Every
    /// agent of the run, because forgetting a run forgets all of it.
    pub(crate) fn close(&mut self, session: &SessionId) {
        self.open.retain(|(held, _), _| held != session);
    }

    /// Every scratch file under this home, oldest first.
    ///
    /// **Two levels, run then agent.** The layout gained an agent segment and this walk did
    /// not, so it looked for `<session>/memory.db`, found none, and returned an empty vector —
    /// `Ok`-shaped emptiness, not an error. Everything downstream reads this: decay, sweeping
    /// and corroboration all stopped and every one of them reported success.
    ///
    /// Directory names are harness names that survived being one, so this is the listing and
    /// not the identities: a name that had to be mangled to be a directory cannot be turned
    /// back. Callers that need the identity open the store and read its session rows.
    #[must_use]
    pub fn runs(&self) -> Vec<PathBuf> {
        let Ok(runs) = std::fs::read_dir(&self.home) else {
            return Vec::new();
        };
        let mut found: Vec<PathBuf> = runs
            .flatten()
            .filter_map(|run| std::fs::read_dir(run.path()).ok())
            .flat_map(|agents| {
                agents
                    .flatten()
                    .map(|agent| agent.path().join("memory.db"))
                    .filter(|path| path.is_file())
                    .collect::<Vec<_>>()
            })
            .collect();
        found.sort();
        found
    }

    /// Scratch saying the same thing in `at_least` different runs, across every run's file.
    ///
    /// CALLUS, now that a run's scratch is not a set of rows one query can group. Deliberately
    /// **not** `ATTACH`: SQLite's default `SQLITE_MAX_ATTACHED` is 10, so attaching would fail
    /// at exactly the size where corroboration starts to matter. Reading each run and grouping
    /// here scales instead, because one run's scratch is bounded by what fit a context window.
    ///
    /// Bounded twice. `since` skips runs whose scratch has either crossed already or decayed
    /// out of the live set, and `cap` limits how many files one pass opens — newest first, so a
    /// project with ten thousand runs makes progress every pass rather than timing out.
    pub fn recurring(
        &self,
        scope: &str,
        at_least: usize,
        since: balthasar_model::Timestamp,
        cap: usize,
    ) -> Result<Vec<crate::Cluster>, StoreError> {
        let mut seen: HashMap<String, (String, balthasar_model::Timestamp, Vec<SessionId>)> =
            HashMap::new();

        for path in self.newest(cap) {
            let store = Store::open(&path)?;
            let mut statement = store.db().prepare(
                "SELECT content_hash, text, observed_at, session FROM memory \
                 WHERE scope = ?1 AND tier = 'scratch' AND archived_at IS NULL \
                   AND session IS NOT NULL AND text != '' AND observed_at >= ?2",
            )?;
            let rows = statement
                .query_map(rusqlite::params![scope, since], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, balthasar_model::Timestamp>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;

            for (hash, text, at, session) in rows {
                let held = seen.entry(hash).or_insert_with(|| (text, at, Vec::new()));
                held.1 = held.1.min(at);
                let run = SessionId::new(session);
                if !held.2.contains(&run) {
                    held.2.push(run);
                }
            }
        }

        let mut out: Vec<crate::Cluster> = seen
            .into_iter()
            .filter(|(_, (_, _, runs))| runs.len() >= at_least)
            .map(|(hash, (text, first_seen, sessions))| crate::Cluster {
                // Left empty on purpose: these ids live in each run's own file, and a link row
                // in the project's store cannot reference them. What a promoted claim keeps
                // instead is the session, which `purge_session` already follows.
                sources: Vec::new(),
                text,
                hash,
                sessions,
                first_seen,
            })
            .collect();
        out.sort_by(|a, b| {
            b.sessions
                .len()
                .cmp(&a.sessions.len())
                .then(a.first_seen.cmp(&b.first_seen))
                .then(a.hash.cmp(&b.hash))
        });
        Ok(out)
    }

    /// The most recently written runs, newest first.
    fn newest(&self, cap: usize) -> Vec<PathBuf> {
        let mut held: Vec<(std::time::SystemTime, PathBuf)> = self
            .runs()
            .into_iter()
            .map(|path| {
                let when = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH);
                (when, path)
            })
            .collect();
        // Newest first, so the key is reversed rather than the comparison.
        held.sort_by_key(|(when, _)| std::cmp::Reverse(*when));
        held.into_iter().take(cap).map(|(_, path)| path).collect()
    }

    /// Let every run's scratch fade, and archive what has fallen past the floor.
    ///
    /// The same two steps the project's store takes, in the same order and for the same reason:
    /// sweeping before the ladder has looked would take away the scratch it was about to find
    /// corroboration in.
    pub fn weaken_all(&mut self, now: balthasar_model::Timestamp) -> Result<usize, StoreError> {
        let mut faded = 0;
        for path in self.runs() {
            let mut store = Store::open(&path)?;
            faded += store.weaken(now)?.weakened.len();
        }
        Ok(faded)
    }

    /// Archive what every run's scratch no longer holds up.
    pub fn sweep_all(&mut self, now: balthasar_model::Timestamp) -> Result<usize, StoreError> {
        let mut swept = 0;
        for path in self.runs() {
            let mut store = Store::open(&path)?;
            swept += store.sweep(now)?.swept.len();
        }
        Ok(swept)
    }

    /// Carry a scratch memory into the project's store.
    ///
    /// Two writes across two files, in this order: **into the project first, then mark the
    /// session's copy carried**. There is no transaction spanning them and none is needed,
    /// because a memory is idempotent by content hash — a crash between the two produces a
    /// reinforcement on the next run rather than a duplicate. The reverse order would lose the
    /// memory outright, which is why the order is the contract rather than an implementation
    /// detail.
    ///
    /// The invariant that matters is unaffected: one live answer per slot is a partial unique
    /// index in the destination, and it does not care which file the write came from.
    pub fn carry(
        project: &mut Store,
        run: &mut Store,
        held: balthasar_model::Memory,
        witness: balthasar_model::Witness,
        at: balthasar_model::Timestamp,
    ) -> Result<crate::Landing, StoreError> {
        let was = held.id.clone();
        let mut moving = held;
        moving.tier = balthasar_model::Tier::Fact;
        let landed = project.remember(moving, witness, at)?;
        // Only now. A session copy marked carried before the project has it is a memory that
        // exists nowhere once the process dies between the two.
        run.archive(&was, at)?;
        Ok(landed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use balthasar_model::scratch::Scratch;
    use balthasar_model::{Body, Memory, NoteKind, ScopeId, Tier, Witness, WitnessKind};

    const NOW: balthasar_model::Timestamp = 1_700_000_000;

    fn scratch(name: &str) -> Scratch {
        Scratch::new("balthasar-pad", name)
    }

    fn note(text: &str, session: &SessionId) -> Memory {
        let mut held = Memory::new(
            crate::mint(NOW),
            Tier::Scratch,
            ScopeId::new("/w/p"),
            Body::note(text, NoteKind::Observation),
            NOW,
        );
        held.session = Some(session.clone());
        held
    }

    fn said_by(session: &SessionId) -> Witness {
        Witness::new(
            balthasar_model::WitnessId::new(crate::mint(NOW).as_str()),
            WitnessKind::Imperative,
            session.clone(),
            ScopeId::new("/w/p"),
            NOW,
        )
    }

    fn kept(pad: &mut Scratchpad, run: &SessionId, text: &str) -> Memory {
        kept_by(pad, run, &AgentId::main(), text)
    }

    fn kept_by(pad: &mut Scratchpad, run: &SessionId, agent: &AgentId, text: &str) -> Memory {
        let held = note(text, run);
        let store = pad.of(run, agent).expect("open");
        store
            .remember(held.clone(), said_by(run), NOW)
            .expect("remember");
        held
    }

    #[test]
    fn a_run_that_says_nothing_leaves_nothing_behind() {
        // Otherwise a week of sessions is a week of empty directories, and `runs()` counts
        // them as runs with scratch to consolidate.
        let home = scratch("empty");
        let mut pad = Scratchpad::at(home.to_path_buf());
        let quiet = SessionId::new("01K5X8");

        assert!(pad.peek(&quiet, &AgentId::main()).expect("peek").is_none());
        assert!(
            !pad.path_of(&quiet, &AgentId::main()).exists(),
            "looking did not create it"
        );
        assert!(pad.runs().is_empty());
    }

    #[test]
    fn a_run_gets_its_own_file_on_its_first_write() {
        let home = scratch("first-write");
        let mut pad = Scratchpad::at(home.to_path_buf());
        let run = SessionId::new("01K5X8");
        kept(&mut pad, &run, "it deploys to fly");

        assert!(pad.path_of(&run, &AgentId::main()).is_file());
        assert_eq!(pad.runs().len(), 1);
    }

    #[test]
    fn two_runs_do_not_share_a_file() {
        // The property the whole restructure is for: deleting one run cannot catch a
        // neighbour, because a neighbour is not in the file.
        let home = scratch("two-runs");
        let mut pad = Scratchpad::at(home.to_path_buf());
        let one = SessionId::new("01K5X8");
        let two = SessionId::new("01K5XB");
        kept(&mut pad, &one, "mine");
        kept(&mut pad, &two, "theirs");

        assert_ne!(
            pad.path_of(&one, &AgentId::main()),
            pad.path_of(&two, &AgentId::main())
        );
        assert_eq!(pad.runs().len(), 2);

        std::fs::remove_dir_all(crate::run_dir_in(&home, &one)).expect("rm");
        assert_eq!(pad.runs().len(), 1, "the neighbour survived");
    }

    #[test]
    fn two_agents_of_one_run_do_not_share_a_file() {
        // The isolation the agent segment is for. One subagent's working notes are not
        // another's to read, and both of them are still one run to forget.
        let home = scratch("two-agents");
        let mut pad = Scratchpad::at(home.to_path_buf());
        let run = SessionId::new("01K5X8");
        let reader = AgentId::new("reader");
        let writer = AgentId::new("writer");
        kept_by(&mut pad, &run, &reader, "mine");
        kept_by(&mut pad, &run, &writer, "theirs");

        assert_ne!(pad.path_of(&run, &reader), pad.path_of(&run, &writer));
        assert_eq!(pad.agents_of(&run), vec![reader.clone(), writer]);
        assert_eq!(
            pad.of(&run, &reader)
                .expect("open")
                .all()
                .expect("all")
                .len(),
            1,
            "an agent reads its own scratch and no other's"
        );
    }

    #[test]
    fn every_agent_of_every_run_is_a_run_to_consolidate() {
        // The walk this had to gain, and the one whose absence is silent: the layout put an
        // agent between a run and its file, so a one-level walk found nothing, returned an
        // empty vector, and decay, sweeping and corroboration all stopped while reporting
        // success. Two levels, and the count is the whole assertion.
        let home = scratch("two-levels");
        let mut pad = Scratchpad::at(home.to_path_buf());
        let agents = [AgentId::main(), AgentId::new("reviewer")];
        for run in ["01ONE", "01TWO", "01THREE"] {
            for agent in &agents {
                kept_by(&mut pad, &SessionId::new(run), agent, "it deploys to fly");
            }
        }

        assert_eq!(pad.runs().len(), 6, "three runs of two agents each");
        assert!(pad.runs().iter().all(|path| path.is_file()));
        assert_eq!(
            pad.weaken_all(NOW + 365 * 24 * 60 * 60).expect("weaken"),
            6,
            "decay reached every one of them"
        );
        assert!(
            !pad.recurring("/w/p", 2, 0, 16)
                .expect("recurring")
                .is_empty(),
            "and the ladder can see across them"
        );
    }

    #[test]
    fn a_run_written_before_agents_existed_is_still_found() {
        // Migration through the door every caller comes in by. An old tree is one file per
        // run; opening a scratchpad over it moves each into `main`, and the scratch a person
        // wrote last week is readable this week rather than invisible.
        let home = scratch("older-tree");
        let old = home.join("01K5X8");
        std::fs::create_dir_all(&old).expect("mkdir");
        {
            let mut store = Store::open(&old.join("memory.db")).expect("open");
            let held = note("the deploy target is fly.io", &SessionId::new("01K5X8"));
            store
                .remember(held, said_by(&SessionId::new("01K5X8")), NOW)
                .expect("remember");
        }

        let mut pad = Scratchpad::at(home.to_path_buf());
        let run = SessionId::new("01K5X8");
        assert_eq!(pad.runs().len(), 1, "the old file was brought forward");
        assert_eq!(
            pad.peek(&run, &AgentId::main())
                .expect("peek")
                .expect("it wrote before")
                .all()
                .expect("all")
                .len(),
            1,
            "and it is the same scratch, not a new empty file"
        );
    }

    #[test]
    fn what_is_carried_across_arrives_as_the_projects() {
        let home = scratch("carry");
        let mut pad = Scratchpad::at(home.to_path_buf());
        let run = SessionId::new("01K5X8");
        let mut project = Store::ephemeral().expect("open");
        let held = kept(&mut pad, &run, "the deploy target is fly.io");

        let store = pad.of(&run, &AgentId::main()).expect("open");
        Scratchpad::carry(&mut project, store, held, said_by(&run), NOW).expect("carry");

        let landed = project.all().expect("all");
        assert_eq!(landed.len(), 1);
        assert_eq!(landed[0].tier, Tier::Fact, "it stopped being one run's own");
    }

    #[test]
    fn a_carried_memory_stops_being_the_runs_to_offer() {
        // Otherwise `balthasar promote` keeps offering the same memory after it has crossed.
        let home = scratch("carried-once");
        let mut pad = Scratchpad::at(home.to_path_buf());
        let run = SessionId::new("01K5X8");
        let mut project = Store::ephemeral().expect("open");
        let held = kept(&mut pad, &run, "the deploy target is fly.io");

        let store = pad.of(&run, &AgentId::main()).expect("open");
        assert_eq!(store.uncrossed(&run).expect("uncrossed").len(), 1);
        Scratchpad::carry(&mut project, store, held, said_by(&run), NOW).expect("carry");
        assert!(
            pad.of(&run, &AgentId::main())
                .expect("open")
                .uncrossed(&run)
                .expect("uncrossed")
                .is_empty(),
            "it crossed"
        );
    }

    #[test]
    fn carrying_the_same_thing_twice_agrees_rather_than_duplicating() {
        // What makes the two writes safe without a transaction spanning them: a crash between
        // them costs a repeat, and a repeat is a reinforcement.
        let home = scratch("carry-twice");
        let mut pad = Scratchpad::at(home.to_path_buf());
        let run = SessionId::new("01K5X8");
        let mut project = Store::ephemeral().expect("open");
        let held = kept(&mut pad, &run, "the deploy target is fly.io");

        let store = pad.of(&run, &AgentId::main()).expect("open");
        Scratchpad::carry(&mut project, store, held.clone(), said_by(&run), NOW).expect("first");
        Scratchpad::carry(&mut project, store, held, said_by(&run), NOW).expect("again");

        assert_eq!(project.all().expect("all").len(), 1, "one memory, not two");
    }

    #[test]
    fn a_reopened_run_is_the_same_file() {
        let home = scratch("reopen");
        let run = SessionId::new("01K5X8");
        {
            let mut pad = Scratchpad::at(home.to_path_buf());
            kept(&mut pad, &run, "held");
        }
        let mut pad = Scratchpad::at(home.to_path_buf());
        let store = pad
            .peek(&run, &AgentId::main())
            .expect("peek")
            .expect("it wrote before");
        assert_eq!(store.all().expect("all").len(), 1);
    }
}
