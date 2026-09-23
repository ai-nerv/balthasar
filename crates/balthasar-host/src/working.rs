//! What a task looks like at one moment, and what a proposed replacement must keep.
//!
//! Prepared in the background and swapped in whole, so the checks here are the only thing between
//! a helper's account of a session and what the next request is built from. They are deterministic:
//! a transition either carries the evidence it needs or is refused.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The most of each list that is kept, so one prepared state cannot grow without bound.
pub const MOST: usize = 64;

/// The most characters one entry may hold.
pub const WIDEST: usize = 2_000;

/// Where something is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Requested,
    Planned,
    Attempted,
    Done,
}

/// One piece of work, and the rows that say so.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub what: String,
    pub stage: Stage,
    /// Cursors of the rows this rests on.
    #[serde(default)]
    pub evidence: Vec<u64>,
}

/// Something that was run, and what it said.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Check {
    /// The command as it was run.
    pub ran: String,
    /// What it did. Absent is "not run", which is never the same as passing.
    #[serde(default)]
    pub passed: Option<bool>,
    pub said: String,
    #[serde(default)]
    pub evidence: Vec<u64>,
}

/// Something the person put right, kept word for word.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Correction {
    /// Their words, not a paraphrase of them.
    pub said: String,
    pub cursor: u64,
}

/// A task as the next request should see it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Working {
    #[serde(default)]
    pub goal: String,
    #[serde(default)]
    pub work: Vec<Item>,
    #[serde(default)]
    pub checks: Vec<Check>,
    #[serde(default)]
    pub corrections: Vec<Correction>,
    /// Questions and steps still outstanding.
    #[serde(default)]
    pub open: Vec<String>,
    /// Exact paths, symbols and error text the active task still needs.
    #[serde(default)]
    pub references: Vec<String>,
}

/// Why a proposed working state was not installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// A list is longer than [`MOST`], or an entry wider than [`WIDEST`].
    TooLarge { field: &'static str },
    /// Work reached `Done` with nothing to say so.
    Unevidenced { what: String },
    /// A check that was never run is reported as having passed.
    Untested { ran: String },
    /// A failing check went missing without a later run of it.
    FailureDropped { ran: String },
    /// Work that was not finished is no longer mentioned.
    WorkDropped { what: String },
    /// A correction the person made is gone.
    CorrectionDropped { said: String },
}

impl Working {
    /// Read one from what a helper answered. Anything unparseable is no state at all rather than
    /// a partial one.
    #[must_use]
    pub fn read(said: &Value) -> Option<Self> {
        serde_json::from_value(said.clone()).ok()
    }

    /// Whether `self` may replace `was`.
    ///
    /// # Errors
    /// The first [`Refused`] that applies. A state is bounded, its completions are evidenced, and
    /// nothing unresolved is dropped by being left out.
    pub fn may_replace(&self, was: Option<&Self>) -> Result<(), Refused> {
        self.bounded()?;
        for item in self.work.iter().filter(|i| i.stage == Stage::Done) {
            if item.evidence.is_empty() {
                return Err(Refused::Unevidenced {
                    what: item.what.clone(),
                });
            }
        }
        for check in &self.checks {
            if check.passed == Some(true) && check.evidence.is_empty() {
                return Err(Refused::Untested {
                    ran: check.ran.clone(),
                });
            }
        }
        let Some(was) = was else {
            return Ok(());
        };
        for check in was.checks.iter().filter(|c| c.passed == Some(false)) {
            if !self.checks.iter().any(|now| now.ran == check.ran) {
                return Err(Refused::FailureDropped {
                    ran: check.ran.clone(),
                });
            }
        }
        for item in was.work.iter().filter(|i| i.stage != Stage::Done) {
            if !self.work.iter().any(|now| now.what == item.what) {
                return Err(Refused::WorkDropped {
                    what: item.what.clone(),
                });
            }
        }
        for said in &was.corrections {
            if !self.corrections.iter().any(|now| now.said == said.said) {
                return Err(Refused::CorrectionDropped {
                    said: said.said.clone(),
                });
            }
        }
        Ok(())
    }

    /// Every list within [`MOST`] and every entry within [`WIDEST`].
    fn bounded(&self) -> Result<(), Refused> {
        let long = |n: usize, field| (n > MOST).then_some(Refused::TooLarge { field });
        let wide = |text: &str, field| {
            (text.chars().count() > WIDEST).then_some(Refused::TooLarge { field })
        };
        let first = long(self.work.len(), "work")
            .or_else(|| long(self.checks.len(), "checks"))
            .or_else(|| long(self.corrections.len(), "corrections"))
            .or_else(|| long(self.open.len(), "open"))
            .or_else(|| long(self.references.len(), "references"))
            .or_else(|| wide(&self.goal, "goal"))
            .or_else(|| self.work.iter().find_map(|i| wide(&i.what, "work")))
            .or_else(|| self.checks.iter().find_map(|c| wide(&c.said, "checks")))
            .or_else(|| {
                self.corrections
                    .iter()
                    .find_map(|c| wide(&c.said, "corrections"))
            })
            .or_else(|| self.open.iter().find_map(|o| wide(o, "open")));
        first.map_or(Ok(()), Err)
    }

    /// What the main model is shown. The person's corrections and any failing check go as they
    /// were; the rest is a listing, not a retelling.
    #[must_use]
    pub fn rendered(&self) -> String {
        let mut out = String::new();
        if !self.goal.is_empty() {
            out.push_str(&format!("Goal: {}\n", self.goal));
        }
        let listed = |out: &mut String, title: &str, lines: Vec<String>| {
            if lines.is_empty() {
                return;
            }
            out.push_str(&format!("\n{title}\n"));
            for line in lines {
                out.push_str(&format!("- {line}\n"));
            }
        };
        for (title, stage) in [
            ("Asked for", Stage::Requested),
            ("Planned", Stage::Planned),
            ("Tried", Stage::Attempted),
            ("Done", Stage::Done),
        ] {
            listed(
                &mut out,
                title,
                self.work
                    .iter()
                    .filter(|i| i.stage == stage)
                    .map(|i| i.what.clone())
                    .collect(),
            );
        }
        listed(
            &mut out,
            "Still failing",
            self.checks
                .iter()
                .filter(|c| c.passed == Some(false))
                .map(|c| format!("`{}` — {}", c.ran, c.said))
                .collect(),
        );
        listed(
            &mut out,
            "Not run",
            self.checks
                .iter()
                .filter(|c| c.passed.is_none())
                .map(|c| format!("`{}`", c.ran))
                .collect(),
        );
        listed(
            &mut out,
            "What they said, in their words",
            self.corrections
                .iter()
                .map(|c| format!("\"{}\"", c.said))
                .collect(),
        );
        listed(&mut out, "Open", self.open.clone());
        listed(&mut out, "Exactly", self.references.clone());
        out
    }
}

#[cfg(test)]
#[path = "working/tests.rs"]
mod tests;
