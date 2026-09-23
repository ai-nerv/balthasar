//! How the room is split and when to act, as `balthasar.window` says. Every number is a share of
//! what is left after the fixed items and the reply, so one set of rules fits 32k and 1M windows.

use serde_json::Value;

/// Shares of the room held for what is not the conversation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shares {
    pub memory: f64,
    pub pinned: f64,
    pub summary: f64,
}

/// The rules one layout is made by.
#[derive(Debug, Clone, PartialEq)]
pub struct Rules {
    pub shares: Shares,
    /// The share of the window one request may occupy, and what is held back against a
    /// provider counting differently.
    pub policy: crate::Policy,
    /// Start stubbing old tool results, as a share of the conversation's room.
    pub prune_at: f64,
    /// Ask for a summary of the oldest turns.
    pub compact_at: f64,
    /// How far a summary should bring the conversation down.
    pub compact_to: f64,
    /// Tell the main model the conversation is getting long.
    pub warn_at: f64,
    /// The last N user turns stay word for word.
    pub keep_turns: usize,
    /// And the last N tool results.
    pub keep_results: usize,
    /// Only results bigger than this are worth stubbing.
    pub stub_over: u32,
    /// Stub only when that frees at least this share: in batches, for the prompt cache.
    pub prune_min: f64,
    /// Summaries asked for per prompt, at most.
    pub max_compactions_per_prompt: u32,
    /// What balthasar's own estimates divide a length by.
    pub estimate_chars_per_token: u32,
    /// Seconds after which a provider's prompt cache is taken to be gone.
    pub cache_ttl_s: u64,
    /// What the warning note says.
    pub warning: String,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            policy: crate::Policy::default(),
            shares: Shares {
                memory: 0.05,
                pinned: 0.03,
                summary: 0.08,
            },
            prune_at: 0.70,
            compact_at: 0.85,
            compact_to: 0.50,
            warn_at: 0.75,
            keep_turns: 2,
            keep_results: 3,
            stub_over: 1_500,
            prune_min: 0.10,
            max_compactions_per_prompt: 3,
            estimate_chars_per_token: 4,
            cache_ttl_s: 300,
            // A statement, not a chore. Keeping what matters is this layer's work, and asking
            // the model to do it spends a turn on infrastructure it cannot see the state of.
            warning: "(this conversation is long enough that earlier turns are being \
                      summarised rather than sent word for word)"
                .to_owned(),
        }
    }
}

impl Rules {
    /// Read `balthasar.window`, keeping the shipped value for anything missing or unusable.
    #[must_use]
    pub fn read(said: Option<&Value>) -> Self {
        let base = Self::default();
        let Some(said) = said else {
            return base;
        };
        let share = |value: Option<&Value>, fallback: f64| {
            value
                .and_then(Value::as_f64)
                .filter(|n| (0.0..=1.0).contains(n))
                .unwrap_or(fallback)
        };
        let count =
            |name: &str, fallback: u64| said.get(name).and_then(Value::as_u64).unwrap_or(fallback);
        let held = said.get("shares");
        // A share outside (0, 1] or a non-finite one is no policy at all, and a request planned
        // against one would be planned against nothing. The shipped value stands instead.
        let budget = said.get("budget");
        let policy = crate::Policy {
            share: budget
                .and_then(|b| b.get("share"))
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite() && *n > 0.0 && *n <= 1.0)
                .unwrap_or(base.policy.share),
            margin: budget
                .and_then(|b| b.get("margin"))
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(base.policy.margin),
        };
        let mut rules = Self {
            policy,
            shares: Shares {
                memory: share(held.and_then(|s| s.get("memory")), base.shares.memory),
                pinned: share(held.and_then(|s| s.get("pinned")), base.shares.pinned),
                summary: share(held.and_then(|s| s.get("summary")), base.shares.summary),
            },
            prune_at: share(said.get("prune_at"), base.prune_at),
            compact_at: share(said.get("compact_at"), base.compact_at),
            compact_to: share(said.get("compact_to"), base.compact_to),
            warn_at: share(said.get("warn_at"), base.warn_at),
            keep_turns: count("keep_turns", base.keep_turns as u64) as usize,
            keep_results: count("keep_results", base.keep_results as u64) as usize,
            stub_over: u32::try_from(count("stub_over", base.stub_over.into()))
                .unwrap_or(base.stub_over),
            prune_min: share(said.get("prune_min"), base.prune_min),
            max_compactions_per_prompt: u32::try_from(count(
                "max_compactions_per_prompt",
                base.max_compactions_per_prompt.into(),
            ))
            .unwrap_or(base.max_compactions_per_prompt),
            estimate_chars_per_token: u32::try_from(count(
                "estimate_chars_per_token",
                base.estimate_chars_per_token.into(),
            ))
            .unwrap_or(base.estimate_chars_per_token)
            .max(1),
            cache_ttl_s: count("cache_ttl_s", base.cache_ttl_s),
            warning: said
                .get("warning")
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
                .map_or(base.warning, str::to_owned),
        };
        // Shares that leave the conversation almost nothing are refused together.
        if rules.shares.memory + rules.shares.pinned + rules.shares.summary > 0.9 {
            rules.shares = base.shares;
        }
        if rules.compact_to >= rules.compact_at {
            rules.compact_to = base.compact_to.min(rules.compact_at / 2.0);
        }
        rules
    }

    /// What `text` is estimated to cost, before any correction.
    #[must_use]
    pub fn estimate(&self, text: &str) -> u32 {
        u32::try_from(text.len().div_ceil(self.estimate_chars_per_token as usize))
            .unwrap_or(u32::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saying_nothing_is_the_shipped_rules() {
        assert_eq!(Rules::read(None), Rules::default());
        assert_eq!(Rules::read(Some(&serde_json::json!({}))), Rules::default());
    }

    #[test]
    fn one_knob_can_be_set_alone() {
        let rules = Rules::read(Some(&serde_json::json!({
            "shares": { "memory": 0.1 }, "prune_at": 0.6, "keep_turns": 4,
        })));
        assert_eq!(rules.shares.memory, 0.1);
        assert_eq!(rules.shares.summary, 0.08);
        assert_eq!(rules.prune_at, 0.6);
        assert_eq!(rules.keep_turns, 4);
        assert_eq!(rules.compact_at, 0.85);
    }

    #[test]
    fn nonsense_falls_back_rather_than_breaking_the_window() {
        let rules = Rules::read(Some(&serde_json::json!({
            "shares": { "memory": 0.5, "pinned": 0.3, "summary": 0.3 },
            "prune_at": 7, "compact_to": 0.9, "estimate_chars_per_token": 0,
        })));
        assert_eq!(rules.shares, Rules::default().shares);
        assert_eq!(rules.prune_at, 0.70);
        assert!(rules.compact_to < rules.compact_at);
        assert_eq!(rules.estimate_chars_per_token, 1);
    }
}
