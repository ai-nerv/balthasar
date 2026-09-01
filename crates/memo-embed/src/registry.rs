//! Choosing an embedder, and falling back when the chosen one is not there.

use crate::{Embed, Hashed};

/// Which backend a configuration asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The local, dependency-free one. Coarse, deterministic, always available.
    Hashed,
    /// A bundled transformer, downloaded once.
    ///
    /// Named here so a configuration can ask for it and be told plainly that this build does
    /// not carry it, rather than having its setting silently ignored.
    Onnx,
    /// Nothing. Lexical search alone.
    None,
}

/// What a configuration said about embedding.
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    /// Which backend.
    pub kind: Kind,
    /// Which model, for the backends that have a choice.
    pub model: String,
}

impl Default for Spec {
    fn default() -> Self {
        Self {
            kind: Kind::Hashed,
            model: "hashed-3gram-256".to_owned(),
        }
    }
}

impl Spec {
    /// Read what a configuration declared.
    ///
    /// An unrecognised `kind` is not an error and not silence either: it falls back to the
    /// local embedder and says which one it actually got, because a setting that is quietly
    /// ignored is worse than one that is refused.
    #[must_use]
    pub fn read(said: Option<&serde_json_lite::Value>) -> Self {
        let Some(said) = said else {
            return Self::default();
        };
        let kind = match said.kind.as_deref() {
            Some("onnx") => Kind::Onnx,
            Some("none") => Kind::None,
            _ => Kind::Hashed,
        };
        Self {
            kind,
            model: said.model.clone().unwrap_or_else(|| match kind {
                Kind::Onnx => "bge-small-en-v1.5".to_owned(),
                _ => "hashed-3gram-256".to_owned(),
            }),
        }
    }
}

/// The bits of a declaration this crate reads, without depending on a JSON library.
///
/// A tiny struct rather than `serde_json::Value`, so `memo-embed` stays a leaf: it has one
/// dependency and no reason for a second.
pub mod serde_json_lite {
    /// An embedder declaration.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct Value {
        /// `hashed`, `onnx` or `none`.
        pub kind: Option<String>,
        /// Which model.
        pub model: Option<String>,
    }
}

/// Open the embedder a spec asks for, or the nearest one that works.
///
/// Answers what it actually opened, so a caller can say so. `None` means nothing is embedding
/// and recall is lexical, which is a supported state rather than a failure.
#[must_use]
pub fn open(spec: &Spec) -> Option<Box<dyn Embed>> {
    match spec.kind {
        Kind::None => None,
        // The transformer backend is a later milestone. Until it exists, asking for it gets the
        // local one — worse at paraphrase, present on every machine — rather than nothing,
        // because falling all the way back to no vectors is a bigger change than the caller
        // asked for.
        Kind::Onnx | Kind::Hashed => Some(Box::new(Hashed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saying_nothing_gets_the_local_embedder() {
        let spec = Spec::read(None);
        assert_eq!(spec.kind, Kind::Hashed);
        assert!(open(&spec).is_some());
    }

    #[test]
    fn asking_for_nothing_gets_nothing() {
        let spec = Spec::read(Some(&serde_json_lite::Value {
            kind: Some("none".into()),
            model: None,
        }));
        assert!(open(&spec).is_none(), "lexical alone is a supported state");
    }

    #[test]
    fn an_unrecognised_backend_falls_back_rather_than_failing() {
        let spec = Spec::read(Some(&serde_json_lite::Value {
            kind: Some("something-else".into()),
            model: None,
        }));
        assert_eq!(spec.kind, Kind::Hashed);
    }

    #[test]
    fn what_is_opened_names_itself() {
        // The model name goes on every row it produces, so a later change can be noticed
        // rather than silently compared against.
        let opened = open(&Spec::default()).expect("something");
        assert_eq!(opened.model(), "hashed-3gram-256");
        assert!(opened.dimensions() > 0);
    }
}
