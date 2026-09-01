//! One run's scratch, in its own file.
//!
//! A session's memories are the session's own until something on the ladder carries them
//! across, so they live in that run's directory rather than as rows in the project's store
//! wearing a `session` column. What that buys is deletion: removing one run is removing one
//! directory, and it takes the scrollback with it.
//!
//! What it costs is that promotion crosses a database boundary, which is [`Scratchpad::carry`].
//! There is no transaction spanning the two files and there does not need to be — see the note
//! there.

use crate::{Store, StoreError};
use memo_model::SessionId;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Every run's scratch under one tool's home, opened as it is needed.
///
/// Held open for as long as the process is: a session writes many times, and reopening the file
/// per turn would be paying SQLite's setup cost for nothing.
pub struct Scratchpad {
    home: PathBuf,
    open: HashMap<SessionId, Store>,
}

impl Scratchpad {
    /// Scratch beneath a tool's home — `<project>/memo/<tool>`.
    #[must_use]
    pub fn at(home: impl Into<PathBuf>) -> Self {
        Self {
            home: home.into(),
            open: HashMap::new(),
        }
    }

    /// Where this scratchpad keeps its runs.
    #[must_use]
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Where `session` keeps its scratch.
    #[must_use]
    pub fn path_of(&self, session: &SessionId) -> PathBuf {
        crate::session_dir_in(&self.home, session).join("memory.db")
    }

    /// The store holding `session`'s scratch, creating it if this is that run's first write.
    ///
    /// Creation is deliberately here and not at session start: a harness that opens a session
    /// and says nothing should leave nothing behind, and an `memo/` tree full of empty
    /// directories is what the alternative looks like after a week.
    pub fn of(&mut self, session: &SessionId) -> Result<&mut Store, StoreError> {
        if !self.open.contains_key(session) {
            let store = Store::open(&self.path_of(session))?;
            self.open.insert(session.clone(), store);
        }
        self.open
            .get_mut(session)
            .ok_or_else(|| StoreError::Foreign("session".to_owned()))
    }

    /// The store holding `session`'s scratch, if that run has ever written.
    ///
    /// For readers. A recall must not bring a run's directory into being merely by looking for
    /// it, which is what [`Scratchpad::of`] would do.
    pub fn peek(&mut self, session: &SessionId) -> Result<Option<&mut Store>, StoreError> {
        if !self.open.contains_key(session) && !self.path_of(session).is_file() {
            return Ok(None);
        }
        self.of(session).map(Some)
    }

    /// Let go of a run's store, so its file can be moved or removed.
    ///
    /// Only [`purge`](crate::purge) has reason to call this: an open connection to a file that
    /// is about to stop existing would hand the next caller a store backed by nothing.
    pub(crate) fn close(&mut self, session: &SessionId) {
        self.open.remove(session);
    }

