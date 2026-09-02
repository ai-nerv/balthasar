//! Fitting the ranking policy.
//!
//! Gradient descent on a logistic regression: about forty lines of arithmetic, no dependency,
//! no network. The interesting parts are not the fitting — they are the four things that stop a
//! fitted model from being believed when it should not be.
//!
//! **A holdout, always.** Training accuracy is not a result. The data is split before anything
//! is fitted, and the number reported is from rows the model never saw.
//!
//! **AUC, not accuracy.** Most candidates are not helpful, so a model predicting "no" for
//! everything scores well on accuracy and is worthless. AUC asks whether it *ranks* a helpful
//! candidate above an unhelpful one, which is the actual job.
//!
//! **A floor on the data.** Twelve weights from forty rows describe the sample.
//!
//! **A comparison against the rules.** The existing score is already a good ranker. A model that
//! does not beat it has not earned a place, and reporting its AUC without the baseline's beside
//! it would make a tie look like a win.

use memo_recall::{FEATURES, LAYOUT, MINIMUM, Model};
use memo_store::TrainingRow;

/// One labelled example.
#[derive(Debug, Clone, PartialEq)]
pub struct Example {
    /// The features, in [`FEATURES`] order.
    pub features: Vec<f64>,
    /// Whether it turned out to help.
    pub helpful: bool,
    /// What the rules already scored it, for the baseline comparison.
    pub rule_score: f64,
}

/// What fitting produced, and whether to believe it.
#[derive(Debug, Clone, PartialEq)]
pub struct Fitted {
    /// The model.
    pub model: Model,
    /// How well the existing score ranks the same held-out rows.
    pub baseline_auc: f64,
    /// How many examples were usable.
    pub examples: usize,
    /// How many of those were positive.
    pub helpful: usize,
}

impl Fitted {
    /// Whether the model beat the rules on held-out data.
    ///
    /// Beating them by anything is not enough — a hundredth of an AUC point is noise, and a
    /// model adopted on noise is a model that will be un-adopted on noise later.
    #[must_use]
    pub fn beats_the_rules(&self) -> bool {
        self.model.holdout_auc > self.baseline_auc + 0.02
    }
}

/// What went wrong.
///
/// Both of these are refusals rather than failures: there is nothing wrong with the data, there
/// is simply not enough of it to fit twelve weights to and say anything honest afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrainError {
    /// Not enough labelled rows to fit twelve weights to.
    TooLittle(usize),
    /// Every row has the same label, so there is nothing to separate.
    OneSided(&'static str),
}

impl std::fmt::Display for TrainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLittle(n) => write!(
                f,
                "only {n} labelled example(s); {MINIMUM} is the floor for {} weights",
                FEATURES.len()
            ),
            Self::OneSided(side) => write!(
                f,
                "every example is {side} — there is nothing to learn from one side of a question"
            ),
        }
    }
}

impl std::error::Error for TrainError {}

/// Turn exported rows into labelled examples.
///
/// Only rows with a countable outcome. `ignored` and `unknown` are not labels — an action nobody
/// evaluated is not a failure, and training on that assumption would teach the model that
/// anything unreported is bad, which is most of everything.
#[must_use]
pub fn label(rows: &[TrainingRow]) -> Vec<Example> {
    rows.iter()
        .filter_map(|row| {
            let helpful = match row.outcome.as_deref()? {
                "succeeded" => true,
                "failed" | "corrected" | "reverted" => false,
                // ignored, abstained, unknown: not evidence either way.
                _ => return None,
            };
            Some(Example {
                features: features_of(row),
                helpful,
                rule_score: row.score,
            })
        })
        .collect()
}

/// A row's features, in the order the weights expect.
#[must_use]
pub fn features_of(row: &TrainingRow) -> Vec<f64> {
    vec![
        row.rank as f64,
        row.score,
        row.semantic,
        row.lexical,
        row.entity,
        row.frecency,
        row.confidence,
        row.strength,
        row.scope_signal,
        f64::from(u8::from(row.vectors)),
        row.witnesses as f64,
        row.domains as f64,
    ]
}

