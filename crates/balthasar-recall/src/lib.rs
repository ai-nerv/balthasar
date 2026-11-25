//! What a model is told, and in what order.

mod assemble;
mod budget;
mod classify;
mod model;
mod policy;
mod section;

pub use assemble::{Ask, Bound, Context, Rendered, assemble, assemble_refs, stores_for};
pub use budget::{CHARS_PER_TOKEN, fit, near_duplicate, share, tokens};
pub use classify::{Shape, shape_of};
pub use model::{FEATURES, LAYOUT, MINIMUM, Model};
pub use policy::{Policy, Shadow};
pub use section::{Filter, Section};
