//! The thresholds, in one place.
//!
//! Four numbers decide what balthasar keeps and what it says. They are written here rather than
//! where they are used because two files disagreeing about where assertion begins is the kind
//! of bug that produces a confident wrong answer and no error at all.
//!
//! Lua owns these from M2. This stays as what they are when nothing says otherwise.

/// Confidence at which a memory is **asserted** — presented to a model as current truth.
///
/// Above this, the model is told. Below it the memory is still there, still searchable, still
/// explained by `balthasar why`; it simply stops being stated as fact. That gap is the whole answer
/// to staleness: an agent that can say "you told me this in March, it may be stale" rather than
/// repeating it flatly.
pub const INJECT: f64 = 0.35;

/// Confidence at which a memory stays in the **live** set at all.
///
/// Below this it moves to the archive, keeping everything about it, and is searched only when
/// the live results are weak or `--archived` is asked for.
pub const LIVE: f64 = 0.10;

/// Score a candidate must reach to leave the session it was learned in.
///
/// Deliberately above the weight of a lone distillation: a thing that merely scrolled out of a
/// context window is a candidate, not a fact.
pub const PROMOTE: f64 = 0.5;

/// Score at which a candidate waits in scratch for a second witness rather than dying with the
/// session.
pub const HOLD: f64 = 0.3;

/// Strength below which a memory is swept out of the live set.
///
/// Separate from [`LIVE`], which is about evidence. This one is about use: a well-witnessed
/// fact nobody has needed in a year is confident and faint, and it is the faintness that
/// retires it.
pub const SPENT: f64 = 0.05;

// Checked when the crate is compiled rather than when the suite is run.
//
// The two floors are the design. Were they ever equal, a memory would go straight from being
// stated as fact to being archived, with nothing in between for an agent to hedge about — and
// that is a mistake worth refusing to build over rather than reporting later.
const _: () = assert!(
    INJECT > LIVE,
    "assertion must be a higher bar than being kept"
);
const _: () = assert!(
    PROMOTE > HOLD,
    "promoting must be a higher bar than holding"
);
const _: () = assert!(LIVE > 0.0, "everything would be kept forever");
const _: () = assert!(INJECT < 1.0, "nothing could ever be asserted");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lone_distillation_falls_between_them() {
        // §5.1's whole point, asserted here so a weight change cannot quietly break it.
        let alone = crate::WitnessKind::Distillation.weight();
        assert!(alone < PROMOTE, "one distillation does not make a fact");
        assert!(alone >= HOLD, "but it is worth waiting on");
    }
}