/// Fit a policy, and say whether it is worth anything.
///
/// `holdout` is the share kept back. The split is deterministic — every third row, rather than
/// shuffled — so two runs on the same data produce the same model and the same number, which is
/// the only way a training result can be compared with a later one.
pub fn fit(examples: &[Example], holdout: f64, passes: usize) -> Result<Fitted, TrainError> {
    if examples.len() < MINIMUM {
        return Err(TrainError::TooLittle(examples.len()));
    }
    let helpful = examples.iter().filter(|e| e.helpful).count();
    if helpful == 0 {
        return Err(TrainError::OneSided("unhelpful"));
    }
    if helpful == examples.len() {
        return Err(TrainError::OneSided("helpful"));
    }

    let every = ((1.0 / holdout.clamp(0.1, 0.5)).round() as usize).max(2);
    let mut train: Vec<&Example> = Vec::with_capacity(examples.len());
    let mut test: Vec<&Example> = Vec::new();
    for (at, example) in examples.iter().enumerate() {
        if at % every == 0 {
            test.push(example);
        } else {
            train.push(example);
        }
    }

    // Standardised on the training half only. Using the whole set would leak the holdout's
    // distribution into the model and flatter every number that follows.
    let width = FEATURES.len();
    let mut mean = vec![0.0; width];
    let mut scale = vec![1.0; width];
    for i in 0..width {
        let column: Vec<f64> = train.iter().map(|e| e.features[i]).collect();
        let m = column.iter().sum::<f64>() / column.len() as f64;
        let variance = column.iter().map(|x| (x - m).powi(2)).sum::<f64>() / column.len() as f64;
        mean[i] = m;
        // A constant feature gets a scale of one rather than dividing by zero. It contributes
        // nothing, which is correct: it distinguishes nothing.
        scale[i] = if variance > 1e-12 {
            variance.sqrt()
        } else {
            1.0
        };
    }

    let standardise = |e: &Example| -> Vec<f64> {
        e.features
            .iter()
            .zip(&mean)
            .zip(&scale)
            .map(|((x, m), s)| (x - m) / s)
            .collect()
    };

    let mut weights = vec![0.0; width];
    let mut bias = 0.0;
    let rate = 0.1;
    for _ in 0..passes {
        let mut gradient = vec![0.0; width];
        let mut bias_gradient = 0.0;
        for example in &train {
            let x = standardise(example);
            let z: f64 = x.iter().zip(&weights).map(|(a, b)| a * b).sum::<f64>() + bias;
            let p = 1.0 / (1.0 + (-z).exp());
            let error = p - f64::from(u8::from(example.helpful));
            for (g, xi) in gradient.iter_mut().zip(&x) {
                *g += error * xi;
            }
            bias_gradient += error;
        }
        let n = train.len() as f64;
        for (w, g) in weights.iter_mut().zip(&gradient) {
            *w -= rate * g / n;
        }
        bias -= rate * bias_gradient / n;
    }

    let model = Model {
        weights,
        bias,
        mean,
        scale,
        trained_on: train.len(),
        holdout_auc: 0.0,
        layout: LAYOUT,
    };

    // Both numbers from the same held-out rows, so the comparison is like for like.
    let scored: Vec<(f64, bool)> = test
        .iter()
        .map(|e| (model.predict(&e.features), e.helpful))
        .collect();
    let by_rules: Vec<(f64, bool)> = test.iter().map(|e| (e.rule_score, e.helpful)).collect();

    Ok(Fitted {
        model: Model {
            holdout_auc: auc(&scored),
            ..model
        },
        baseline_auc: auc(&by_rules),
        examples: examples.len(),
        helpful,
    })
}

