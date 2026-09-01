//! Choosing an embedder, and falling back when the chosen one is not there.

use crate::{Embed, Hashed};

/// Which backend a configuration asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The local, dependency-free one. Coarse, deterministic, always available.
    Hashed,
    /// A sentence transformer, from weights on disk.
    ///
    /// Present only in a build made with `--features dense`. Named here either way, so a
    /// configuration asking for it in a build without it is told plainly rather than having its
    /// setting silently ignored.
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
    /// Where its weights are, for the backends that have any.
    ///
    /// `None` leaves the caller to decide, because this crate is a leaf and does not know where
    /// a project keeps things.
    pub path: Option<String>,
}

impl Default for Spec {
    fn default() -> Self {
        Self {
            kind: Kind::Hashed,
            model: "hashed-3gram-256".to_owned(),
            path: None,
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
            path: said.path.clone(),
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
        /// Where its weights are.
        pub path: Option<String>,
    }
}

/// Open the embedder a spec asks for, or the nearest one that works.
///
/// `None` means nothing is embedding and recall is lexical, which is a supported state rather
/// than a failure. Anything else answers with a working embedder whose [`Embed::model`] names
/// what was *actually* opened — which is how a caller reports a fallback instead of hiding it.
#[must_use]
pub fn open(spec: &Spec) -> Option<Box<dyn Embed>> {
    open_explaining(spec).0
}

/// The same, and why, when the answer is not what was asked for.
///
/// Split out because "you asked for a transformer and got the hashing trick" is something a
/// person needs to be able to read. `memo status` prints it; the plain [`open`] is for callers
/// that only need a vector.
#[must_use]
pub fn open_explaining(spec: &Spec) -> (Option<Box<dyn Embed>>, Option<String>) {
    match spec.kind {
        Kind::None => (None, None),
        Kind::Hashed => (Some(Box::new(Hashed)), None),
        Kind::Onnx => dense(spec),
    }
}

/// The transformer, in a build that has one.
#[cfg(feature = "dense")]
fn dense(spec: &Spec) -> (Option<Box<dyn Embed>>, Option<String>) {
    let Some(path) = spec.path.as_deref() else {
        return (
            Some(Box::new(Hashed)),
            Some(format!(
                "'{}' needs a path to its weights — set `path` on the embedder",
                spec.model
            )),
        );
    };
    match crate::Dense::open(std::path::Path::new(path), &spec.model) {
        Ok(model) => (Some(Box::new(model)), None),
        // Still a working store, still a worse one, and it says so. Falling all the way to no
        // vectors would be a bigger change than the caller asked for; falling silently would be
        // worse than either.
        Err(why) => (Some(Box::new(Hashed)), Some(why.to_string())),
    }
}

/// The transformer, in a build without one.
#[cfg(not(feature = "dense"))]
fn dense(spec: &Spec) -> (Option<Box<dyn Embed>>, Option<String>) {
    (
        Some(Box::new(Hashed)),
        Some(format!(
            "'{}' needs a build made with --features dense; using the local embedder",
            spec.model
        )),
    )
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
            ..serde_json_lite::Value::default()
        }));
        assert!(open(&spec).is_none(), "lexical alone is a supported state");
    }

    #[test]
    fn an_unrecognised_backend_falls_back_rather_than_failing() {
        let spec = Spec::read(Some(&serde_json_lite::Value {
            kind: Some("something-else".into()),
            ..serde_json_lite::Value::default()
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

    #[test]
    fn asking_for_a_transformer_is_answered_rather_than_ignored() {
        // The bug this replaces: `kind = "onnx"` used to get the hashing trick with nobody
        // told. A person who configured a transformer and got surface similarity had no way to
        // find out, and every explanation of their search results was wrong.
        let spec = Spec::read(Some(&serde_json_lite::Value {
            kind: Some("onnx".into()),
            ..serde_json_lite::Value::default()
        }));
        assert_eq!(spec.kind, Kind::Onnx);

        let (opened, why) = open_explaining(&spec);
        assert!(opened.is_some(), "there is still a working embedder");
        let why = why.expect("and a reason it is not the one that was asked for");
        assert!(
            why.contains("dense") || why.contains("path"),
            "the reason says what to do about it: {why}"
        );
    }

    #[test]
    fn a_transformer_that_loads_is_the_one_that_is_used() {
        // Only meaningful in a build that carries one, and only when somebody has installed
        // weights — `MEMO_MODEL_DIR` is how the suite is pointed at them.
        #[cfg(feature = "dense")]
        {
            let Ok(dir) = std::env::var("MEMO_MODEL_DIR") else {
                return;
            };
            let spec = Spec::read(Some(&serde_json_lite::Value {
                kind: Some("onnx".into()),
                model: Some("bge-small-en-v1.5".into()),
                path: Some(dir),
            }));
            let (opened, why) = open_explaining(&spec);
            assert_eq!(why, None, "it loaded, so there is nothing to explain");
            let opened = opened.expect("an embedder");
            assert_eq!(opened.model(), "bge-small-en-v1.5");
            assert_eq!(opened.dimensions(), 384);
        }
    }
}
