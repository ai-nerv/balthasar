//! An embedder that needs nothing.
//!
//! Hashed character n-grams projected into a fixed number of dimensions. It knows nothing about
//! meaning: what it captures is surface similarity, so `make test` and `make tests` land close
//! together and `make test` and `cargo build` do not.

use crate::{Embed, EmbedError, normalise};

/// How wide a vector this produces.
const DIMENSIONS: usize = 256;

/// The n-gram width: three characters catches stems and typos without matching every word that
/// shares a vowel.
const GRAM: usize = 3;

/// A local, dependency-free embedder.
#[derive(Debug, Clone, Copy, Default)]
pub struct Hashed;

impl Embed for Hashed {
    fn model(&self) -> &str {
        "hashed-3gram-256"
    }

    fn dimensions(&self) -> usize {
        DIMENSIONS
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        Ok(texts.iter().map(|text| vector(text)).collect())
    }
}

/// One text as a unit vector.
fn vector(text: &str) -> Vec<f32> {
    let mut out = vec![0.0_f32; DIMENSIONS];
    let normalised: Vec<char> = text
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();

    // Whole words as well as their n-grams, so an exact match counts for more than fragments.
    for word in normalised.iter().collect::<String>().split_whitespace() {
        add(&mut out, word, 2.0);
        let chars: Vec<char> = word.chars().collect();
        if chars.len() > GRAM {
            for window in chars.windows(GRAM) {
                add(&mut out, &window.iter().collect::<String>(), 1.0);
            }
        }
    }
    normalise(&mut out);
    out
}

/// Add one token's weight to the dimension it hashes to.
///
/// Signed, so two tokens landing in one dimension are as likely to cancel as to reinforce;
/// unsigned hashing makes every collision look like agreement.
fn add(out: &mut [f32], token: &str, weight: f32) {
    let hash = fnv(token.as_bytes());
    let at = (hash % DIMENSIONS as u64) as usize;
    let sign = if hash & (1 << 63) == 0 { 1.0 } else { -1.0 };
    out[at] += weight * sign;
}

/// FNV-1a. Small, fast, well-spread, and no dependency.
fn fnv(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn similarity(a: &str, b: &str) -> f64 {
        let vectors = Hashed.embed(&[a.to_owned(), b.to_owned()]).expect("embeds");
        let dot: f32 = vectors[0].iter().zip(&vectors[1]).map(|(x, y)| x * y).sum();
        f64::from(dot)
    }

    #[test]
    fn the_same_text_is_maximally_similar_to_itself() {
        assert!((similarity("make test", "make test") - 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_near_miss_is_nearer_than_a_different_thing() {
        let near = similarity("we run the tests", "we run the test");
        let far = similarity("we run the tests", "the database is postgres");
        assert!(near > far, "near {near} should beat far {far}");
        assert!(
            near > 0.7,
            "a one-letter difference should stay close: {near}"
        );
    }

    #[test]
    fn unrelated_text_is_not_similar() {
        let value = similarity("the build takes forty seconds", "her name is Sam");
        assert!(value < 0.3, "{value}");
    }

    #[test]
    fn word_order_does_not_decide_meaning() {
        // A bag of n-grams, so this is expected rather than a defect.
        let value = similarity("make test", "test make");
        assert!(value > 0.9, "{value}");
    }

    #[test]
    fn every_vector_is_the_declared_width_and_unit_length() {
        for text in ["", "a", "a much longer piece of prose about several things"] {
            let v = &Hashed.embed(&[text.to_owned()]).expect("embeds")[0];
            assert_eq!(v.len(), Hashed.dimensions());
            let magnitude: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            assert!(
                magnitude < 1e-6 || (magnitude - 1.0).abs() < 1e-5,
                "{text:?} gave {magnitude}"
            );
        }
    }

    #[test]
    fn it_is_deterministic() {
        assert_eq!(
            Hashed.embed(&["make test".to_owned()]).expect("a"),
            Hashed.embed(&["make test".to_owned()]).expect("b")
        );
    }

    #[test]
    fn collisions_are_as_likely_to_cancel_as_to_agree() {
        let v = &Hashed
            .embed(&["a varied sentence with many different tokens in it".to_owned()])
            .expect("embeds")[0];
        assert!(v.iter().any(|x| *x < 0.0), "no dimension came out negative");
    }
}
