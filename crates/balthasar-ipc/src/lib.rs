//! balthasar's socket: the end of the family arrangement that listens, offering the handful of
//! things another program has a reason to ask a memory layer.

mod corpses;
mod encoding;
mod frame;
mod peer;
mod serve;
mod stop;

pub use corpses::swept;
pub use encoding::Wire;
pub use frame::{FAMILY, Fault, MAX_FRAME, Reply, Request, SURFACE, WireError, recv, send};
pub use peer::Peer;
pub use serve::{Listener, socket_dir, socket_path, tool_descriptor};
pub use stop::hold as hold_stop_signals;
