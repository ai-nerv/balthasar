//! What a harness should send.
//!
//! A harness does not ask "should I compact". It asks **what should I send**, and gets an
//! instruction set back. That inversion is the point: compaction from inside a turn has a
//! character count and nothing else, and this has the whole session.
//!
//! The order is not negotiable and it is most of the design:
//!
//! ```text
//!   pin   ──►  what a harness said not to touch
//!   mask  ──►  free, reversible, and where a coding agent's tokens actually are
//!   drop  ──►  what a summary already covers
//!   sum   ──►  a model call, lossy, non-deterministic. The last resort, never the first.
//! ```
//!
//! Masking and summarising are not equals. Masking costs nothing, is reversible because the
//! text is still in scratch, and targets tool output — which is most of a coding session's
//! window. Summarising costs a request, loses information irreversibly, and does it differently
//! every run. Reaching for the second before exhausting the first is the mistake every harness
//! makes.

mod plan;
mod window;

pub use plan::{Masked, Plan, Span, plan};
pub use window::{Shape, Window};