/// The probability that a helpful example outranks an unhelpful one.
///
/// Accuracy is the wrong measure here: most candidates are not helpful, so predicting "no" for
/// everything scores well and ranks nothing. This asks the question the model is actually for.
///
/// Computed by counting concordant pairs, which is exact and quadratic — fine at the sizes a
/// local ledger reaches, and honest about what it is doing.
#[must_use]
pub fn auc(scored: &[(f64, bool)]) -> f64 {
    let good: Vec<f64> = scored.iter().filter(|(_, y)| *y).map(|(p, _)| *p).collect();
    let bad: Vec<f64> = scored
        .iter()
        .filter(|(_, y)| !*y)
        .map(|(p, _)| *p)
        .collect();
    if good.is_empty() || bad.is_empty() {
        return 0.5;
    }
    let mut wins = 0.0;
    for g in &good {
        for b in &bad {
            wins += match g.partial_cmp(b) {
                Some(std::cmp::Ordering::Greater) => 1.0,
                Some(std::cmp::Ordering::Equal) => 0.5,
                _ => 0.0,
            };
        }
    }
    wins / (good.len() * bad.len()) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Examples where `confidence` genuinely predicts helpfulness and nothing else does.
    fn learnable(n: usize) -> Vec<Example> {
        (0..n)
            .map(|i| {
                let confident = i % 2 == 0;
                let mut features = vec![0.0; FEATURES.len()];
                features[6] = if confident { 0.9 } else { 0.2 }; // confidence
                features[5] = ((i % 7) as f64) / 7.0; // frecency: noise
                Example {
                    features,
                    helpful: confident,
                    rule_score: 0.5,
                }
            })
            .collect()
    }

    #[test]
    fn it_finds_the_signal_that_is_there() {
        let held = fit(&learnable(600), 0.3, 400).expect("fit");
        assert!(
            held.model.holdout_auc > 0.9,
            "held-out AUC {:.3}",
            held.model.holdout_auc
        );
        assert_eq!(held.model.explain()[0].0, "confidence");
    }

    #[test]
    fn it_beats_a_baseline_that_knows_nothing() {
        // Every rule_score is 0.5 here, so the rules rank at chance. A model that could not
        // beat that on separable data would be broken.
        let held = fit(&learnable(600), 0.3, 400).expect("fit");
        assert!((held.baseline_auc - 0.5).abs() < 0.01);
        assert!(held.beats_the_rules());
    }

    #[test]
    fn a_hair_better_than_the_rules_is_not_better() {
        // A model adopted on noise is one that will be un-adopted on noise later.
        let mut held = fit(&learnable(600), 0.3, 400).expect("fit");
        held.baseline_auc = held.model.holdout_auc - 0.01;
        assert!(!held.beats_the_rules());
    }

    #[test]
    fn too_little_data_is_refused_rather_than_fitted() {
        let held = fit(&learnable(40), 0.3, 100);
        assert!(matches!(held, Err(TrainError::TooLittle(40))));
    }

    #[test]
    fn one_sided_data_is_refused() {
        // Nothing to separate. A model fitted here would predict one class perfectly and mean
        // nothing at all.
        let all_good: Vec<Example> = learnable(400)
            .into_iter()
            .map(|mut e| {
                e.helpful = true;
                e
            })
            .collect();
        assert!(matches!(
            fit(&all_good, 0.3, 100),
            Err(TrainError::OneSided("helpful"))
        ));
    }

    #[test]
    fn the_split_is_the_same_every_time() {
        // Two runs on the same data must produce the same model and the same number, or no
        // training result can be compared with a later one.
        let data = learnable(600);
        let one = fit(&data, 0.3, 200).expect("fit");
        let two = fit(&data, 0.3, 200).expect("fit");
        assert_eq!(one, two);
    }

    #[test]
    fn the_holdout_is_never_trained_on() {
        // Standardisation included. Using the whole set would leak the holdout's distribution
        // into the model and flatter every number after it.
        let data = learnable(600);
        let held = fit(&data, 0.3, 200).expect("fit");
        assert!(held.model.trained_on < data.len(), "some was kept back");
        assert!(held.model.trained_on > data.len() / 2, "and most was used");
    }

    #[test]
    fn unreported_outcomes_are_not_labels() {
        // Training on "unreported means bad" would teach the model that most of everything is
        // bad, which is a statement about instrumentation.
        let rows = vec![
            row(Some("succeeded")),
            row(Some("failed")),
            row(Some("ignored")),
            row(Some("unknown")),
            row(None),
        ];
        let held = label(&rows);
        assert_eq!(held.len(), 2, "only the countable ones");
        assert!(held[0].helpful);
        assert!(!held[1].helpful);
    }

    #[test]
    fn auc_of_a_perfect_ranker_is_one() {
        assert!((auc(&[(0.9, true), (0.8, true), (0.2, false)]) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn auc_of_a_backwards_ranker_is_zero() {
        assert!((auc(&[(0.1, true), (0.9, false)]) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn auc_with_one_class_is_a_coin() {
        // Nothing to rank against. Reporting anything else would invent a result.
        assert!((auc(&[(0.9, true), (0.8, true)]) - 0.5).abs() < 1e-9);
        assert!((auc(&[]) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_model_that_predicts_one_class_scores_badly_on_auc_and_well_on_accuracy() {
        // Why AUC and not accuracy. Nineteen unhelpful, one helpful, and a model that says "no"
        // to everything: 95% accurate, and useless.
        let mut scored: Vec<(f64, bool)> = (0..19).map(|_| (0.1, false)).collect();
        scored.push((0.1, true));
        assert!((auc(&scored) - 0.5).abs() < 1e-9, "AUC sees through it");
    }

    fn row(outcome: Option<&str>) -> TrainingRow {
        TrainingRow {
            recall: "r1".to_owned(),
            query_hash: "h".to_owned(),
            config: "c".to_owned(),
            rank: 0,
            selected: true,
            score: 0.5,
            semantic: 0.0,
            lexical: 0.5,
            entity: 0.0,
            frecency: 0.0,
            confidence: 1.0,
            strength: 1.0,
            scope_signal: 1.0,
            vectors: false,
            presentation: None,
            outcome: outcome.map(str::to_owned),
            attribution: Some("explicit".to_owned()),
            witnesses: 1,
            domains: 1,
            at: 0,
        }
    }
}
