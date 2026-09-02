//! A learned ranking policy, small enough to read.
//!
//! Twelve weights and a bias. Not a language model, not a neural network, not something that
//! needs a key or a GPU — a logistic regression over the features the ledger already records,
//! which comes to about a hundred bytes and predicts in a dot product.
//!
//! **Readability is why it is this and not something better.** Gradient-boosted trees would
//! score higher on tabular data and would give an answer nobody can argue with. A system whose
//! entire thesis is that it can show its reasoning cannot then rank by an oracle. These weights
//! print as sentences: *confidence counted for a lot, frecency counted for almost nothing*.
//!
//! **It proposes and nothing more.** The output is a probability used to reorder candidates.
//! There is no path from here to asserting, to confidence, or to what may be injected — those
//! are constraints, and a model that could move them would be infrastructure rather than an
//! experiment.

use std::path::Path;

/// The features, in the order the weights expect.
///
/// Fixed and named, because a model whose inputs shifted underneath it would keep predicting
/// confidently from the wrong columns. [`LAYOUT`] moves if this list does, and a model fitted
/// to an older one is refused.
pub const FEATURES: &[&str] = &[
    "rank",
    "score",
    "semantic",
    "lexical",
    "entity",
    "frecency",
    "confidence",
    "strength",
    "scope_signal",
    "vectors",
    "witnesses",
    "domains",
];

/// Which arrangement of features these weights were fitted to.
pub const LAYOUT: u32 = 1;

/// A fitted policy.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Model {
    /// One per feature, in [`FEATURES`] order.
    pub weights: Vec<f64>,
    /// The intercept.
    pub bias: f64,
    /// Each feature's mean in training, for standardising an input the same way.
    pub mean: Vec<f64>,
    /// Each feature's spread. Never zero — a constant feature gets one.
    pub scale: Vec<f64>,
    /// How many labelled examples it saw.
    pub trained_on: usize,
    /// How well it separated held-out data. Below 0.5 is worse than a coin.
    pub holdout_auc: f64,
    /// Which feature layout.
    pub layout: u32,
}

impl Model {
    /// What this thinks of one candidate, between zero and one.
    ///
    /// A malformed input answers 0.5 rather than panicking or guessing. A caller that got the
    /// feature count wrong should get "no opinion", because the alternative is a confident
    /// number derived from whatever happened to be in the vector.
    #[must_use]
    pub fn predict(&self, features: &[f64]) -> f64 {
        if features.len() != self.weights.len() || features.len() != self.mean.len() {
            return 0.5;
        }
        let z: f64 = features
            .iter()
            .zip(&self.mean)
            .zip(&self.scale)
            .zip(&self.weights)
            .map(|(((x, m), s), w)| ((x - m) / s) * w)
            .sum::<f64>()
            + self.bias;
        1.0 / (1.0 + (-z).exp())
    }

    /// What it learned, strongest first.
    ///
    /// The whole reason for choosing this shape. A weight of +2.3 on `confidence` and −0.1 on
    /// `frecency` is a readable claim about retrieval that somebody can disagree with.
    #[must_use]
    pub fn explain(&self) -> Vec<(&'static str, f64)> {
        let mut held: Vec<(&'static str, f64)> = FEATURES
            .iter()
            .copied()
            .zip(self.weights.iter().copied())
            .collect();
        held.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()));
        held
    }

    /// Whether this is worth listening to at all.
    ///
    /// Three ways to fail, and each is a real thing that happens: the feature layout moved, it
    /// was fitted on too little to mean anything, or it does not separate held-out data better
    /// than chance. Any of them and the deterministic policy is used instead — which is not a
    /// fallback so much as the normal case with an experiment declining to interfere.
    #[must_use]
    pub fn is_useful(&self) -> bool {
        self.layout == LAYOUT && self.trained_on >= MINIMUM && self.holdout_auc > 0.55
    }

    /// Read a model from disk, if there is one worth having.
    ///
    /// Every failure is `None`: missing, unreadable, wrong layout, not good enough. A caller
    /// gets the rules and never has to handle an error, because a policy that could fail loudly
    /// would be a policy the turn path depends on.
    #[must_use]
    pub fn load(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        let held: Self = serde_json::from_str(&text).ok()?;
        held.is_useful().then_some(held)
    }

    /// Write it out.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, text)
    }
}

