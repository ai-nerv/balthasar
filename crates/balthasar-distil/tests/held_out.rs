//! F2 and F6 held out: wording a person wrote after the freeze, scored against the rules as
//! they stood. Run by `oslo make test-held-out`; see `held-out/README.md`.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_distil::{Observation, Role, decisions, read_instruction};
use balthasar_model::ScopeId;
use balthasar_store::Store;
use std::path::PathBuf;

/// One case: its label, and its fields after the label.
type Case = (String, Vec<String>);

/// The cases in `name`, or a failure saying the set has not been written.
fn cases(name: &str) -> Vec<Case> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/held-out")
        .join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "no held-out set at {}: a person writes it, see held-out/README.md",
            path.display()
        )
    });
    let held: Vec<Case> = text
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|line| {
            let mut fields = line.split('\t').map(str::to_owned);
            let label = fields.next().unwrap_or_default();
            (label, fields.collect())
        })
        .collect();
    assert!(!held.is_empty(), "{} holds no cases", path.display());
    held
}

fn share(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        1.0
    } else {
        part as f64 / whole as f64
    }
}

#[test]
#[ignore = "the held-out set, run by `oslo make test-held-out`"]
fn polarity_held_out() {
    let held = cases("polarity.tsv");
    let (mut kept, mut lost, mut missed, mut taken, mut plain) = (0, Vec::new(), 0, Vec::new(), 0);
    for (label, fields) in &held {
        match (label.as_str(), fields.as_slice()) {
            ("keep", [word, said]) => match read_instruction(said, &[]).and_then(|r| r.claim) {
                Some(claim) if claim.to_lowercase().contains(&word.to_lowercase()) => kept += 1,
                Some(claim) => lost.push(format!("{said} -> {claim}")),
                None => missed += 1,
            },
            ("not", [said]) => {
                plain += 1;
                if read_instruction(said, &[]).is_some() {
                    taken.push(said.clone());
                }
            }
            _ => panic!("not a polarity case: {label} {fields:?}"),
        }
    }
    let precision = 1.0 - share(taken.len(), plain);
    println!(
        "F2 held out: precision {precision:.3} on {plain}, recall {:.3} of {} directives",
        share(kept, kept + lost.len() + missed),
        kept + lost.len() + missed
    );
    assert!(lost.is_empty(), "a negation was lost: {lost:?}");
    assert!(precision >= 0.95, "taken as instructions: {taken:?}");
}

#[test]
#[ignore = "the held-out set, run by `oslo make test-held-out`"]
fn decisions_held_out() {
    let store = Store::ephemeral().expect("store");
    let scope = ScopeId::new("/w/thing");
    let found = |said: &str| {
        let turn = Observation {
            role: Role::User,
            text: said.to_owned(),
            ..Observation::default()
        };
        !decisions(&store, &scope, &[turn], 1_756_000_000, &[]).is_empty()
    };
    let (mut caught, mut decided, mut taken, mut plain) = (0, 0, Vec::new(), 0);
    for (label, fields) in cases("decision.tsv") {
        match (label.as_str(), fields.as_slice()) {
            ("decision", [said]) => {
                decided += 1;
                caught += usize::from(found(said));
            }
            ("not", [said]) => {
                plain += 1;
                if found(said) {
                    taken.push(said.clone());
                }
            }
            _ => panic!("not a decision case: {label} {fields:?}"),
        }
    }
    let precision = 1.0 - share(taken.len(), plain);
    println!(
        "F6 held out: precision {precision:.3} on {plain}, recall {:.3} of {decided} decisions",
        share(caught, decided)
    );
    assert!(precision >= 0.9, "taken as decisions: {taken:?}");
}
