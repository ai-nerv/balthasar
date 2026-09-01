//! What a model is told, and in what order.
//!
//! Retrieval answers "which memories"; this answers "which of them are worth the room". The
//! two are separate because the second is a budget problem and the first is a ranking one, and
//! conflating them is how a context ends up full of the most similar things rather than the
//! most useful ones.

mod assemble;
mod budget;
mod classify;
mod section;

pub use assemble::{Ask, Bound, Context, Rendered, assemble, stores_for};
pub use budget::{CHARS_PER_TOKEN, fit, near_duplicate, share, tokens};
pub use classify::{Shape, shape_of};
pub use section::{Filter, Section};
