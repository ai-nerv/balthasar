//! What another program may ask balthasar, and what it may not.

mod ceiling;
mod dispatch;
mod hooks;
mod layout;
mod outcome;
mod supply;
mod verbs;
mod window;

pub use balthasar_buffer::Rules;
pub use ceiling::Door;
pub use dispatch::{Answering, answer, answer_hooked, answer_with};
pub use hooks::{Describing, Hooks};
pub use verbs::{NEVER, SURFACE, Verb, known};
pub use window::{observe, plan};
