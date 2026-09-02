//! Folding clusters that say the same thing into one another.
//!
//! The judgement itself is [`memo_model::same_claim`] — whether two claims are one claim is a
//! question about claims, not about clustering, and the store needs the same answer on the
//! write path. This is what applies it to a pass over a project's scratch.

use memo_store::Cluster;

/// How many clusters one pass will compare against each other.
///
/// The comparison is quadratic, so this is a wall-clock budget rather than a correctness one —
/// newest first, and anything missed is found by the next pass, exactly as with the run cap.
const COMPARED: usize = 300;

/// How many clusters one pass will compare against each other.
///
/// A cluster, and whether it took a near match to assemble.
#[derive(Debug, Clone, PartialEq)]
pub struct Akin {
    /// The claim and the runs that made it.
    pub cluster: Cluster,
    /// Whether two differently-worded claims were treated as one.
    ///
    /// Carried into the witness note, so `memo why` can say the corroboration was a rewording
    /// rather than a repeat and a person can disagree with it.
    pub near: bool,
}

/// rewording joins the best-established statement of the claim rather than founding a rival one.
#[must_use]
pub fn merge(clusters: Vec<Cluster>) -> Vec<Akin> {
    let mut out: Vec<Akin> = Vec::new();

    for cluster in clusters {
        let joined = out
            .iter_mut()
            .take(COMPARED)
            .find(|held| memo_model::same_claim(&held.cluster.text, &cluster.text));

        match joined {
            Some(held) => {
                // Only a different digest is a rewording. Identical text folding together is an
                // exact repeat, and a witness note claiming otherwise would be a lie about the
                // one thing this flag exists to report.
                let reworded = held.cluster.hash != cluster.hash;
                for session in cluster.sessions {
                    if !held.cluster.sessions.contains(&session) {
                        held.cluster.sessions.push(session);
                    }
                }
                held.cluster.first_seen = held.cluster.first_seen.min(cluster.first_seen);
                held.cluster.sources.extend(cluster.sources);
                held.near |= reworded;
            }
            None => out.push(Akin {
                cluster,
                near: false,
            }),
        }
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    use memo_model::SessionId;

    fn cluster(text: &str, sessions: &[&str]) -> Cluster {
        Cluster {
            text: text.to_owned(),
            hash: memo_model::content_hash(text),
            sessions: sessions.iter().map(|s| SessionId::new(*s)).collect(),
            sources: Vec::new(),
            first_seen: 0,
        }
    }

    #[test]
    fn two_runs_wording_it_differently_corroborate() {
        // The whole point. Before this each of these was one claim seen once, so neither
        // reached the two distinct sessions CALLUS needs and neither crossed.
        let merged = merge(vec![
            cluster("we use make test", &["01A"]),
            cluster("run make test instead", &["01B"]),
        ]);
        assert_eq!(merged.len(), 1, "one claim: {merged:?}");
        assert_eq!(merged[0].cluster.sessions.len(), 2);
        assert!(merged[0].near, "and it says the match was a rewording");
    }

    #[test]
    fn a_claim_never_corroborates_its_own_replacement() {
        // The failure that matters. These are the same subject with a different value, which is
        // a revision — and a store that took it as agreement would hold a fact no run stated.
        for (a, b) in [
            ("we deploy with fly.io", "we deploy with heroku"),
            (
                "the api key is in .env.local",
                "the api key is in .env.production",
            ),
            (
                "never commit to main directly",
                "never commit to develop directly",
            ),
            (
                "run make test before pushing",
                "run make test after pulling",
            ),
        ] {
            assert!(
                !memo_model::same_claim(a, b),
                "{a:?} must not corroborate {b:?}"
            );
            let merged = merge(vec![cluster(a, &["01A"]), cluster(b, &["01B"])]);
            assert_eq!(merged.len(), 2, "{a:?} / {b:?}");
        }
    }

    #[test]
    fn different_subjects_stay_apart() {
        for (a, b) in [
            ("we use make test", "we use cargo build"),
            ("the database is postgres", "the cache is redis"),
            (
                "migrations live in db/migrate",
                "fixtures live in db/fixtures",
            ),
            (
                "the staging box is at 10.0.0.7",
                "the production box is at 10.0.0.8",
            ),
            (
                "run make fmt before committing",
                "run make lint after committing",
            ),
        ] {
            assert!(
                !memo_model::same_claim(a, b),
                "{a:?} must not merge with {b:?}"
            );
        }
    }

    #[test]
    fn an_exact_repeat_is_not_reported_as_a_near_match() {
        // `near` drives what the witness note says, so it has to mean something. An identical
        // claim from two runs is a repeat and must read as one.
        let merged = merge(vec![
            cluster("we use make test", &["01A"]),
            cluster("we use make test", &["01B"]),
        ]);
        assert_eq!(merged.len(), 1);
        assert!(!merged[0].near);
    }

    #[test]
    fn a_run_saying_it_twice_is_still_one_run() {
        // Merging must not manufacture the diversity it is being counted for.
        let merged = merge(vec![
            cluster("we use make test", &["01A"]),
            cluster("run make test instead", &["01A"]),
        ]);
        assert_eq!(merged.len(), 1);
        assert_eq!(
            merged[0].cluster.sessions.len(),
            1,
            "one session, however many ways it said it"
        );
    }

    #[test]
    fn a_claim_with_nothing_in_it_matches_nothing() {
        assert!(!memo_model::same_claim("", ""));
        assert!(!memo_model::same_claim("the it is", "of and to"));
        assert_eq!(memo_model::claim_overlap("", "we use make"), 0.0);
    }
}
