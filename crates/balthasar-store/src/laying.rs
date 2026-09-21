//! The tables behind the layouts, apart from the code that reads them so the scrollback can create
//! them on open without that code and the scrollback depending on each other.

/// The tables behind the layouts.
pub(crate) const SCHEMA: &str = r"
CREATE TABLE IF NOT EXISTS layout (
  n          INTEGER PRIMARY KEY AUTOINCREMENT,
  session    TEXT NOT NULL,
  at         INTEGER NOT NULL,
  model      TEXT NOT NULL,
  estimated  INTEGER NOT NULL,
  body       TEXT NOT NULL,
  asked      TEXT NOT NULL,
  applied    INTEGER,
  real       INTEGER
) STRICT;
CREATE INDEX IF NOT EXISTS layout_of ON layout(session, n);

CREATE TABLE IF NOT EXISTS factor (
  model    TEXT PRIMARY KEY,
  factor   REAL NOT NULL,
  samples  INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS summary (
  n            INTEGER PRIMARY KEY AUTOINCREMENT,
  session      TEXT NOT NULL,
  from_cursor  INTEGER NOT NULL,
  to_cursor    INTEGER NOT NULL,
  text         TEXT NOT NULL,
  at           INTEGER NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS summary_of ON summary(session, n);

CREATE TABLE IF NOT EXISTS prompt (
  session      TEXT PRIMARY KEY,
  mark         TEXT NOT NULL,
  compactions  INTEGER NOT NULL DEFAULT 0,
  overflows    INTEGER NOT NULL DEFAULT 0,
  memory       TEXT,
  pinned       TEXT,
  helpers      TEXT
) STRICT;
";
