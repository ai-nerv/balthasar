//! What a model is actually sent: a window into the scrollback, and what memory has to add.
//!
//! The view is not a buffer the harness keeps beside the store — it is a window *into* the
//! store. The transcript is appended to as the run happens and read back as a slice, so what a
//! model sees and what memo holds cannot drift: there is only one copy, and the prompt is a
//! rendering of part of it.
//!
//! **The split moves as the session does.** At the start there is almost no scrollback, so
//! memory fills the budget — which is right, because that is the moment the agent knows least
//! and last week's lessons are all it has. As the run grows, the recent turns matter more and
//! take the room. Memory never disappears, though: it keeps a floor, because a session long
//! enough to crowd it out is exactly the one that will otherwise rediscover something.
//!
//! Nothing here decides what is *true*. It decides what fits.

use crate::{Ask, Context, Section, assemble};
use memo_model::SessionId;
use memo_store::{Budget, Store, Transcript, Turn, Want};

/// How a budget is divided between what was just said and what is known.
#[derive(Debug, Clone, PartialEq)]
pub struct Split {
    /// The whole allowance.
    pub tokens: usize,
    /// The least memory may have, however long the run gets.
    ///
    /// A fraction rather than a count, so it scales with whatever budget a caller has. A
    /// quarter: enough for a handful of facts and a habit, which is the difference between an
    /// agent that repeats last week's mistake and one that does not.
    pub memory_floor: f64,
    /// The most the scrollback may take before memory's floor bites.
    pub scrollback_ceiling: f64,
    /// What the reply and a compaction call need kept clear.
    pub reserve: usize,
}

impl Default for Split {
    fn default() -> Self {
        Self {
            tokens: 8_000,
            memory_floor: 0.25,
            scrollback_ceiling: 0.75,
            reserve: 2_000,
        }
    }
}

impl Split {
    /// What the scrollback may spend, given how much of it there is.
    ///
    /// Grows with the run and stops at the ceiling. A short session leaves most of the budget to
    /// memory without anybody configuring that — the scrollback simply has not got enough to
    /// claim it yet.
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
///
/// The number a caller needs before a request, not after it: whether this fits, and whether the
/// next one will. A harness that finds out by being refused has already lost the turn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pressure {
    /// What the prompt costs.
    pub used: usize,
    /// What the model will take, less what the reply and a compaction need.
    ///
    /// Not the raw context size. Compacting costs a request of its own and that request needs
    /// room, so a plan that filled the window to the brim would leave the summarisation itself
    /// as the thing that overflows.
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
    ///
    /// Before the window is full, not at it. Compaction needs a request of its own, and a
    /// harness that waits until nothing fits has nowhere to run it.
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
            tokens: 0,
            pressure: Pressure { used: 0, usable: 0 },
        }
    }
}

impl Prompt {
    /// Whether the window is showing the start of the run.
    ///
    /// When it is, nothing has been cut and the model is seeing the whole conversation. When it
    /// is not, something earlier exists and only memory can speak for it — which is the moment
    /// memory stops being a nicety.
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
/// The other order would fix memory's allowance before knowing whether the run needed the room,
/// and a long session would then carry a full page of facts it had already acted on.
pub fn compose(
    stores: &[(Store, bool)],
    scrollback: &Transcript,
    session: &SessionId,
    sections: &[Section],
    ask: &Ask,
    split: &Split,
    redact: impl FnMut(&str, &memo_model::Memory) -> Option<String>,
) -> Result<Prompt, memo_store::StoreError> {
    // The run's own model when it said which one, and the caller's split otherwise. A budget
    // set from a default is a guess about the one number that decides whether a prompt is
    // accepted, and the run already knows the answer.
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

    let memory = assemble(
        stores,
        sections,
        &Ask {
            tokens: split.for_memory(read.tokens),
            ..ask.clone()
        },
        redact,
    )?;

    let used = read.tokens + memory.tokens;
    Ok(Prompt {
        tokens: used,
        scrollback_tokens: read.tokens,
        omitted: read.omitted,
        turns: read.turns,
        memory,
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
        // The start of a session: nothing has been said, and last week's lessons are all the
        // agent has. Memory should not be rationed against a scrollback that does not exist.
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
        // A session long enough to crowd memory out is exactly the one that will otherwise
        // rediscover something, so the floor is the whole point.
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
        // When it is not, something earlier exists that only memory can speak for.
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
        // A caller with a small budget and one with a large budget get the same shape, rather
        // than the small one losing memory entirely to a count somebody tuned once.
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
