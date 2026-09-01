//! Vectors.
//!
//! A ranking signal, never the floor. Lexical search answers on its own and always; an
//! embedding makes the answer better when there is one. Everything here is arranged so that
//! nothing above it has to know whether an embedder exists — the store keeps `Option<Vec<f32>>`
//! and the scorer redistributes the weight when it is `None`.
//!
//! # What is here, and what is not
//!
//! The trait, the model registry, and a local embedder that needs no network and no model file.
//! The bundled transformer (`bge-small-en-v1.5`) is a backend behind this same trait; it is
//! deliberately not the only one, because a memory layer whose search degrades to nothing
//! without a 30 MB download is a memory layer that fails on the machine that needed it most.

mod hashed;
mod registry;

pub use hashed::Hashed;
pub use registry::{Kind, Spec, open, serde_json_lite::Value};

/// What went wrong.
#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    /// The model could not be found or loaded.
    #[error("the '{0}' embedder is not available: {1}")]
    Unavailable(String, String),
    /// The text could not be embedded.
    #[error("could not embed: {0}")]
    Failed(String),
}

/// Something that turns text into a vector.
pub trait Embed {
    /// The model's name, which is stored beside every vector it produces.
    ///
    /// A model change invalidates every vector: comparing a `bge-small` embedding against a
    /// `bge-base` one produces a number, and the number is meaningless. Keeping the name on the
    /// row is what lets `doctor` notice and `reindex` fix it.
    fn model(&self) -> &str;

    /// How many dimensions it produces.
    fn dimensions(&self) -> usize;

    /// Embed a batch.
    ///
    /// A batch rather than one at a time because every real backend is faster that way, and a
    /// caller that only has one has a batch of one.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;

    /// Whether this embedder can answer right now.
    ///
    /// Checked before use, so a backend whose model has not been downloaded falls through to
    /// the lexical floor rather than failing an ingest.
    fn ready(&self) -> bool {
        true
    }
}

/// Normalise a vector to unit length, in place.
///
/// Cosine similarity of unit vectors is a dot product, and every backend here answers unit
/// vectors so the scorer never has to care which produced them.
pub fn normalise(vector: &mut [f32]) {
    let magnitude: f32 = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    if magnitude > 0.0 {
        for value in vector.iter_mut() {
            *value /= magnitude;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_normalised_vector_has_unit_length() {
        let mut v = vec![3.0_f32, 4.0];
        normalise(&mut v);
        let magnitude: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((magnitude - 1.0).abs() < 1e-6, "{magnitude}");
    }

    #[test]
    fn normalising_nothing_does_not_divide_by_zero() {
        let mut v = vec![0.0_f32, 0.0];
        normalise(&mut v);
        assert_eq!(v, [0.0, 0.0]);
    }
}
