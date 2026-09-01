//! Working out which memories are related, from what the store already holds.
//!
//! Every derivation here is a rule over clocks, spans, slots and entities. No model, no
//! embedder — which is what makes the temporal, causal and entity families survive
//! `AEON_NO_EMBED=1` intact, and leaves semantic with an exact-overlap floor rather than
//! nothing.
//!
//! The causal rules are the ones worth being careful about. Temporal adjacency is not
//! causation, and a derivation that treated it as such would fill the store with confident
//! nonsense. What is used instead is transcript *structure*: a thing failed, a different thing
//! then worked, and both are inside one episode. That is still an inference, and it is labelled
//! with its source so a reader can discount it.

use aeon_model::{Derivation, Memory, MemoryId, Relation, Timestamp, View};

/// Which version of these rules produced an edge.
pub const DERIVATION: u32 = 1;

/// What the rules will not do below these.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thresholds {
    /// Seconds within which two memories count as overlapping rather than ordered.
    pub overlap_seconds: i64,
    /// How many entities two memories must share before that is worth an edge.
    pub shared_entities: usize,
    /// The share of words two memories must have in common to be called similar.
    pub overlap_share: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            overlap_seconds: 300,
            shared_entities: 1,
            overlap_share: 0.6,
        }
    }
}

/// Order edges between memories, from the clocks they carry.
///
/// Uses `happened_at` where a memory has one and `observed_at` otherwise, because when a thing
/// occurred and when aeon was told about it are different questions and only the first orders
/// the world. Memories closer together than `overlap_seconds` are called overlapping rather
/// than ordered — two observations from the same minute are not evidence of sequence.
#[must_use]
pub fn temporal(memories: &[Memory], rules: &Thresholds, now: Timestamp) -> Vec<Relation> {
    let mut held: Vec<(&MemoryId, Timestamp)> = memories
        .iter()
        .map(|m| {
            (
                &m.id,
                m.temporal.happened_at.unwrap_or(m.temporal.observed_at),
            )
        })
        .collect();
    held.sort_by_key(|(id, at)| (*at, id.as_str().to_owned()));

    let mut out = Vec::new();
    for pair in held.windows(2) {
        let ((one, when), (two, then)) = (&pair[0], &pair[1]);
        let apart = then - when;
        let view = if apart.abs() <= rules.overlap_seconds {
            View::Overlaps
        } else {
            View::Before
        };
        out.push(Relation {
            from: (*one).clone(),
            to: (*two).clone(),
            view,
            // Adjacency in a sorted list is weak evidence of anything, and the weight says so.
            weight: 0.5,
            source: Derivation::Rule,
            derivation_version: DERIVATION,
            evidence_cursor: None,
            created_at: now,
        });
    }
    out
}

/// Repair chains, from the shape of the transcript.
///
/// A failure followed by a different approach that worked, inside one episode. Both directions
/// are written because they answer different questions — *what fixed this* and *why did this
/// exist* — and neither is the inverse of the other.
///
/// `within` bounds the pairing to one span. Without it a failure in the morning would be
/// "resolved by" a success in the evening that had nothing to do with it, which is exactly the
/// temporal-adjacency mistake this module exists to avoid.
#[must_use]
pub fn repairs(steps: &[Step], now: Timestamp) -> Vec<Relation> {
    let mut out = Vec::new();
    for (at, step) in steps.iter().enumerate() {
        if !step.failed {
            continue;
        }
        // The next success in the same episode, and only the next: a failure is repaired once.
        let fix = steps
            .iter()
            .skip(at + 1)
            .take_while(|later| later.episode == step.episode)
            .find(|later| !later.failed && later.command != step.command);
        let Some(fix) = fix else { continue };

        out.push(Relation {
            from: fix.memory.clone(),
            to: step.memory.clone(),
            view: View::Resolved,
            weight: 0.8,
            source: Derivation::Structure,
            derivation_version: DERIVATION,
            evidence_cursor: fix.cursor,
            created_at: now,
        });
        out.push(Relation {
            from: step.memory.clone(),
            to: fix.memory.clone(),
            view: View::Caused,
            weight: 0.6,
            source: Derivation::Structure,
            derivation_version: DERIVATION,
            evidence_cursor: step.cursor,
            created_at: now,
        });
    }
    out
}

/// One tool call inside an episode, as a repair chain needs to see it.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    /// The memory that holds it.
    pub memory: MemoryId,
    /// Which episode it belongs to.
    pub episode: String,
    /// What was run.
    pub command: String,
    /// Whether it failed.
    pub failed: bool,
    /// Where in the transcript.
    pub cursor: Option<u64>,
}