    /// Every run that has left scratch behind, oldest first.
    ///
    /// Directory names are session names that survived being one, so this is the listing and
    /// not the identities: a name that had to be mangled to be a directory cannot be turned
    /// back. Callers that need the identity open the store and read its session rows.
    #[must_use]
    pub fn runs(&self) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(&self.home) else {
            return Vec::new();
        };
        let mut found: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path().join("memory.db"))
            .filter(|p| p.is_file())
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
        since: memo_model::Timestamp,
        cap: usize,
    ) -> Result<Vec<crate::Cluster>, StoreError> {
        let mut seen: HashMap<String, (String, memo_model::Timestamp, Vec<SessionId>)> =
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
                        r.get::<_, memo_model::Timestamp>(2)?,
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
        held.sort_by(|a, b| b.0.cmp(&a.0));
        held.into_iter().take(cap).map(|(_, path)| path).collect()
    }

    /// Let every run's scratch fade, and archive what has fallen past the floor.
    ///
    /// The same two steps the project's store takes, in the same order and for the same reason:
    /// sweeping before the ladder has looked would take away the scratch it was about to find
    /// corroboration in.
    pub fn weaken_all(&mut self, now: memo_model::Timestamp) -> Result<usize, StoreError> {
        let mut faded = 0;
        for path in self.runs() {
            let mut store = Store::open(&path)?;
            faded += store.weaken(now)?.weakened.len();
        }
        Ok(faded)
    }

    /// Archive what every run's scratch no longer holds up.
    pub fn sweep_all(&mut self, now: memo_model::Timestamp) -> Result<usize, StoreError> {
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
        held: memo_model::Memory,
        witness: memo_model::Witness,
        at: memo_model::Timestamp,
    ) -> Result<crate::Landing, StoreError> {
        let was = held.id.clone();
        let mut moving = held;
        moving.tier = memo_model::Tier::Fact;
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
    use memo_model::{Body, Memory, NoteKind, ScopeId, Tier, Witness, WitnessKind};

    const NOW: memo_model::Timestamp = 1_700_000_000;

    fn scratch(name: &str) -> PathBuf {
        let at = std::env::temp_dir().join(format!("memo-pad-{name}"));
        let _ = std::fs::remove_dir_all(&at);
        std::fs::create_dir_all(&at).expect("mkdir");
        at
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
            memo_model::WitnessId::new(crate::mint(NOW).as_str()),
            WitnessKind::Imperative,
            session.clone(),
            ScopeId::new("/w/p"),
            NOW,
        )
    }

    fn kept(pad: &mut Scratchpad, run: &SessionId, text: &str) -> Memory {
        let held = note(text, run);
        let store = pad.of(run).expect("open");
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
        let mut pad = Scratchpad::at(&home);
        let quiet = SessionId::new("01K5X8");

        assert!(pad.peek(&quiet).expect("peek").is_none());
        assert!(!pad.path_of(&quiet).exists(), "looking did not create it");
        assert!(pad.runs().is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_run_gets_its_own_file_on_its_first_write() {
        let home = scratch("first-write");
        let mut pad = Scratchpad::at(&home);
        let run = SessionId::new("01K5X8");
        kept(&mut pad, &run, "it deploys to fly");

        assert!(pad.path_of(&run).is_file());
        assert_eq!(pad.runs().len(), 1);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn two_runs_do_not_share_a_file() {
        // The property the whole restructure is for: deleting one run cannot catch a
        // neighbour, because a neighbour is not in the file.
        let home = scratch("two-runs");
        let mut pad = Scratchpad::at(&home);
        let one = SessionId::new("01K5X8");
        let two = SessionId::new("01K5XB");
        kept(&mut pad, &one, "mine");
        kept(&mut pad, &two, "theirs");

        assert_ne!(pad.path_of(&one), pad.path_of(&two));
        assert_eq!(pad.runs().len(), 2);

        std::fs::remove_dir_all(crate::session_dir_in(&home, &one)).expect("rm");
        assert_eq!(pad.runs().len(), 1, "the neighbour survived");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn what_is_carried_across_arrives_as_the_projects() {
        let home = scratch("carry");
        let mut pad = Scratchpad::at(&home);
        let run = SessionId::new("01K5X8");
        let mut project = Store::ephemeral().expect("open");
        let held = kept(&mut pad, &run, "the deploy target is fly.io");

        let store = pad.of(&run).expect("open");
        Scratchpad::carry(&mut project, store, held, said_by(&run), NOW).expect("carry");

        let landed = project.all().expect("all");
        assert_eq!(landed.len(), 1);
        assert_eq!(landed[0].tier, Tier::Fact, "it stopped being one run's own");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_carried_memory_stops_being_the_runs_to_offer() {
        // Otherwise `memo promote` keeps offering the same memory after it has crossed.
        let home = scratch("carried-once");
        let mut pad = Scratchpad::at(&home);
        let run = SessionId::new("01K5X8");
        let mut project = Store::ephemeral().expect("open");
        let held = kept(&mut pad, &run, "the deploy target is fly.io");

        let store = pad.of(&run).expect("open");
        assert_eq!(store.uncrossed(&run).expect("uncrossed").len(), 1);
        Scratchpad::carry(&mut project, store, held, said_by(&run), NOW).expect("carry");
        assert!(
            pad.of(&run)
                .expect("open")
                .uncrossed(&run)
                .expect("uncrossed")
                .is_empty(),
            "it crossed"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn carrying_the_same_thing_twice_agrees_rather_than_duplicating() {
        // What makes the two writes safe without a transaction spanning them: a crash between
        // them costs a repeat, and a repeat is a reinforcement.
        let home = scratch("carry-twice");
        let mut pad = Scratchpad::at(&home);
        let run = SessionId::new("01K5X8");
        let mut project = Store::ephemeral().expect("open");
        let held = kept(&mut pad, &run, "the deploy target is fly.io");

        let store = pad.of(&run).expect("open");
        Scratchpad::carry(&mut project, store, held.clone(), said_by(&run), NOW).expect("first");
        Scratchpad::carry(&mut project, store, held, said_by(&run), NOW).expect("again");

        assert_eq!(project.all().expect("all").len(), 1, "one memory, not two");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_reopened_run_is_the_same_file() {
        let home = scratch("reopen");
        let run = SessionId::new("01K5X8");
        {
            let mut pad = Scratchpad::at(&home);
            kept(&mut pad, &run, "held");
        }
        let mut pad = Scratchpad::at(&home);
        let store = pad.peek(&run).expect("peek").expect("it wrote before");
        assert_eq!(store.all().expect("all").len(), 1);
        let _ = std::fs::remove_dir_all(&home);
    }
}
