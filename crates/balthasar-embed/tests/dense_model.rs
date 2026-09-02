//! The transformer, against real weights.
//!
//! Skipped unless `MAGI_BALTHASAR_MODEL_DIR` names a directory holding a sentence transformer, because a
//! 127 MB download is not something a test suite may assume. Point it at one and this becomes
//! the evidence that the dense path works, and that it is better at the job it was added for.
#![cfg(feature = "dense")]

use balthasar_embed::{Dense, Embed, Hashed};

fn model() -> Option<Dense> {
    let dir = std::env::var("MAGI_BALTHASAR_MODEL_DIR").ok()?;
    Some(Dense::open(std::path::Path::new(&dir), "bge-small-en-v1.5").expect("the model loads"))
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn vectors(model: &Dense, texts: &[&str]) -> Vec<Vec<f32>> {
    model
        .embed(&texts.iter().map(|t| (*t).to_owned()).collect::<Vec<_>>())
        .expect("embed")
}

#[test]
fn it_produces_unit_vectors_of_the_declared_width() {
    let Some(model) = model() else { return };
    assert_eq!(model.dimensions(), 384);

    let out = vectors(&model, &["we deploy with fly.io"]);
    assert_eq!(out[0].len(), 384);
    let length: f32 = out[0].iter().map(|v| v * v).sum::<f32>().sqrt();
    assert!((length - 1.0).abs() < 1e-4, "not normalised: {length}");
}

#[test]
fn batching_does_not_change_what_a_claim_embeds_to() {
    // Padding is masked, so a short claim scores the same alone as it does beside a long one.
    // Without the mask an embedding would depend on what happened to be ingested with it.
    let Some(model) = model() else { return };

    let alone = vectors(&model, &["make test"]);
    let crowded = vectors(
        &model,
        &[
            "make test",
            "a much longer sentence about something else entirely, at length",
        ],
    );
    let moved = cosine(&alone[0], &crowded[0]);
    assert!(moved > 0.999, "batching moved it: {moved:.4}");
}

#[test]
fn it_finds_meaning_where_the_hashed_embedder_finds_spelling() {
    // The reason to carry 127 MB. These two say the same thing and share almost no characters,
    // so surface similarity has nothing to go on and a transformer does.
    let Some(model) = model() else { return };

    let pair = ["how do I run the tests?", "the suite is invoked with make"];
    let dense = vectors(&model, &pair);
    let dense_score = cosine(&dense[0], &dense[1]);

    let coarse = Hashed
        .embed(&pair.iter().map(|t| (*t).to_owned()).collect::<Vec<_>>())
        .expect("hashed");
    let coarse_score = cosine(&coarse[0], &coarse[1]);

    println!("\n  dense  {dense_score:.3}\n  hashed {coarse_score:.3}\n");
    assert!(
        dense_score > coarse_score + 0.2,
        "the transformer has to earn its size: dense {dense_score:.3}, hashed {coarse_score:.3}"
    );
}

#[test]
fn it_is_still_the_wrong_tool_for_deciding_two_claims_agree() {
    // Kept as a test because it is the trap. A dense model puts a claim and its own replacement
    // very close together — closer, often, than two true rewordings — so corroboration must go
    // on using content words no matter how good the embedder gets.
    let Some(model) = model() else { return };

    // Apples to apples: a genuine rewording beside a genuine contradiction. If the second is
    // not clearly lower than the first, no threshold on this signal can tell them apart.
    let out = vectors(
        &model,
        &[
            "we use make test",
            "run make test instead",
            "we deploy with fly.io",
            "we deploy with heroku",
        ],
    );
    let rewording = cosine(&out[0], &out[1]);
    let contradiction = cosine(&out[2], &out[3]);

    println!("\n  a true rewording            {rewording:.3}");
    println!("  a claim vs its replacement  {contradiction:.3}\n");
    assert!(
        contradiction >= rewording - 0.1,
        "a contradiction has to stay about as close as a rewording for this to be the trap it \
         is: rewording {rewording:.3}, contradiction {contradiction:.3}"
    );
}
