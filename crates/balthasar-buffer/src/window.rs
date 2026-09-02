//! How much room there is, and how much of it is spoken for.

use balthasar_store::Turn;

/// What a harness said about its window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Window {
    /// The model's context window, in tokens.
    pub size: u32,
    /// What the reply needs, plus what a summarising call would need to run.
    ///
    /// Not decoration. Compacting costs a request of its own and that request needs room; a
    /// plan that filled the window to the brim would leave the summarisation itself as the
    /// thing that overflows.
    pub reserve: u32,
    /// The share of the window memory may claim for injection.
    pub inject: u32,
    /// A tool result larger than this is worth masking.
    pub mask_over: u32,
    /// How many recent turns are never touched.
    ///
    /// The detail still in play lives here — the file just read, the error just seen — and a
    /// summary of it is always worse than having it.
    pub keep: usize,
    /// Roughly what a masked turn's replacement costs.
    pub masked_cost: u32,
}

impl Default for Window {
    fn default() -> Self {
        Self {
            size: 200_000,
            reserve: 50_000,
            inject: 10_000,
            mask_over: 1_500,
            keep: 8,
            masked_cost: 40,
        }
    }
}

impl Window {
    /// How many tokens the conversation itself may occupy.
    ///
    /// Saturating, so a window smaller than its own reserve answers zero rather than wrapping
    /// to four billion — which would tell a harness it had unlimited room on exactly the
    /// configuration that has none.
    #[must_use]
    pub fn target(&self) -> u32 {
        self.size
            .saturating_sub(self.reserve)
            .saturating_sub(self.inject)
    }

    /// Whether a plan can possibly fit inside this window.
    ///
    /// A window whose reserve leaves no room for a conversation is a misconfiguration, and
    /// saying so beats returning a plan that will be refused by the provider.
    #[must_use]
    pub fn is_workable(&self) -> bool {
        self.target() > 0
    }
}

/// What a window currently holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Shape {
    /// What everything costs as it stands.
    pub used: u32,
    /// How many turns are still being sent verbatim.
    pub live: usize,
    /// How much of the cost is tool output.
    pub tools: u32,
}

impl Shape {
    /// Measure a ledger.
    #[must_use]
    pub fn of(entries: &[Turn], masked_cost: u32) -> Self {
        let mut shape = Self::default();
        for entry in entries {
            let cost = entry.cost(masked_cost);
            shape.used = shape.used.saturating_add(cost);
            if entry.state.is_live() {
                shape.live += 1;
            }
            if entry.role == "tool" {
                shape.tools = shape.tools.saturating_add(cost);
            }
        }
        shape
    }

    /// What fraction of the cost is tool output.
    ///
    /// The number that says whether masking will be enough. In a coding session it is usually
    /// most of the window, which is why masking is tried first.
    #[must_use]
    pub fn tool_share(&self) -> f64 {
        if self.used == 0 {
            return 0.0;
        }
        f64::from(self.tools) / f64::from(self.used)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use balthasar_store::State;

    fn turn(cursor: u64, role: &str, tokens: u32) -> Turn {
        Turn {
            cursor,
            role: role.into(),
            kind: "prose".into(),
            tool: None,
            tokens: Some(tokens),
            state: State::Live,
            at: 0,
            ..Turn::default()
        }
    }

    #[test]
    fn the_reserve_is_taken_out_of_what_a_conversation_may_use() {
        let window = Window {
            size: 1000,
            reserve: 200,
            inject: 100,
            ..Window::default()
        };
        assert_eq!(window.target(), 700);
    }

    #[test]
    fn a_window_smaller_than_its_reserve_says_so_rather_than_wrapping() {
        // Unsaturated, this answers four billion — telling a harness it has unlimited room on
        // exactly the configuration that has none.
        let window = Window {
            size: 100,
            reserve: 200,
            inject: 100,
            ..Window::default()
        };
        assert_eq!(window.target(), 0);
        assert!(!window.is_workable());
    }

    #[test]
    fn a_shape_counts_what_is_actually_sent() {
        let mut masked = turn(2, "tool", 3000);
        masked.state = State::Masked;
        let mut dropped = turn(3, "user", 500);
        dropped.state = State::Dropped;

        let shape = Shape::of(&[turn(1, "user", 100), masked, dropped], 40);
        assert_eq!(shape.used, 140);
        assert_eq!(shape.live, 1);
    }

    #[test]
    fn a_shape_says_how_much_of_the_window_is_tool_output() {
        // The number that decides whether masking will be enough, and in a coding session it
        // usually is.
        let shape = Shape::of(&[turn(1, "user", 100), turn(2, "tool", 900)], 40);
        assert!((shape.tool_share() - 0.9).abs() < 0.01);
    }

    #[test]
    fn an_empty_window_has_no_tool_share_rather_than_dividing_by_zero() {
        assert_eq!(Shape::of(&[], 40).tool_share(), 0.0);
    }
}
