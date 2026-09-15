//! What a model is actually sent: a window into the scrollback, and what memory has to add.

use crate::{Ask, Context, Section, assemble};
use balthasar_model::SessionId;
use balthasar_store::{Budget, Store, Transcript, Turn, Want};

/// How a budget is divided between what was just said and what is known.
#[derive(Debug, Clone, PartialEq)]
pub struct Split {
    /// The whole allowance.
    pub tokens: usize,
    /// The least memory may have, however long the run gets.
    pub memory_floor: f64,
    /// The most the scrollback may take before memory's floor bites.
    pub scrollback_ceiling: f64,
    /// The share of memory's own allowance that verbatim spans may take.
    pub span_share: f64,
    /// What the reply and a compaction call need kept clear.
    pub reserve: usize,
}

impl Default for Split {
    fn default() -> Self {
        Self {
            tokens: 8_000,
            memory_floor: 0.25,
            scrollback_ceiling: 0.75,
            span_share: 0.3,
            reserve: 2_000,
        }
    }
}

impl Split {
    /// What the scrollback may spend, given how much of it there is.
    #[must_use]
    pub fn for_scrollback(&self, available: usize) -> usize {
        let ceiling = (self.tokens as f64 * self.scrollback_ceiling) as usize;
        available.min(ceiling)
    }

    /// What memory may spend, given what the scrollback actually took.
    #[must_use]
    pub fn for_memory(&self, scrollback_took: usize) -> usize {
        let floor = (self.tokens as f64 * self.memory_floor) as usize;
        self.tokens.saturating_sub(scrollback_took).max(floor)
    }
}

/// How full the window is, and how long that can last.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pressure {
    /// What the prompt costs.
    pub used: usize,
    /// What the model will take, less what the reply and a compaction need.
    pub usable: usize,
}

impl Pressure {
    /// The share of the usable window this prompt takes.
    #[must_use]
    pub fn share(&self) -> f64 {
        if self.usable == 0 {
            return 1.0;
        }
        self.used as f64 / self.usable as f64
    }

    /// What is left.
    #[must_use]
    pub fn headroom(&self) -> usize {
        self.usable.saturating_sub(self.used)
    }

    /// Whether this prompt still fits.
    #[must_use]
    pub fn fits(&self) -> bool {
        self.used <= self.usable
    }

    /// Whether it is time to compact.
    #[must_use]
    pub fn should_compact(&self) -> bool {
        self.share() >= 0.8
    }
}

/// A prompt: recent turns, and what memory has to add about them.
#[derive(Debug, Clone, PartialEq)]
pub struct Prompt {
    /// The window into the scrollback, oldest first.
    pub turns: Vec<Turn>,
    /// What those turns cost.
    pub scrollback_tokens: usize,
    /// How many earlier turns did not fit.
    pub omitted: usize,
    /// What memory added.
    pub memory: Context,
    /// What was said that no memory records.
    pub spans: crate::Quotes,
    /// The whole thing.
    pub tokens: usize,
    /// How full this leaves the window.
    pub pressure: Pressure,
}

impl Default for Prompt {
    fn default() -> Self {
        Self {
            turns: Vec::new(),
            scrollback_tokens: 0,
            omitted: 0,
            memory: Context::default(),
            spans: crate::Quotes::default(),
            tokens: 0,
            pressure: Pressure { used: 0, usable: 0 },
        }
    }
}

impl Prompt {
    /// Whether the window is showing the start of the run.
    #[must_use]
    pub fn is_whole(&self) -> bool {
        self.omitted == 0
    }

    /// The share of the prompt spent on what was just said.
    #[must_use]
    pub fn scrollback_share(&self) -> f64 {
        if self.tokens == 0 {
            return 0.0;
        }
        self.scrollback_tokens as f64 / self.tokens as f64
    }
}

/// Build one.
///
/// The scrollback is read first, because how much of it there is decides what memory has left.
pub fn compose(
    stores: &[(Store, bool)],
    scrollback: &Transcript,
    session: &SessionId,
    sections: &[Section],
    ask: &Ask,
    split: &Split,
    redact: impl FnMut(&str, &balthasar_model::Memory) -> Option<String>,
) -> Result<Prompt, balthasar_store::StoreError> {
    // The run's own model when it said which one, and the caller's split otherwise.
    let split = &match scrollback.model_of(session) {
        Ok(Some((_, context))) => Split {
            tokens: context as usize,
            ..split.clone()
        },
        _ => split.clone(),
    };

    let read = scrollback.read(
        session,
        &Want::Tail,
        &Budget {
            tokens: split.for_scrollback(split.tokens),
            ..Budget::default()
        },
    )?;

    let for_memory = split.for_memory(read.tokens);
    let memory = assemble(
        stores,
        sections,
        &Ask {
            tokens: for_memory,
            ..ask.clone()
        },
        redact,
    )?;

    // Spans get what memory did not spend, capped at their share.
    let spare = for_memory.saturating_sub(memory.tokens);
    let allowance = spare.min((for_memory as f64 * split.span_share) as usize);
    let already: Vec<String> = memory
        .sections
        .iter()
        .flat_map(|s| s.lines.iter().cloned())
        .collect();
    let spans = crate::quote(scrollback, &ask.turn, &already, allowance, |_| false)?;

    let used = read.tokens + memory.tokens + spans.tokens;
    Ok(Prompt {
        tokens: used,
        scrollback_tokens: read.tokens,
        omitted: read.omitted,
        turns: read.turns,
        memory,
        spans,
        pressure: Pressure {
            used,
            usable: split.tokens.saturating_sub(split.reserve),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_run_leaves_the_whole_budget_to_memory() {
        let split = Split::default();
        assert_eq!(split.for_memory(0), split.tokens);
    }

    #[test]
    fn a_growing_run_takes_the_room_it_needs() {
        let split = Split::default();
        let early = split.for_memory(200);
        let later = split.for_memory(4_000);
        assert!(later < early, "{later} should be less than {early}");
    }

    #[test]
    fn memory_never_falls_below_its_floor() {
        let split = Split::default();
        let floor = (split.tokens as f64 * split.memory_floor) as usize;
        assert_eq!(split.for_memory(split.tokens * 10), floor);
        assert!(floor > 0);
    }

    #[test]
    fn the_scrollback_stops_at_its_ceiling() {
        let split = Split::default();
        let ceiling = (split.tokens as f64 * split.scrollback_ceiling) as usize;
        assert_eq!(split.for_scrollback(usize::MAX), ceiling);
    }

    #[test]
    fn a_short_scrollback_asks_for_no_more_than_it_has() {
        let split = Split::default();
        assert_eq!(split.for_scrollback(120), 120);
    }

    #[test]
    fn a_prompt_says_whether_it_is_showing_the_whole_run() {
        let whole = Prompt::default();
        assert!(whole.is_whole());

        let cut = Prompt {
            omitted: 40,
            ..Prompt::default()
        };
        assert!(!cut.is_whole());
    }

    #[test]
    fn the_share_of_a_prompt_with_nothing_in_it_is_zero() {
        assert!((Prompt::default().scrollback_share() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn the_split_is_a_fraction_so_it_scales_with_the_budget() {
        let small = Split {
            tokens: 1_000,
            ..Split::default()
        };
        let large = Split {
            tokens: 100_000,
            ..Split::default()
        };
        let ratio = |s: &Split| s.for_memory(s.tokens * 10) as f64 / s.tokens as f64;
        assert!((ratio(&small) - ratio(&large)).abs() < 0.01);
    }
}
