//! What a harness should send: an instruction set, rather than an answer to "should I compact".
//! The order is pin, then mask, then drop, then sum — a model call is the last resort.

mod plan;
mod window;

pub use plan::{Masked, Plan, Span, plan};
pub use window::{Shape, Window};
