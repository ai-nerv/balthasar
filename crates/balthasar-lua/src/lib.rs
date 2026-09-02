//! balthasar's configuration, which is a program rather than a data file.
//!
//! Settings are assigned, behaviour is registered, and the file returns nothing. Because it is
//! Lua it can probe the machine it is running on, loop over a directory, and branch — which is
//! the whole reason a configuration format was not enough.
//!
//! # The two namespaces
//!
//! A config author must be able to tell a handler's return contract without checking, so balthasar
//! spends a namespace on it:
//!
//! | | contract | a raise |
//! |---|---|---|
//! | `balthasar.on.<question>` | **asked.** `nil` = not mine, carry on. A table = do this instead. First non-`nil` wins. | reported, that handler skipped, the rest still run |
//! | `balthasar.did.<verb>` | **told.** Pure side effect; the return value is ignored. | reported and survived |
//!
//! `on.` is asked; `did.` is told. Nothing else in the API takes a function, so there is no
//! third contract to remember.

mod client;
mod config;
mod convert;
mod engine;
mod handler;
mod helpers;
mod plugins;
mod settings;
mod stream;

pub use client::CLIENT;
pub use config::{Config, Registered};
pub use engine::Engine;
pub use engine::{PRIVILEGED, PRIVILEGED_SETTINGS, REGISTRARS, SPECS};
pub use handler::{ASKED, TOLD};
pub use helpers::glob_paths;
pub use plugins::{Roots, Trust, runtimepath, vouched_for};
pub use settings::{Budget, Decay, Floors, Ledger, Settings, Weights};

/// What went wrong while reading a configuration.
///
/// A file that exists and does not load is fatal: it expressed an intention that has not been
/// carried out, and applying half of it is worse than refusing.
#[derive(Debug, thiserror::Error)]
pub enum LuaError {
    /// The file could not be read.
    #[error("{file}: {source}")]
    Io {
        /// Which file.
        file: String,
        /// Why not.
        #[source]
        source: std::io::Error,
    },
    /// It would not parse.
    #[error("{file}: {message}")]
    Syntax {
        /// Which file.
        file: String,
        /// What Lua said.
        message: String,
    },
    /// It parsed and then raised.
    #[error("{file}: {message}")]
    Runtime {
        /// Which file.
        file: String,
        /// What Lua said.
        message: String,
    },
    /// A project file tried to declare something only the owner may declare.
    #[error("{file} may choose but not declare: {what}")]
    Untrusted {
        /// Which file.
        file: String,
        /// What it tried to declare.
        what: String,
    },
}