/// The fewest labelled examples worth fitting twelve weights to.
///
/// Two hundred. Below that the weights are describing the sample rather than retrieval, and a
/// model that confidently reorders results on the strength of forty observations is worse than
/// none — it is the rules plus noise, wearing the authority of having been trained.
pub const MINIMUM: usize = 200;

#[cfg(test)]
mod tests {
    use super::*;

    fn a_model() -> Model {
        Model {
            weights: vec![0.0; FEATURES.len()],
            bias: 0.0,
            mean: vec![0.0; FEATURES.len()],
            scale: vec![1.0; FEATURES.len()],
            trained_on: 1_000,
            holdout_auc: 0.72,
            layout: LAYOUT,
        }
    }

    #[test]
    fn a_model_of_all_zeros_has_no_opinion() {
        assert!((a_model().predict(&[1.0; 12]) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_weight_moves_the_answer_in_its_own_direction() {
        let mut held = a_model();
        held.weights[6] = 2.0; // confidence
        let mut low = vec![0.0; 12];
        let mut high = vec![0.0; 12];
        low[6] = -1.0;
        high[6] = 1.0;
        assert!(held.predict(&high) > held.predict(&low));
    }

    #[test]
    fn a_wrong_shaped_input_gets_no_opinion_rather_than_a_guess() {
        // A confident number derived from whatever happened to be in the vector is the worst
        // possible answer here.
        assert!((a_model().predict(&[1.0, 2.0]) - 0.5).abs() < 1e-9);
        assert!((a_model().predict(&[]) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn what_it_learned_reads_as_sentences() {
        // The reason this shape was chosen over something that scores better.
        let mut held = a_model();
        held.weights[6] = 2.31; // confidence
        held.weights[5] = -0.18; // frecency
        let said = held.explain();
        assert_eq!(said[0].0, "confidence", "the strongest is first");
        assert!(said[0].1 > 0.0);
        assert!(said.iter().any(|(name, w)| *name == "frecency" && *w < 0.0));
        assert_eq!(said.len(), FEATURES.len());
    }

    #[test]
    fn a_model_fitted_on_too_little_is_refused() {
        // Twelve weights from forty observations is the rules plus noise, wearing the authority
        // of having been trained.
        let mut held = a_model();
        held.trained_on = 40;
        assert!(!held.is_useful());
    }

    #[test]
    fn a_model_no_better_than_a_coin_is_refused() {
        let mut held = a_model();
        held.holdout_auc = 0.5;
        assert!(!held.is_useful());
    }

    #[test]
    fn a_model_from_a_different_feature_layout_is_refused() {
        // Otherwise it keeps predicting confidently from the wrong columns.
        let mut held = a_model();
        held.layout = LAYOUT + 1;
        assert!(!held.is_useful());
    }

    #[test]
    fn every_way_of_having_no_model_is_the_same_way() {
        // A caller gets `None` and uses the rules. There is no error to handle, because a policy
        // the turn path had to handle errors from would be one the turn path depends on.
        let missing = std::env::temp_dir().join("balthasar-no-such-model-9f3a.json");
        let _ = std::fs::remove_file(&missing);
        assert!(Model::load(&missing).is_none());

        let junk = std::env::temp_dir().join("balthasar-junk-model.json");
        std::fs::write(&junk, "not json at all").expect("write");
        assert!(Model::load(&junk).is_none());

        let weak = std::env::temp_dir().join("balthasar-weak-model.json");
        let mut held = a_model();
        held.holdout_auc = 0.4;
        held.save(&weak).expect("save");
        assert!(
            Model::load(&weak).is_none(),
            "a bad model is the same as no model"
        );

        let _ = std::fs::remove_file(&junk);
        let _ = std::fs::remove_file(&weak);
    }

    #[test]
    fn a_good_model_survives_a_round_trip() {
        let at = std::env::temp_dir().join("balthasar-good-model.json");
        let held = a_model();
        held.save(&at).expect("save");
        assert_eq!(Model::load(&at).expect("load"), held);
        let _ = std::fs::remove_file(&at);
    }

    #[test]
    fn the_whole_model_is_smaller_than_a_page() {
        // Twelve weights, a bias, and the standardisation. It ships in the repository.
        let text = serde_json::to_string(&a_model()).expect("serialize");
        assert!(text.len() < 1024, "{} bytes", text.len());
    }
}