/// Same-entity edges, from what two memories are both about.
///
/// Weighted by rarity, so two memories sharing `make` says less than two sharing
/// `flyctl deploy --now`. A term everything mentions is not what anything is about.
#[must_use]
pub fn entities(
    memories: &[(MemoryId, Vec<String>)],
    rarity: &dyn Fn(&str) -> f64,
    rules: &Thresholds,
    now: Timestamp,
) -> Vec<Relation> {
    let mut out = Vec::new();
    for (at, (one, mine)) in memories.iter().enumerate() {
        for (two, theirs) in memories.iter().skip(at + 1) {
            let shared: Vec<&String> = mine.iter().filter(|e| theirs.contains(e)).collect();
            if shared.len() < rules.shared_entities {
                continue;
            }
            // The rarest shared term carries the edge. Summing would let a pile of common words
            // outweigh one distinctive one, which is the opposite of what rarity is for.
            let weight = shared
                .iter()
                .map(|e| rarity(e))
                .fold(0.0_f64, f64::max)
                .clamp(0.0, 1.0);
            out.push(Relation {
                from: one.clone(),
                to: two.clone(),
                view: View::SameEntity,
                weight,
                source: Derivation::Rule,
                derivation_version: DERIVATION,
                evidence_cursor: None,
                created_at: now,
            });
        }
    }
    out
}

/// Similar-to edges with no embedder at all.
///
/// Exact word overlap over the distinctive words of each memory. Crude next to cosine, and the
/// point is that it exists: with embeddings switched off the semantic family still returns
/// something rather than collapsing, so nothing downstream has to special-case its absence.
#[must_use]
pub fn overlap(
    memories: &[(MemoryId, String)],
    rules: &Thresholds,
    now: Timestamp,
) -> Vec<Relation> {
    let words: Vec<(MemoryId, Vec<String>)> = memories
        .iter()
        .map(|(id, text)| (id.clone(), distinctive(text)))
        .collect();

    let mut out = Vec::new();
    for (at, (one, mine)) in words.iter().enumerate() {
        if mine.is_empty() {
            continue;
        }
        for (two, theirs) in words.iter().skip(at + 1) {
            if theirs.is_empty() {
                continue;
            }
            let shared = mine.iter().filter(|w| theirs.contains(w)).count();
            let share = shared as f64 / mine.len().min(theirs.len()) as f64;
            if share < rules.overlap_share {
                continue;
            }
            out.push(Relation {
                from: one.clone(),
                to: two.clone(),
                view: View::SimilarTo,
                weight: share.clamp(0.0, 1.0),
                source: Derivation::Rule,
                derivation_version: DERIVATION,
                evidence_cursor: None,
                created_at: now,
            });
        }
    }
    out
}

