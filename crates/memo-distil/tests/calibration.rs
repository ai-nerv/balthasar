//! The corpus the near-match threshold was chosen from, held open.
//!
//! `akin::SAME_CLAIM` is a number, and a number picked once and never checked again is a number
//! that drifts. This is the evidence for it: two populations of claim pairs, and the assertion
//! that they do not touch. A change to the stemmer, the grammar list or the threshold that
//! closes the gap fails here rather than in somebody's project six months later.
//!
//! Run with `--nocapture` to see the numbers rather than only the verdict.

use memo_distil::{claim_overlap, same_claim};

/// Pairs that are one claim worded two ways. All must merge.
const SAME: &[(&str, &str)] = &[
    ("we use make test", "run make test instead"),
    ("this project uses make", "the project uses make"),
    ("we deploy with fly.io", "deploy target is fly.io"),
    (
        "always run make fmt before committing",
        "run make fmt before you commit",
    ),
    (
        "the tests are run with make test",
        "make test runs the tests",
    ),
    ("use make test", "use `make test`"),
    ("the linter is run with make lint", "run make lint to lint"),
    (
        "migrations live in db/migrate",
        "the migrations are in db/migrate",
    ),
    (
        "never commit to main directly",
        "do not commit directly to main",
    ),
    (
        "the api key is in .env.local",
        "we keep the api key in .env.local",
    ),
];

/// Pairs that are different claims, several of them a claim beside its own replacement.
/// None may merge, and the ones that are revisions are the ones that would do real harm.
const DIFFERENT: &[(&str, &str)] = &[
    ("we use make test", "we use cargo build"),
    (
        "the staging box is at 10.0.0.7",
        "the production box is at 10.0.0.8",
    ),
    ("we deploy with fly.io", "we deploy with heroku"),
    (
        "run make fmt before committing",
        "run make lint after committing",
    ),
    ("the database is postgres", "the cache is redis"),
    (
        "the api key is in .env.local",
        "the api key is in .env.production",
    ),
    (
        "migrations live in db/migrate",
        "fixtures live in db/fixtures",
    ),
    (
        "never commit to main directly",
        "never commit to develop directly",
    ),
    (
        "run make test before pushing",
        "run make test after pulling",
    ),
    (
        "the staging url is app.staging.example.com",
        "the prod url is app.example.com",
    ),
];

#[test]
fn every_rewording_is_recognised_as_one_claim() {
    for (a, b) in SAME {
        assert!(
            same_claim(a, b),
            "{a:?} and {b:?} are one claim, scored {:.3}",
            claim_overlap(a, b)
        );
    }
}

#[test]
fn no_different_claim_is_taken_for_a_rewording() {
    // The direction that matters. A missed merge delays one promotion; a false one puts a claim
    // in a project's memory that no run ever made.
    for (a, b) in DIFFERENT {
        assert!(
            !same_claim(a, b),
            "{a:?} and {b:?} are different claims, scored {:.3}",
            claim_overlap(a, b)
        );
    }
}

#[test]
fn the_two_populations_do_not_touch() {
    // Not just "the threshold works" but "there is room around it". A gap that has narrowed to
    // nothing still passes the two tests above and is one new phrasing away from not.
    let worst_rewording = SAME
        .iter()
        .map(|(a, b)| claim_overlap(a, b))
        .fold(f32::MAX, f32::min);
    let best_impostor = DIFFERENT
        .iter()
        .map(|(a, b)| claim_overlap(a, b))
        .fold(f32::MIN, f32::max);

    println!("\n  rewordings score no lower than  {worst_rewording:.3}");
    println!("  different claims no higher than {best_impostor:.3}\n");

    assert!(
        worst_rewording > best_impostor,
        "the populations overlap: a rewording scored {worst_rewording:.3} and a different \
         claim scored {best_impostor:.3}, so no threshold separates them"
    );
    assert!(
        worst_rewording - best_impostor >= 0.1,
        "the gap has narrowed to {:.3}, which is too little to pick a threshold inside",
        worst_rewording - best_impostor
    );
}

#[test]
fn the_embedder_is_not_used_for_this_and_here_is_why() {
    // Kept as a test rather than a comment because it is the reason for a design decision, and
    // the obvious future change is "why not just use the vectors we already compute".
    //
    // On the local hashed embedder, a claim and its own contradiction score higher than a true
    // rewording. That is not a threshold that needs tuning; it is a signal measuring how a
    // sentence is spelled rather than what it says.
    use memo_embed::{Embed, Hashed};

    let cosine = |a: &str, b: &str| -> f32 {
        let v = Hashed
            .embed(&[memo_model::normalised(a), memo_model::normalised(b)])
            .expect("embed");
        v[0].iter().zip(&v[1]).map(|(x, y)| x * y).sum()
    };

    let rewording = cosine("we use make test", "run make test instead");
    let contradiction = cosine("we deploy with fly.io", "we deploy with heroku");
    let other_box = cosine(
        "the staging box is at 10.0.0.7",
        "the production box is at 10.0.0.8",
    );

    println!("\n  rewording      {rewording:.3}");
    println!("  contradiction  {contradiction:.3}");
    println!("  a different box {other_box:.3}\n");

    assert!(
        contradiction > rewording && other_box > rewording,
        "if this ever stops being true the embedder is worth reconsidering: rewording \
         {rewording:.3}, contradiction {contradiction:.3}, different box {other_box:.3}"
    );
}
