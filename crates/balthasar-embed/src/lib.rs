//! Vectors: a ranking signal, never the floor. Lexical search answers on its own and always,
//! and nothing above this has to know whether an embedder exists.

#[cfg(feature = "dense")]
mod dense;
mod hashed;
mod registry;

#[cfg(feature = "dense")]
pub use dense::Dense;
pub use hashed::Hashed;
pub use registry::{Kind, Spec, open, open_explaining, serde_json_lite::Value};

/// What went wrong.
#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("the '{0}' embedder is not available: {1}")]
    Unavailable(String, String),
    #[error("could not embed: {0}")]
    Failed(String),
}

/// Something that turns text into a vector.
pub trait Embed {
    /// The model's name, stored beside every vector: a model change invalidates every vector.
    fn model(&self) -> &str;

    fn dimensions(&self) -> usize;

    /// Embed a batch.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;

    /// Whether this embedder can answer right now, checked before use.
    fn ready(&self) -> bool {
        true
    }
}

/// Normalise a vector to unit length, in place, so cosine similarity is a dot product.
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
