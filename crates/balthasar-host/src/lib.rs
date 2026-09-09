//! What another program may ask balthasar, and what it may not.

mod ceiling;
mod dispatch;
mod outcome;
mod verbs;
mod window;

pub use ceiling::Door;
pub use dispatch::{Answering, answer, answer_with};
pub use verbs::{NEVER, SURFACE, Verb, known};
pub use window::{observe, plan};
