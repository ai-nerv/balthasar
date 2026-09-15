//! balthasar's configuration, which is a program rather than a data file.
//!
//! Settings are assigned, behaviour is registered, and the file returns nothing.
//! `balthasar.on.<question>` is asked: `nil` means not mine, a table means do this instead, and
//! the first non-`nil` wins. `balthasar.did.<verb>` is told: its return value is ignored.

pub mod acknowledged;
mod client;
mod config;
mod convert;
mod engine;
mod handler;
mod helpers;
mod plugins;
mod sandbox;
mod settings;
pub mod setup;
mod stream;

pub use client::CLIENT;
pub use config::{Config, Registered};
pub use engine::Engine;
pub use engine::{PRIVILEGED, PRIVILEGED_SETTINGS, REGISTRARS, SPECS};
pub use handler::{ASKED, TOLD};
pub use helpers::glob_paths;
pub use plugins::{Roots, Trust, installed, runtimepath, vouched_for};
pub use settings::{Budget, Decay, Floors, Ledger, Settings, Weights};

/// What went wrong while reading a configuration.
#[derive(Debug, thiserror::Error)]
pub enum LuaError {
    #[error("{file}: {source}")]
    Io {
        file: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{file}: {message}")]
    Syntax { file: String, message: String },
    #[error("{file}: {message}")]
    Runtime { file: String, message: String },
    /// A project file tried to declare something only the owner may declare.
    #[error("{file} may choose but not declare: {what}")]
    Untrusted { file: String, what: String },
}
