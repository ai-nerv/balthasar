//! The ladder: what crosses from a session into memory, and why.
//!
//! Six paths, each with its own evidence and its own weight, and a candidate crosses when any
//! one of them admits it. They are not stages — they are independent witnesses to the same
//! claim, which is what lets a thing nobody said twice still be remembered because it was
//! expensive to learn.
//!
//! What is built so far is the extractive half: rules that need no model, so the whole thing
//! works with no key and no network. A distiller makes it better and its absence never makes
//! it fail.

mod akin;
mod candidate;
mod consolidate;
mod derive;
mod distil;
mod episode;
mod extract;
pub(crate) mod ingest;
mod instruction;
mod observation;
mod own;
mod segment;

pub use akin::{Akin, claim_overlap, merge, same_claim};
pub use candidate::{Candidate, Decided, Verdict, weigh};
pub use consolidate::{Consolidated, DISTINCT_SESSIONS, RUNS_PER_PASS, consolidate};
pub use derive::{
    DERIVATION as RELATION_DERIVATION, Step, Thresholds, entities, overlap, repairs, temporal,
};
pub use distil::{Budget, Distil, DistilFailure, Spawned, backends, first_answer};
pub use episode::{Told, avoidance, tell};
pub use extract::{Extracted, extract};
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
    Store(#[from] memo_store::StoreError),
    /// A configuration file did not load.
    #[error(transparent)]
    Lua(#[from] memo_lua::LuaError),
    /// The source is not one any configuration declared.
    #[error("no source called '{0}' — declare one with memo.source(\"{0}\", …)")]
    NoSource(String),
    /// The source declared no way to find sessions.
    #[error("the source '{0}' declares no {1}()")]
    Incomplete(String, &'static str),
    /// A file could not be read.
    #[error("{0}: {1}")]
    Io(String, #[source] std::io::Error),
}
