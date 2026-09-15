//! The ladder: what crosses from a session into memory, and why.
//!
//! Six paths, each with its own evidence and its own weight, and a candidate crosses when any one
//! of them admits it. They are independent witnesses to the same claim, not stages.

mod akin;
mod candidate;
mod clash;
mod consolidate;
mod derive;
mod distil;
mod episode;
mod extract;
mod infer;
pub(crate) mod ingest;
mod instruction;
mod observation;
mod own;
mod segment;

pub use akin::{Akin, merge};
pub use candidate::{Candidate, Decided, Verdict, weigh};
pub use clash::clashes;
pub use consolidate::{Consolidated, DISTINCT_SESSIONS, RUNS_PER_PASS, consolidate};
pub use derive::{
    DERIVATION as RELATION_DERIVATION, Step, Thresholds, entities, overlap, repairs, temporal,
};
pub use distil::{Budget, Distil, DistilFailure, Spawned, backends, first_answer};
pub use episode::{Told, avoidance, tell};
pub use extract::{Extracted, extract};
pub use infer::propose;
pub use ingest::{EXTRACTOR_VERSION, Ingest, Provenance, Report, Source, ingest};
pub use instruction::{Instruction, read as read_instruction};
pub use observation::{Kind, Meta, Observation, Role};
pub use own::{SOURCE as TRANSCRIPT_SOURCE, distil_run, undistilled};
pub use segment::{Boundary, DERIVATION, METHOD, Rules as SegmentRules, Segment, Signal, segment};

/// What went wrong while reading somebody else's transcripts.
#[derive(Debug, thiserror::Error)]
pub enum DistilError {
    /// The store said no.
    #[error(transparent)]
    Store(#[from] balthasar_store::StoreError),
    /// A configuration file did not load.
    #[error(transparent)]
    Lua(#[from] balthasar_lua::LuaError),
    #[error("no source called '{0}' — declare one with balthasar.source(\"{0}\", …)")]
    NoSource(String),
    #[error("the source '{0}' declares no {1}()")]
    Incomplete(String, &'static str),
    /// A file could not be read.
    #[error("{0}: {1}")]
    Io(String, #[source] std::io::Error),
}
