//! balthasar's socket.
//!
//! Layer three of the family's arrangement. Layers one and two — the stream primitive and the
//! plain-Lua client stub — are copied between siblings so that a fix to the framing reaches all
//! of them; this is the end that listens.
//!
//! **The surface is small on purpose.** It is not a mirror of what balthasar can do; it is the
//! handful of things another program has a real reason to ask a memory layer. `prompt`, `run`
//! and `eval` are absent and stay absent.

mod frame;
mod peer;
mod serve;

pub use frame::{MAX_FRAME, Reply, Request, WireError, recv, send};
pub use peer::Peer;
pub use serve::{Listener, socket_dir, socket_path, tool_descriptor};
