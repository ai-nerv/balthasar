//! What another program may ask aeon, and what it may not.
//!
//! The dispatcher sits between the socket and the same functions the CLI calls. That is the
//! arrangement that keeps the two from drifting into describing a memory differently: there is
//! one implementation of `recall`, and this decides who is allowed to reach it.

mod ceiling;
mod dispatch;
mod verbs;
mod window;

pub use ceiling::Door;
pub use dispatch::{Answering, answer, answer_with};
pub use verbs::{NEVER, SURFACE, Verb, known};
pub use window::{observe, plan};
