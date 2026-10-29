//! Whether a store is well, as checks a person can act on.

use crate::{Store, StoreError, Transcript};

/// How one check came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// As it should be.
    Ok,
    /// Degraded, still working: worth a look.
    Cold,
    /// Wrong in a way that loses or misstates memory.
    Red,
}

impl State {
    /// The word a status line uses.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Cold => "cold",
            Self::Red => "red",
        }
    }
}

/// One check.
#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub name: &'static str,
    pub state: State,
    pub detail: String,
    /// The command that fixes it, when one does.
    pub fix: Option<String>,
}

impl Check {
    fn new(name: &'static str, state: State, detail: String, fix: Option<&str>) -> Self {
        Self {
            name,
            state,
            detail,
            fix: fix.map(str::to_owned),
        }
    }
}

/// Every command a check names as its fix.
pub const FIXES: &[&str] = &[SAVE, REINDEX, RETRY, CONSOLIDATE];

/// The run whose counters hold when the ladder last climbed: `distilled` and `consolidated`.
pub const LADDER: &str = "ladder";

const SAVE: &str = "balthasar export";
const REINDEX: &str = "balthasar reindex --text";
const RETRY: &str = "balthasar api jobs '\"<session>\"' '{\"retry_extraction\":true}'";
const CONSOLIDATE: &str = "balthasar consolidate --now";

/// Every check on `store`, and on `scrollback` when there is one.
pub fn health(store: &Store, scrollback: Option<&Transcript>) -> Result<Vec<Check>, StoreError> {
    let count =
        |sql: &str| -> Result<i64, StoreError> { Ok(store.db().query_row(sql, [], |r| r.get(0))?) };
    let mut out = Vec::new();

    let version: i64 = store
        .db()
        .pragma_query_value(None, "user_version", |r| r.get(0))?;
    out.push(Check::new(
        "schema",
        State::Ok,
        format!("version {version}"),
        None,
    ));

    let verdict: String = store
        .db()
        .query_row("PRAGMA quick_check", [], |r| r.get(0))?;
    out.push(if verdict == "ok" {
        Check::new("integrity", State::Ok, "quick_check ok".into(), None)
    } else {
        let fix = if verdict.contains("FTS") {
            REINDEX
        } else {
            SAVE
        };
        Check::new("integrity", State::Red, verdict, Some(fix))
    });

    let (held, indexed) = (
        count("SELECT count(*) FROM memory")?,
        count("SELECT count(*) FROM memory_fts")?,
    );
    out.push(indexed_check("text index", held, indexed));

    if let Some(scrollback) = scrollback {
        out.extend(transcript_checks(scrollback)?);
    }
    Ok(out)
}

/// The scrollback's own checks: its search index, extraction, jobs and layout effects.
fn transcript_checks(scrollback: &Transcript) -> Result<Vec<Check>, StoreError> {
    let count = |sql: &str| -> Result<i64, StoreError> {
        Ok(scrollback.db().query_row(sql, [], |r| r.get(0))?)
    };
    let mut out = vec![indexed_check(
        "spans index",
        count("SELECT count(*) FROM turn WHERE trim(text) != ''")?,
        count("SELECT count(*) FROM turn_fts")?,
    )];

    let blocked = count("SELECT count(*) FROM extraction_progress WHERE blocked IS NOT NULL")?;
    out.push(if blocked == 0 {
        Check::new("extraction", State::Ok, "nothing blocked".into(), None)
    } else {
        Check::new(
            "extraction",
            State::Red,
            format!("{blocked} session(s) blocked on a row too large to read"),
            Some(RETRY),
        )
    });

    let failed = count("SELECT count(*) FROM job WHERE state = 'failed'")?;
    out.push(if failed == 0 {
        Check::new("jobs", State::Ok, "none failed".into(), None)
    } else {
        Check::new(
            "jobs",
            State::Cold,
            format!("{failed} helper job(s) failed"),
            Some(RETRY),
        )
    });

    let aside = scrollback.failed_layout_effects()?;
    out.push(if aside == 0 {
        Check::new("effects", State::Ok, "none set aside".into(), None)
    } else {
        Check::new(
            "effects",
            State::Cold,
            format!("{aside} layout effect(s) set aside"),
            None,
        )
    });

    let consolidated = scrollback
        .counter(&balthasar_model::SessionId::new(LADDER), "consolidated")?
        .map_or(0, |at| i64::try_from(at).unwrap_or(i64::MAX));
    let waiting: i64 = scrollback.db().query_row(
        "SELECT count(*) FROM turn WHERE at > ?1 AND at < (SELECT max(at) FROM turn) - ?2",
        rusqlite::params![consolidated, 2 * crate::DAY],
        |r| r.get(0),
    )?;
    out.push(if waiting == 0 {
        Check::new("ladder", State::Ok, "consolidated lately".into(), None)
    } else {
        Check::new(
            "ladder",
            State::Cold,
            format!("{waiting} turn(s) waited over two days for a consolidation"),
            Some(CONSOLIDATE),
        )
    });
    Ok(out)
}

/// Whether an index holds a row for every row it indexes.
fn indexed_check(name: &'static str, held: i64, indexed: i64) -> Check {
    if held == indexed {
        Check::new(name, State::Ok, format!("{indexed} rows"), None)
    } else {
        Check::new(
            name,
            State::Red,
            format!("{indexed} indexed of {held}"),
            Some(REINDEX),
        )
    }
}

/// One line: `balthasar N/N ok`, `… · cold: a, b`, or the first red check and its fix.
#[must_use]
pub fn summary(checks: &[Check]) -> String {
    if let Some(red) = checks.iter().find(|c| c.state == State::Red) {
        let fix = red
            .fix
            .as_deref()
            .map_or_else(String::new, |fix| format!(" — run {fix}"));
        return format!("balthasar RED {}: {}{fix}", red.name, red.detail);
    }
    let fine = checks.iter().filter(|c| c.state == State::Ok).count();
    let cold: Vec<&str> = checks
        .iter()
        .filter(|c| c.state == State::Cold)
        .map(|c| c.name)
        .collect();
    let mut line = format!("balthasar {fine}/{} ok", checks.len());
    if !cold.is_empty() {
        line.push_str(&format!(" · cold: {}", cold.join(", ")));
    }
    line
}

impl Store {
    /// Rebuild the full-text index from `memory`, in one transaction.
    pub fn reindex_text(&mut self) -> Result<usize, StoreError> {
        let tx = self.db_mut().transaction()?;
        tx.execute("DELETE FROM memory_fts", [])?;
        let n = tx.execute(
            "INSERT INTO memory_fts (id, text, subject, predicate, object) \
             SELECT id, text, subject, predicate, object FROM memory",
            [],
        )?;
        tx.commit()?;
        Ok(n)
    }
}
