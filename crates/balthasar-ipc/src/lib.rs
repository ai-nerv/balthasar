//! balthasar's socket: the end of the family arrangement that listens, offering the handful of
//! things another program has a reason to ask a memory layer.

mod corpses;
mod encoding;
mod frame;
mod key;
mod peer;
mod serve;
mod stale;
mod stop;

pub use corpses::swept;
pub use encoding::Wire;
pub use frame::{
    FAMILY, Fault, MAX_FRAME, Reply, Request, SURFACE, WireError, recv, recv_within, send,
};
pub use key::{Key, REFUSED as KEY_REFUSED};
pub use peer::{AGENT_VARS, Peer, named_in, program_name};
pub use serve::{
    Listener, legacy_socket_dir, legacy_socket_path, socket_dir, socket_dirs, socket_path,
    tool_descriptor,
};
pub use stale::lock_path;
pub use stop::hold as hold_stop_signals;
