//! How much of a model's window one request may occupy: `L = min(floor(C * share), C - R) - M`,
//! admitting an input within `L` whose `input + R + M` is within `C`. Capacity, ceiling, reserve,
//! margin and limit stay apart, so a refusal can say which of them bound it.

/// `share` of capacity an input may take, as `0.0..=1.0`, and the `margin` held back against the
/// difference between an estimate and what a provider counts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Policy {
    pub share: f64,
    pub margin: u32,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            share: 0.50,
            margin: 0,
        }
    }
}

/// How the tokens in a request were counted, and so what a limit over them is worth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Counting {
    /// Counted by estimate and correction; a limit over it is an estimated limit.
    #[default]
    Estimated,
    /// Counted for the serialized request as the provider counts it, within a documented bound.
    Verified,
}

impl Counting {
    /// Read what a peer said it can do. Anything absent or unrecognised is [`Self::Estimated`],
    /// which is what a peer that says nothing was already doing.
    #[must_use]
    pub fn read(said: Option<&str>) -> Self {
        match said {
            Some("verified") => Self::Verified,
            _ => Self::Estimated,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Estimated => "estimated",
            Self::Verified => "verified",
        }
    }

    /// Whether a ceiling over this counting can be certified rather than estimated.
    #[must_use]
    pub const fn certifiable(self) -> bool {
        matches!(self, Self::Verified)
    }
}

/// Why no request at all fits, as distinct from one request being too large.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unsatisfiable {
    UnknownCapacity,
    UnusableShare,
    ReplyFillsCapacity { capacity: u32, reply: u32 },
    MarginFillsRoom { room: u32, margin: u32 },
}

/// The room one request has: `C`, `H`, `R`, `M` and the `L` they produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Room {
    pub capacity: u32,
    pub ceiling: u32,
    pub reply: u32,
    pub margin: u32,
    pub limit: u32,
}

/// What a count of a complete input came to against its room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Fits { spare: u32 },
    Over { by: u32 },
}

impl Admission {
    #[must_use]
    pub const fn fits(self) -> bool {
        matches!(self, Self::Fits { .. })
    }
}

impl Room {
    /// Work out the room a policy leaves, or why it leaves none.
    ///
    /// # Errors
    /// [`Unsatisfiable`] when the capacity is unknown, the share is not a fraction, or the
    /// reservation and margin together leave no room for any input at all.
    pub fn of(policy: Policy, capacity: Option<u32>, reply: u32) -> Result<Self, Unsatisfiable> {
        let Some(capacity) = capacity.filter(|window| *window > 0) else {
            return Err(Unsatisfiable::UnknownCapacity);
        };
        if !policy.share.is_finite() || policy.share <= 0.0 || policy.share > 1.0 {
            return Err(Unsatisfiable::UnusableShare);
        }
        let ceiling = share_of(capacity, policy.share);
        let Some(after_reply) = capacity.checked_sub(reply).filter(|left| *left > 0) else {
            return Err(Unsatisfiable::ReplyFillsCapacity { capacity, reply });
        };
        let room = ceiling.min(after_reply);
        let Some(limit) = room.checked_sub(policy.margin).filter(|left| *left > 0) else {
            return Err(Unsatisfiable::MarginFillsRoom {
                room,
                margin: policy.margin,
            });
        };
        Ok(Self {
            capacity,
            ceiling,
            reply,
            margin: policy.margin,
            limit,
        })
    }

    /// Whether a complete input of `count` may be sent: within `L`, and with `R` and `M` still
    /// inside `C`.
    #[must_use]
    pub fn admits(self, count: u32) -> Admission {
        let over_limit = count.saturating_sub(self.limit);
        let whole = count.saturating_add(self.reply).saturating_add(self.margin);
        let over_capacity = whole.saturating_sub(self.capacity);
        match over_limit.max(over_capacity) {
            0 => Admission::Fits {
                spare: self.limit - count,
            },
            by => Admission::Over { by },
        }
    }
}

/// A share of a window, rounded down, and never more than the window itself.
fn share_of(capacity: u32, share: f64) -> u32 {
    let taken = (f64::from(capacity) * share).floor();
    if taken >= f64::from(u32::MAX) {
        return capacity;
    }
    (taken as u32).min(capacity)
}

#[cfg(test)]
#[path = "policy/tests.rs"]
mod tests;
