//! A repository, and a series of sessions worked in it.
//!
//! Synthetic on purpose. A benchmark built from real transcripts measures one person's habits
//! and cannot be re-run when a rule changes; this one states what each session did and what it
//! *should* have known, so a regression is a number rather than a feeling.

use memo_model::Timestamp;

/// One thing a session either rediscovered or already knew.
#[derive(Debug, Clone, PartialEq)]
pub struct Lesson {
    /// What a session was trying to do.
    pub intent: String,
    /// What it reaches for first without memory, and which fails.
    pub wrong: String,
    /// What actually works.
    pub right: String,
}

impl Lesson {
    /// A lesson a coding session learns the hard way.
    #[must_use]
    pub fn new(intent: &str, wrong: &str, right: &str) -> Self {
        Self {
            intent: intent.to_owned(),
            wrong: wrong.to_owned(),
            right: right.to_owned(),
        }
    }
}

/// One run of a harness in the repository.
#[derive(Debug, Clone)]
pub struct Session {
    /// Its identity.
    pub id: String,
    /// When it ran.
    pub at: Timestamp,
    /// What the person asked for.
    pub asked: String,
    /// Which lessons it encountered.
    pub lessons: Vec<Lesson>,
}

/// A repository and everything that happened in it.
#[derive(Debug, Clone)]
pub struct Scenario {
    /// What the project is called.
    pub project: String,
    /// The runs, in order.
    pub sessions: Vec<Session>,
}

/// A day, in seconds.
const DAY: Timestamp = 86_400;

impl Scenario {
    /// The canonical case: one repository, one lesson, learned once and needed repeatedly.
    ///
    /// The agent reaches for `cargo test`, it fails, `make test` works. Every session after the
    /// first should start knowing that — and without memory, every one of them rediscovers it.
    #[must_use]
    pub fn one_lesson(sessions: usize, start: Timestamp) -> Self {
        let lesson = Lesson::new("run the tests", "cargo test", "make test");
        Self {
            project: "/w/thing".to_owned(),
            sessions: (0..sessions)
                .map(|n| Session {
                    id: format!("01SESSION{n:04}"),
                    at: start + n as Timestamp * DAY,
                    asked: format!("get the tests passing, attempt {n}"),
                    lessons: vec![lesson.clone()],
                })
                .collect(),
        }
    }

    /// A repository with several things worth knowing, met in different orders.
    ///
    /// Closer to a real project, and it catches something the single-lesson case cannot: a
    /// memory layer that remembers the *last* thing rather than the *relevant* thing scores
    /// well on one lesson and badly here.
    #[must_use]
    pub fn several_lessons(sessions: usize, start: Timestamp) -> Self {
        let lessons = [
            Lesson::new("run the tests", "cargo test", "make test"),
            Lesson::new("build it", "cargo build", "make build"),
            Lesson::new("check the format", "cargo fmt", "make fmt-check"),
        ];
        Self {
            project: "/w/thing".to_owned(),
            sessions: (0..sessions)
                .map(|n| {
                    // Rotated, so no session meets them in the same order as the last.
                    let mut met: Vec<Lesson> = lessons.to_vec();
                    met.rotate_left(n % lessons.len());
                    Session {
                        id: format!("01SESSION{n:04}"),
                        at: start + n as Timestamp * DAY,
                        asked: met[0].intent.clone(),
                        lessons: met,
                    }
                })
                .collect(),
        }
    }

    /// Many distinct lessons, each needed again long after the window has dropped it.
    ///
    /// The shape a window cannot answer and a memory can, and it took three attempts to build
    /// one that actually separates them. A scenario that repeats a single lesson cannot: the
    /// newest copy is always in the window. Nor can one that revisits a small rotating set, for
    /// the same reason. What works is a lesson met twice in a row — enough to cross the ladder —
    /// and then not mentioned again until far more has been said than a window can hold.
    #[must_use]
    pub fn many_lessons(sessions: usize, start: Timestamp) -> Self {
        /// How long ago the revisited lesson was last mentioned, in sessions.
        ///
        /// Far enough that everything about it has left a bounded window, which is the only
        /// condition under which the two arms can disagree.
        const AGO: usize = 25;

        // Distinctive words on purpose. An intent made of stopwords and a bare digit has nothing
        // a full-text query can match — the first version of this scenario scored memory at
        // exactly zero for that reason, which measured the fixture rather than the system.
        const SUBJECT: &[&str] = &[
            "migrations",
            "billing",
            "webhooks",
            "telemetry",
            "invoices",
            "sessions",
            "payouts",
            "scheduler",
            "indexer",
            "compactor",
            "gateway",
            "ledger",
            "renderer",
            "uploader",
            "publisher",
            "sharder",
            "throttler",
            "reconciler",
            "digester",
            "notifier",
        ];
        let make = |n: usize| {
            let subject = SUBJECT[n % SUBJECT.len()];
            let round = n / SUBJECT.len();
            Lesson::new(
                &format!("work on the {subject} service {round}"),
                &format!("cargo {subject}-{round}"),
                &format!("make {subject}-{round}"),
            )
        };
        Self {
            project: "/w/thing".to_owned(),
            sessions: (0..sessions)
                .map(|n| {
                    let fresh = make(n);
                    let mut met = vec![fresh.clone()];
                    // Seen twice in a row, which is what two distinct sessions agreeing means.
                    if n > 0 {
                        met.push(make(n - 1));
                    }
                    // And once more, long after everything about it has scrolled away.
                    if n >= AGO {
                        met.push(make(n - AGO));
                    }
                    Session {
                        id: format!("01SESSION{n:04}"),
                        at: start + n as Timestamp * DAY,
                        asked: fresh.intent.clone(),
                        lessons: met,
                    }
                })
                .collect(),
        }
    }

    /// Every distinct lesson in the scenario.
    #[must_use]
    pub fn lessons(&self) -> Vec<Lesson> {
        let mut out: Vec<Lesson> = Vec::new();
        for session in &self.sessions {
            for lesson in &session.lessons {
                if !out.contains(lesson) {
                    out.push(lesson.clone());
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_lesson_is_met_by_every_session() {
        let scenario = Scenario::one_lesson(5, 0);
        assert_eq!(scenario.sessions.len(), 5);
        assert_eq!(scenario.lessons().len(), 1);
    }

    #[test]
    fn several_lessons_are_met_in_different_orders() {
        // A memory layer that remembers the last thing rather than the relevant thing scores
        // well on one lesson and badly here, which is the point of having both.
        let scenario = Scenario::several_lessons(3, 0);
        let first: Vec<&str> = scenario
            .sessions
            .iter()
            .map(|s| s.lessons[0].intent.as_str())
            .collect();
        let mut distinct = first.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), 3, "every session opened differently");
    }

    #[test]
    fn sessions_are_a_day_apart() {
        // Far enough that recency does not carry a claim on its own, close enough that decay
        // is not what is being measured.
        let scenario = Scenario::one_lesson(2, 0);
        assert_eq!(scenario.sessions[1].at - scenario.sessions[0].at, DAY);
    }
}
