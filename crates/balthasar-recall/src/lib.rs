//! What a model is told, and in what order.

mod advisor;
mod assemble;
mod budget;
mod classify;
mod compose;
mod model;
mod policy;
mod section;
mod spans;

pub use advisor::{Advisory, Forbidden, Proposal, Stage};
pub use assemble::{Ask, Bound, Context, Rendered, assemble, stores_for};
pub use budget::{CHARS_PER_TOKEN, fit, near_duplicate, share, tokens};
pub use classify::{Shape, shape_of};
pub use compose::{Prompt, Split, compose};
pub use model::{FEATURES, LAYOUT, MINIMUM, Model};
pub use policy::{Policy, Shadow};
pub use section::{Filter, Section};
pub use spans::{Quoted, Quotes, quote};
