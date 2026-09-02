//! Sessions, which are what a project has many of.
//!
//! The two scopes are different in kind and the difference is the whole shape of the thing:
//!
//! * a **project** is where durable memory lives, and every session in it shares that memory;
//! * a **session** is one run, and what it holds is its own until something on the ladder
//!   carries it across.
//!
//! Which means every memory has to be able to say where it came from. "You learned this in
//! some session" is not an answer anybody can act on, so a session gets a name a person can
//! say and a title saying what it was for.

use crate::{Store, StoreError};
use memo_model::{ScopeId, SessionId, Timestamp};
use rusqlite::{OptionalExtension, params};

/// One run of a harness, in one project.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    /// The harness's own identity for it.
    pub id: SessionId,
    /// Short, typeable, and unique within the project.
    pub name: String,
    /// Which project.
    pub scope: ScopeId,
    /// Where it ran.
    pub cwd: String,
    /// Which harness, so a store read from several can say.
    pub harness: String,
    /// When it started.
    pub opened: Timestamp,
    /// When it ended, if it has.
    pub closed: Option<Timestamp>,
    /// What it was for — the first thing asked, which is the closest thing to a name that
    /// exists without asking a model to invent one.
    pub title: Option<String>,
}

impl Session {
    /// Whether it is still running.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.closed.is_none()
    }

    /// What to show a person: the title if there is one, else the name.
    #[must_use]
    pub fn label(&self) -> &str {
        self.title
            .as_deref()
            .filter(|t| !t.is_empty())
            .unwrap_or(&self.name)
    }
}

/// A short, typeable name for a session started at `opened`.
///
/// Month and day, then four characters of the session's own identity. Recognisable at a glance
/// — a person knows what they were doing on the 31st — and short enough to type at a prompt,
/// which a twenty-six character id is not.
#[must_use]
pub fn name_for(id: &SessionId, opened: Timestamp) -> String {
    let days = opened.max(0) / 86_400;
    // Civil date from a day count, without a calendar dependency. Correct from 1970 onwards,
    // which is every timestamp a transcript can carry.
    let (month, day) = civil(days);
    let text = id.as_str();
    let tail: String = text
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{month:02}{day:02}-{}", tail.to_lowercase())
}

/// Month and day from days since the epoch.
fn civil(days: i64) -> (i64, i64) {
    // Howard Hinnant's civil_from_days, shifted so March is month 0 and the leap day lands at
    // the end of a year. Shorter and less wrong than counting months in a loop.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (month, day)
}

impl Store {
    /// Record that a session has started, or take the one already recorded.
    ///
    /// Idempotent by id: a harness that reconnects, or an ingest run twice, must not produce
    /// two sessions where there was one.
    pub fn open_session(
        &mut self,
        id: &SessionId,
        scope: &ScopeId,
        cwd: &str,
        harness: &str,
        opened: Timestamp,
    ) -> Result<Session, StoreError> {
        if let Some(held) = self.session_by_id(id)? {
            return Ok(held);
        }
        let name = name_for(id, opened);
        self.db().execute(
            "INSERT INTO session (id, scope, cwd, harness, opened, name) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id.as_str(), scope.as_str(), cwd, harness, opened, name],
        )?;
        Ok(Session {
            id: id.clone(),
            name,
            scope: scope.clone(),
            cwd: cwd.to_owned(),
            harness: harness.to_owned(),
            opened,
            closed: None,
            title: None,
        })
    }

    /// Say what a session was for.
    ///
    /// Written once and not overwritten: the first thing asked is the title, and a session that
    /// wandered onto three other topics is still named after where it started.
    pub fn title_session(&mut self, id: &SessionId, title: &str) -> Result<(), StoreError> {
        let trimmed: String = title.trim().chars().take(72).collect();
        self.db().execute(
            "UPDATE session SET title = ?2 WHERE id = ?1 AND (title IS NULL OR title = '')",
            params![id.as_str(), trimmed],
        )?;
        Ok(())
    }

    /// Record that a session has ended.
    pub fn close_session(&mut self, id: &SessionId, at: Timestamp) -> Result<(), StoreError> {
        self.db().execute(
            "UPDATE session SET closed = ?2 WHERE id = ?1 AND closed IS NULL",
            params![id.as_str(), at],
        )?;
        Ok(())
    }

    /// Every session in this store, newest first.
    pub fn sessions(&self, limit: usize) -> Result<Vec<Session>, StoreError> {
        let mut statement = self.db().prepare(
            "SELECT id, name, scope, cwd, harness, opened, closed, title \
             FROM session ORDER BY opened DESC LIMIT ?1",
        )?;
        let found = statement
            .query_map(params![limit as i64], |r| Ok(read(r)))?
            .collect::<Result<Vec<_>, _>>()?;
        found.into_iter().collect()
    }

    /// One session by its id.
    pub fn session_by_id(&self, id: &SessionId) -> Result<Option<Session>, StoreError> {
        let found = self
            .db()
            .query_row(
                "SELECT id, name, scope, cwd, harness, opened, closed, title \
                 FROM session WHERE id = ?1",
                params![id.as_str()],
                |r| Ok(read(r)),
            )
            .optional()?;
        found.transpose()
    }

