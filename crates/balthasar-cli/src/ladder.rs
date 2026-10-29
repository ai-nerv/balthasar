//! The ladder, climbed by `serve` on its own: a session quiet long enough is distilled, and once a
//! day what recurred is consolidated, the repository's commits read first. On a thread of its
//! own with stores of its own, so no answer waits on it, and under a time budget, so what a pass
//! did not reach is taken up by the next.

use crate::Which;
use balthasar_model::{ScopeId, SessionId, Timestamp};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How long one pass reads before it leaves the rest to the next.
const BUDGET: Duration = Duration::from_secs(30);

/// The call that never starts a pass: a model request is about to go out.
const LAYING: &str = "layout";

/// Where the passes run, and what they have seen.
pub(crate) struct Ladder {
    store_path: Option<PathBuf>,
    scope: ScopeId,
    which: Which,
    cwd: PathBuf,
    budget: Duration,
    /// Sessions observed since they were last distilled, and when each last spoke.
    spoke: BTreeMap<String, Timestamp>,
    consolidated: Option<Timestamp>,
    running: Option<JoinHandle<Left>>,
}

/// What a pass did not reach.
#[derive(Debug, Default)]
struct Left {
    sessions: Vec<SessionId>,
    consolidation: bool,
}

impl Ladder {
    pub(crate) fn new(store_path: Option<PathBuf>, scope: ScopeId, which: Which) -> Self {
        Self {
            store_path,
            scope,
            which,
            cwd: std::env::current_dir().unwrap_or_default(),
            budget: BUDGET,
            spoke: BTreeMap::new(),
            consolidated: None,
            running: None,
        }
    }

    /// Note that `session` was spoken to at `now`.
    pub(crate) fn heard(&mut self, session: &str, now: Timestamp) {
        self.spoke.insert(session.to_owned(), now);
    }

    /// After answering `call`, start whatever is due, unless a pass is still running.
    pub(crate) fn tick(
        &mut self,
        call: &str,
        cadence: Option<balthasar_lua::Ladder>,
        now: Timestamp,
    ) {
        let Some(cadence) = cadence else {
            return;
        };
        if call == LAYING || self.collect() {
            return;
        }
        let (quiet, daily) = self.due(cadence, now);
        if quiet.is_empty() && !daily {
            return;
        }
        let mut pass = self.pass();
        pass.active = self.spoke.keys().cloned().collect();
        self.running = Some(std::thread::spawn(move || pass.run(&quiet, daily, now)));
    }

    /// Wait for a running pass, then distil whatever was still waiting: the session is ending.
    pub(crate) fn finish(&mut self, now: Timestamp) {
        if let Some(running) = self.running.take() {
            let left = running.join().unwrap_or_default();
            self.requeue(left);
        }
        let waiting: Vec<SessionId> = self.spoke.keys().map(SessionId::new).collect();
        if waiting.is_empty() {
            return;
        }
        self.spoke.clear();
        let mut pass = self.pass();
        pass.budget = Duration::MAX;
        pass.run(&waiting, false, now);
    }

    /// Take back what a finished pass left; whether one is still running.
    fn collect(&mut self) -> bool {
        if self.running.as_ref().is_some_and(|run| !run.is_finished()) {
            return true;
        }
        if let Some(done) = self.running.take() {
            let left = done.join().unwrap_or_default();
            self.requeue(left);
        }
        false
    }

    /// Sessions a pass left are due at once; a consolidation it left is due again.
    fn requeue(&mut self, left: Left) {
        for session in left.sessions {
            self.spoke.entry(session.as_str().to_owned()).or_insert(0);
        }
        if left.consolidation {
            self.consolidated = None;
        }
    }

    /// The sessions quiet long enough, and whether a consolidation is due, taken off the list.
    fn due(&mut self, cadence: balthasar_lua::Ladder, now: Timestamp) -> (Vec<SessionId>, bool) {
        let quiet: Vec<SessionId> = self
            .spoke
            .iter()
            .filter(|(_, at)| now.saturating_sub(**at) >= cadence.after_idle_s as Timestamp)
            .map(|(session, _)| SessionId::new(session))
            .collect();
        for session in &quiet {
            self.spoke.remove(session.as_str());
        }
        let daily = cadence.daily
            && self
                .consolidated
                .is_none_or(|at| now.saturating_sub(at) >= balthasar_store::DAY);
        if daily {
            self.consolidated = Some(now);
        }
        (quiet, daily)
    }

    fn pass(&self) -> Pass {
        Pass {
            store_path: self.store_path.clone(),
            scope: self.scope.clone(),
            which: self.which.clone(),
            cwd: self.cwd.clone(),
            budget: self.budget,
            active: Vec::new(),
        }
    }
}

/// One pass, with stores and a configuration of its own.
struct Pass {
    store_path: Option<PathBuf>,
    scope: ScopeId,
    which: Which,
    cwd: PathBuf,
    budget: Duration,
    /// Sessions still speaking, which a daily pass leaves alone.
    active: Vec<String>,
}

