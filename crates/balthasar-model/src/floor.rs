/// Confidence at which a memory is asserted to a model as current truth.
pub const INJECT: f64 = 0.35;

/// Confidence at which a memory stays in the live set rather than the archive.
pub const LIVE: f64 = 0.10;

/// Score a candidate must reach to leave the session it was learned in.
pub const PROMOTE: f64 = 0.5;

/// Score at which a candidate waits in scratch for a second witness.
pub const HOLD: f64 = 0.3;

/// Strength below which a memory is swept out of the live set.
pub const SPENT: f64 = 0.05;

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
        let alone = crate::WitnessKind::Distillation.weight();
        assert!(alone < PROMOTE, "one distillation does not make a fact");
        assert!(alone >= HOLD, "but it is worth waiting on");
    }
}