/// The words of a memory worth comparing, lowercased and deduplicated.
fn distinctive(text: &str) -> Vec<String> {
    const NOISE: &[&str] = &[
        "the", "a", "an", "is", "are", "was", "were", "to", "of", "in", "on", "at", "for", "and",
        "or", "it", "this", "that", "we", "you", "i", "be", "with", "as", "by", "from",
    ];
    let mut out: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_' && c != '.' && c != '/')
        .filter(|w| w.len() > 2)
        .map(str::to_lowercase)
        .filter(|w| !NOISE.contains(&w.as_str()))
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use aeon_model::{Body, NoteKind, ScopeId, Temporal, Tier};

    const NOW: Timestamp = 1_756_000_000;

    fn memory(id: &str, text: &str, at: Timestamp) -> Memory {
        let mut held = Memory::new(
            MemoryId::new(id),
            Tier::Fact,
            ScopeId::new("/w/p"),
            Body::note(text, NoteKind::Claim),
            at,
        );
        held.temporal = Temporal::recalled(at, at);
        held
    }

    fn step(id: &str, episode: &str, command: &str, failed: bool, cursor: u64) -> Step {
        Step {
            memory: MemoryId::new(id),
            episode: episode.to_owned(),
            command: command.to_owned(),
            failed,
            cursor: Some(cursor),
        }
    }

    #[test]
    fn things_far_apart_are_ordered() {
        let held = vec![memory("a", "first", NOW), memory("b", "second", NOW + 3600)];
        let out = temporal(&held, &Thresholds::default(), NOW);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].view, View::Before);
        assert_eq!(out[0].from, MemoryId::new("a"));
    }

    #[test]
    fn things_from_the_same_minute_are_not_evidence_of_sequence() {
        // Two observations seconds apart say nothing about which caused which, and an ordering
        // edge between them would be a false claim a causal query could pick up.
        let held = vec![memory("a", "first", NOW), memory("b", "second", NOW + 5)];
        let out = temporal(&held, &Thresholds::default(), NOW);
        assert_eq!(out[0].view, View::Overlaps);
    }

    #[test]
    fn ordering_uses_when_it_happened_not_when_we_heard() {
        // Backfilling a transcript from last month must not order it after this morning's work.
        let mut old = memory("old", "happened long ago", NOW);
        old.temporal.happened_at = Some(NOW - 90 * 86_400);
        let new = memory("new", "happened just now", NOW);

        let out = temporal(&[new, old], &Thresholds::default(), NOW);
        assert_eq!(
            out[0].from,
            MemoryId::new("old"),
            "the older event came first"
        );
    }

    #[test]
    fn a_failure_and_the_thing_that_fixed_it_are_linked_both_ways() {
        let steps = vec![
            step("m1", "e1", "cargo test", true, 1),
            step("m2", "e1", "make test", false, 2),
        ];
        let out = repairs(&steps, NOW);

        assert_eq!(out.len(), 2);
        let resolved = out
            .iter()
            .find(|r| r.view == View::Resolved)
            .expect("a fix");
        assert_eq!(resolved.from, MemoryId::new("m2"));
        assert_eq!(resolved.to, MemoryId::new("m1"));
        assert_eq!(resolved.source, Derivation::Structure);
    }

    #[test]
    fn a_repair_is_never_paired_across_episodes() {
        // Otherwise this morning's failure is "resolved by" this evening's success, which is
        // the temporal-adjacency mistake wearing a causal label.
        let steps = vec![
            step("m1", "e1", "cargo test", true, 1),
            step("m2", "e2", "make test", false, 9),
        ];
        assert!(repairs(&steps, NOW).is_empty());
    }

    #[test]
    fn rerunning_the_same_command_is_not_a_repair() {
        // A flaky test that passes on the second run taught nobody anything.
        let steps = vec![
            step("m1", "e1", "cargo test", true, 1),
            step("m2", "e1", "cargo test", false, 2),
        ];
        assert!(repairs(&steps, NOW).is_empty());
    }

    #[test]
    fn a_failure_is_repaired_once() {
        let steps = vec![
            step("m1", "e1", "cargo test", true, 1),
            step("m2", "e1", "make test", false, 2),
            step("m3", "e1", "make lint", false, 3),
        ];
        let fixes: Vec<_> = repairs(&steps, NOW)
            .into_iter()
            .filter(|r| r.view == View::Resolved)
            .collect();
        assert_eq!(fixes.len(), 1);
        assert_eq!(
            fixes[0].from,
            MemoryId::new("m2"),
            "the first success, not every one"
        );
    }

    #[test]
    fn a_rare_shared_term_outweighs_a_common_one() {
        // Two memories both mentioning `make` says little; both mentioning `flyctl` says a lot.
        let held = vec![
            (
                MemoryId::new("a"),
                vec!["make".to_owned(), "flyctl".to_owned()],
            ),
            (
                MemoryId::new("b"),
                vec!["make".to_owned(), "flyctl".to_owned()],
            ),
        ];
        let rarity = |e: &str| if e == "make" { 0.1 } else { 0.9 };
        let out = entities(&held, &rarity, &Thresholds::default(), NOW);

        assert_eq!(out.len(), 1);
        assert!(
            (out[0].weight - 0.9).abs() < f64::EPSILON,
            "{:?}",
            out[0].weight
        );
    }

    #[test]
    fn memories_about_nothing_in_common_are_not_related() {
        let held = vec![
            (MemoryId::new("a"), vec!["make".to_owned()]),
            (MemoryId::new("b"), vec!["flyctl".to_owned()]),
        ];
        let out = entities(&held, &|_| 0.5, &Thresholds::default(), NOW);
        assert!(out.is_empty());
    }

    #[test]
    fn the_semantic_family_still_answers_with_no_embedder() {
        // Commitment 3. Exact overlap is crude next to cosine; the point is that it exists, so
        // nothing downstream has to special-case an absent embedder.
        let held = vec![
            (MemoryId::new("a"), "the deploy target is fly.io".to_owned()),
            (
                MemoryId::new("b"),
                "deploy target fly.io confirmed".to_owned(),
            ),
        ];
        let out = overlap(&held, &Thresholds::default(), NOW);
        assert_eq!(out.len(), 1, "{out:#?}");
        assert_eq!(out[0].view, View::SimilarTo);
        assert_eq!(out[0].source, Derivation::Rule);
    }

    #[test]
    fn common_words_alone_do_not_make_two_memories_similar() {
        let held = vec![
            (MemoryId::new("a"), "we are in the office".to_owned()),
            (MemoryId::new("b"), "the deploy target is fly.io".to_owned()),
        ];
        assert!(overlap(&held, &Thresholds::default(), NOW).is_empty());
    }

    #[test]
    fn every_derived_edge_names_what_produced_it() {
        let steps = vec![
            step("m1", "e1", "cargo test", true, 1),
            step("m2", "e1", "make test", false, 2),
        ];
        let mut all = repairs(&steps, NOW);
        all.extend(temporal(
            &[memory("a", "one", NOW), memory("b", "two", NOW + 3600)],
            &Thresholds::default(),
            NOW,
        ));
        all.extend(entities(
            &[
                (MemoryId::new("a"), vec!["flyctl".to_owned()]),
                (MemoryId::new("b"), vec!["flyctl".to_owned()]),
            ],
            &|_| 0.9,
            &Thresholds::default(),
            NOW,
        ));

        assert!(!all.is_empty());
        for edge in all {
            assert!(
                edge.explain().contains(edge.source.as_str()),
                "{}",
                edge.explain()
            );
            assert!(
                !edge.source.is_optional(),
                "a rule needed a model: {edge:?}"
            );
        }
    }
}
