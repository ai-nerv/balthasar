//! What a harness should send: an instruction set, rather than an answer to "should I compact".
//! The order is pin, then mask, then drop, then sum — a model call is the last resort.

mod layout;
mod plan;
mod policy;
mod rules;
mod window;

pub use layout::{
    Ask, Budget, Compacting, Covered, Held, Layout, Row, Slot, account, budget, corrected, lay,
};
pub use plan::{Masked, Plan, Span, plan};
pub use policy::{Admission, Counting, Policy, Room, Unsatisfiable};
pub use rules::{Rules, Shares};
pub use window::{Shape, Window};