    /// One session by name, by id, or by enough of either to be unambiguous.
    ///
    /// A name first, because that is what is printed and therefore what somebody will type.
    pub fn session(&self, handle: &str) -> Result<Option<Session>, StoreError> {
        let wanted = handle.to_lowercase();
        let all = self.sessions(usize::MAX)?;
        let mut matches: Vec<Session> = all
            .iter()
            .filter(|s| s.name.eq_ignore_ascii_case(handle))
            .cloned()
            .collect();
        if matches.is_empty() {
            matches = all
                .into_iter()
                .filter(|s| {
                    s.id.as_str().eq_ignore_ascii_case(handle)
                        || s.name.starts_with(&wanted)
                        || s.id.as_str().to_lowercase().ends_with(&wanted)
                })
                .collect();
        }
        // An ambiguous handle answers nothing rather than guessing. Acting on the wrong
        // session is worse than being asked again.
        Ok((matches.len() == 1).then(|| matches.remove(0)))
    }

    /// How many memories each session contributed, newest session first.
    ///
    /// The question a person actually has about a session list: not "which runs happened" but
    /// "which of them left anything behind".
    pub fn session_yield(&self, id: &SessionId) -> Result<usize, StoreError> {
        let count: i64 = self.db().query_row(
            "SELECT count(DISTINCT memory) FROM witness WHERE session = ?1",
            params![id.as_str()],
            |r| r.get(0),
        )?;
        Ok(count.max(0) as usize)
    }
}

/// One session from a row.
fn read(row: &rusqlite::Row<'_>) -> Result<Session, StoreError> {
    Ok(Session {
        id: SessionId::new(row.get::<_, String>(0)?),
        name: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
        scope: ScopeId::new(row.get::<_, String>(2)?),
        cwd: row.get(3)?,
        harness: row.get(4)?,
        opened: row.get(5)?,
        closed: row.get(6)?,
        title: row.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUGUST_31: Timestamp = 1_756_598_400;

    #[test]
    fn a_name_says_when_and_which() {
        let name = name_for(&SessionId::new("01M1CTNN7SZ613D58ZXM4JYT8Z"), AUGUST_31);
        assert!(name.starts_with("0831-"), "{name}");
        assert_eq!(name.len(), 9, "{name} must be typeable");
    }

    #[test]
    fn two_sessions_on_one_day_get_different_names() {
        let a = name_for(&SessionId::new("01M1CTNN7SZ613D58ZXM4JYT8Z"), AUGUST_31);
        let b = name_for(&SessionId::new("01M1CTNN7SZ613D58ZXM4JQQQQ"), AUGUST_31);
        assert_ne!(a, b);
    }

    #[test]
    fn the_calendar_is_right_about_a_leap_day() {
        // A day count converted with a loop over month lengths gets this wrong, and the
        // failure is one wrong character in a name nobody would think to check.
        let leap = 1_709_164_800; // 2024-02-29
        assert_eq!(civil(leap / 86_400), (2, 29));
    }

    #[test]
    fn a_session_is_opened_once_however_often_it_reconnects() {
        let mut store = Store::ephemeral().expect("store");
        let id = SessionId::new("01H");
        let first = store
            .open_session(&id, &ScopeId::global(), "/w", "cli", AUGUST_31)
            .expect("open");
        let again = store
            .open_session(&id, &ScopeId::global(), "/w", "cli", AUGUST_31 + 500)
            .expect("open again");
        assert_eq!(first, again);
        assert_eq!(store.sessions(10).expect("list").len(), 1);
    }

    #[test]
    fn a_title_is_the_first_thing_asked_and_stays_that() {
        // A session that wandered onto three other topics is still named after where it
        // started, which is what makes a list of them readable.
        let mut store = Store::ephemeral().expect("store");
        let id = SessionId::new("01H");
        store
            .open_session(&id, &ScopeId::global(), "/w", "cli", AUGUST_31)
            .expect("open");
        store.title_session(&id, "run the tests").expect("title");
        store
            .title_session(&id, "something else entirely")
            .expect("again");
        let held = store.session_by_id(&id).expect("get").expect("there");
        assert_eq!(held.title.as_deref(), Some("run the tests"));
    }

    #[test]
    fn a_session_is_found_by_the_name_that_was_printed() {
        let mut store = Store::ephemeral().expect("store");
        let id = SessionId::new("01M1CTNN7SZ613D58ZXM4JYT8Z");
        let opened = store
            .open_session(&id, &ScopeId::global(), "/w", "cli", AUGUST_31)
            .expect("open");
        assert_eq!(
            store.session(&opened.name).expect("find").map(|s| s.id),
            Some(id)
        );
    }

    #[test]
    fn an_ambiguous_handle_answers_nothing_rather_than_guessing() {
        let mut store = Store::ephemeral().expect("store");
        for tail in ["AAAA", "AAAB"] {
            store
                .open_session(
                    &SessionId::new(format!("01M1CTNN7SZ613D58ZXM4J{tail}")),
                    &ScopeId::global(),
                    "/w",
                    "cli",
                    AUGUST_31,
                )
                .expect("open");
        }
        assert!(store.session("0831").expect("look").is_none());
    }

    #[test]
    fn a_session_says_how_much_it_left_behind() {
        let mut store = Store::ephemeral().expect("store");
        let id = SessionId::new("01H");
        store
            .open_session(&id, &ScopeId::global(), "/w", "cli", AUGUST_31)
            .expect("open");
        assert_eq!(store.session_yield(&id).expect("count"), 0);
    }
}