impl Pass {
    fn run(&self, sessions: &[SessionId], daily: bool, now: Timestamp) -> Left {
        match self.climb(sessions, daily, now) {
            Ok(left) => left,
            Err(why) => {
                balthasar_model::noted!("ladder: a pass failed: {why}");
                Left::default()
            }
        }
    }

    fn climb(&self, sessions: &[SessionId], daily: bool, now: Timestamp) -> anyhow::Result<Left> {
        let started = Instant::now();
        let mut loaded = crate::loaded::Loaded::read(&self.cwd)?;
        let path = self.store_path.as_deref();
        let mut store = crate::open(path, &self.scope, &self.which)?;
        let held = crate::scrollback(path, &self.scope, &self.which)?;
        let marked = SessionId::new(balthasar_store::LADDER);
        let mut queue = sessions.to_vec();
        if daily {
            let unread =
                balthasar_distil::undistilled(&store, &held, balthasar_distil::RUNS_PER_PASS)?;
            for run in unread {
                if !self.active.iter().any(|a| a == run.as_str()) && !queue.contains(&run) {
                    queue.push(run);
                }
            }
        }
        let left = within(&queue, self.budget, started, |session| {
            crate::distil::pass(
                &mut store,
                &held,
                &self.scope,
                std::slice::from_ref(session),
                now,
                false,
                &mut loaded,
            )
            .map(drop)
        })?;
        if left.len() < queue.len() {
            held.set_counter(&marked, "distilled", u64::try_from(now).unwrap_or(0))?;
        }
        let deferred = daily && (!left.is_empty() || started.elapsed() >= self.budget);
        if daily && !deferred {
            let settings = loaded.settings().clone();
            balthasar_distil::read_commits(&mut store, &self.scope, &settings, now)?;
            let mut pad =
                balthasar_store::Scratchpad::at(crate::runs_under(path, &self.scope, &self.which));
            loaded.consolidate(&mut store, Some(&mut pad), &self.scope, now, false)?;
            held.set_counter(&marked, "consolidated", u64::try_from(now).unwrap_or(0))?;
        }
        Ok(Left {
            sessions: left,
            consolidation: deferred,
        })
    }
}

/// Read `sessions` in order until `budget` has passed since `started`, always at least one;
/// those not reached.
fn within(
    sessions: &[SessionId],
    budget: Duration,
    started: Instant,
    mut read: impl FnMut(&SessionId) -> anyhow::Result<()>,
) -> anyhow::Result<Vec<SessionId>> {
    for (n, session) in sessions.iter().enumerate() {
        if n > 0 && started.elapsed() >= budget {
            return Ok(sessions.get(n..).unwrap_or_default().to_vec());
        }
        read(session)?;
    }
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = 1_800_000_000;

    fn ladder() -> Ladder {
        Ladder::new(
            None,
            ScopeId::new("/w"),
            Which {
                tool: balthasar_store::Tool::default(),
                named: false,
            },
        )
    }

    fn every(after_idle_s: u64) -> balthasar_lua::Ladder {
        balthasar_lua::Ladder {
            after_idle_s,
            daily: false,
        }
    }

    #[test]
    fn consolidation_never_runs_during_layout() {
        let mut ladder = ladder();
        ladder.heard("s", NOW);
        let daily = balthasar_lua::Ladder {
            after_idle_s: 0,
            daily: true,
        };
        ladder.tick(LAYING, Some(daily), NOW);
        assert!(ladder.running.is_none(), "nothing started on a layout");
        assert!(ladder.spoke.contains_key("s"), "and nothing was taken off");
        assert_eq!(ladder.consolidated, None);
    }

    #[test]
    fn a_session_still_speaking_waits() {
        let mut ladder = ladder();
        ladder.heard("s", NOW);
        assert_eq!(ladder.due(every(600), NOW + 10), (Vec::new(), false));
        assert_eq!(
            ladder.due(every(600), NOW + 600).0,
            vec![SessionId::new("s")]
        );
        assert!(ladder.spoke.is_empty());
    }

    #[test]
    fn a_pass_over_budget_resumes_next_time() {
        let sessions = [SessionId::new("a"), SessionId::new("b")];
        let mut read = Vec::new();
        let left = within(&sessions, Duration::ZERO, Instant::now(), |s| {
            read.push(s.clone());
            Ok(())
        })
        .expect("pass");
        assert_eq!(
            read,
            vec![SessionId::new("a")],
            "at least one, then the budget"
        );
        assert_eq!(left, vec![SessionId::new("b")]);

        let mut ladder = ladder();
        ladder.consolidated = Some(NOW);
        ladder.running = Some(std::thread::spawn(move || Left {
            sessions: left,
            consolidation: true,
        }));
        while ladder.collect() {
            std::thread::yield_now();
        }
        let (quiet, daily) = ladder.due(
            balthasar_lua::Ladder {
                after_idle_s: 600,
                daily: true,
            },
            NOW + 1,
        );
        assert_eq!(quiet, vec![SessionId::new("b")], "due at once, idle or not");
        assert!(daily, "and the consolidation it left is due again");
    }
}
