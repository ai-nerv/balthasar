//! The record everything else is a projection of.

use crate::{
    Body, Contradiction, MemoryId, Privacy, ScopeId, SessionId, Strength, Temporal, Tier,
    Timestamp, Witness, confidence,
};

/// How one memory relates to another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkRelation {
    /// This one replaced that one. Written when a correction lands.
    Supersedes,
    /// These two cannot both be true.
    Contradicts,
    /// This one is evidence for that one.
    Supports,
    /// This one was distilled or consolidated out of that one.
    DerivedFrom,
    /// This one is about that one — an episode about a fact, say.
    About,
}

impl LinkRelation {
    /// The wire and column spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Supersedes => "supersedes",
            Self::Contradicts => "contradicts",
            Self::Supports => "supports",
            Self::DerivedFrom => "derived_from",
            Self::About => "about",
        }
    }
}

/// One edge.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Link {
    /// What it relates to.
    pub to: MemoryId,
    /// How.
    pub rel: LinkRelation,
    /// When the edge was drawn.
    pub at: Timestamp,
}

/// Which door a memory came through.
///
/// Not decoration. The write ceiling is enforced on this: a socket peer cannot pin, cannot
/// reach global, and cannot claim an imperative, and none of that is checkable without knowing
/// how the write arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Through {
    /// The CLI, or the configuration. The owner, at the keyboard.
    #[default]
    Local,
    /// A socket peer. Capped.
    Peer,
    /// A backfill of somebody's transcripts.
    Ingest,
    /// memo's own consolidation.
    Sleep,
}

/// Who wrote it, and how.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Provenance {
    /// Which door.
    pub through: Through,
    /// The peer, when there was one — `"harness[pid 4021]"`, taken from the kernel and never
    /// from a name the peer sent.
    pub who: Option<String>,
}

impl Default for Provenance {
    fn default() -> Self {
        Self {
            through: Through::Local,
            who: None,
        }
    }
}

impl Provenance {
    /// Written by the person at the keyboard.
    #[must_use]
    pub fn local() -> Self {
        Self::default()
    }

    /// Written by a peer, which caps what it may claim.
    #[must_use]
    pub fn peer(who: impl Into<String>) -> Self {
        Self {
            through: Through::Peer,
            who: Some(who.into()),
        }
    }
}

/// One memory.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Memory {
    /// Sortable by creation, and doubling as a timeline.
    pub id: MemoryId,
    /// Which tier.
    pub tier: Tier,
    /// Which store.
    pub scope: ScopeId,
    /// The session that produced it, for scratch and for tracing anything else back.
    pub session: Option<SessionId>,
    /// What it says.
    pub body: Body,
    /// The four clocks.
    pub temporal: Temporal,
    /// How faded.
    pub strength: Strength,
    /// How sure. Derived — see [`Memory::rescore`].
    pub confidence: f64,
    /// How it knows.
    pub witnesses: Vec<Witness>,
    /// Who wrote it.
    pub provenance: Provenance,
    /// How far it may travel.
    pub privacy: Privacy,
    /// What it relates to.
    pub links: Vec<Link>,
    /// Digest of the rendered text, for dedup and the cheap half of clustering.
    pub content_hash: String,
    /// When it left the live set. `None` while it is live.
    pub archived_at: Option<Timestamp>,
    /// Its vector, when something has embedded it.
    ///
    /// Absent is the ordinary state, not a degraded one: recall works lexically and the vector
    /// is a signal that improves the ranking when it is there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
}

impl Memory {
    /// A memory of `body`, in `tier`, learned at `now`.
    ///
    /// Confidence starts at zero and stays there until evidence arrives. There is no
    /// constructor that takes one, because there is no honest way to know it before the
    /// witnesses are attached.
    #[must_use]
    pub fn new(id: MemoryId, tier: Tier, scope: ScopeId, body: Body, now: Timestamp) -> Self {
        let content_hash = crate::content_hash(&body.text());
        Self {
            id,
            tier,
            scope,
            session: None,
            body,
            temporal: Temporal::observed(now),
            strength: Strength::fresh(crate::Importance::Normal, now),
            confidence: 0.0,
            witnesses: Vec::new(),
            provenance: Provenance::local(),
            privacy: Privacy::Open,
            links: Vec::new(),
            content_hash,
            archived_at: None,
            embedding: None,
        }
    }

    /// Attach evidence and recompute what it is worth.
    ///
    /// The only way confidence moves. Adding a witness without rescoring would leave a record
    /// whose number and whose argument disagree, which is the one state `memo why` must never
    /// be able to show.
    pub fn witness(&mut self, witness: Witness, now: Timestamp) {
        self.witnesses.push(witness);
        self.rescore(&[], now);
    }

    /// Recompute confidence from the current evidence and whatever is pulling against it.
    pub fn rescore(&mut self, against: &[Contradiction], now: Timestamp) {
        self.confidence = confidence::of(
            &self.witnesses,
            against,
            !self.temporal.is_live(),
            self.strength.pinned,
            now,
        );
    }

    /// Whether this may be asserted to a model as current truth.
    ///
    /// Two floors, and they are different floors: below `floor` a memory stops being
    /// *asserted* long before it stops being *findable*. That gap is the whole answer to
    /// staleness — an agent that can say "you told me this in March, it may be stale" instead
    /// of stating it flatly.
    #[must_use]
    pub fn is_assertable(&self, floor: f64, now: Timestamp, remote: bool) -> bool {
        self.archived_at.is_none()
            && self.temporal.is_live()
            && self.privacy.may_reach(remote)
            && self.confidence >= floor
            // Faded past the point of being worth keeping is faded past the point of being
            // worth stating. Testing for "greater than zero" let a memory that had decayed to
            // 0.004 still be presented to a model as current truth.
            && self.strength.at_tier(self.tier, now) >= crate::floor::SPENT
            // And stale is not the same as faded. A fact barely decays — disuse is not evidence
            // against a claim about the world — so without this a passing remark from last year
            // would still be stated flatly, having never been contradicted and never re-seen.
            // Staleness is about the clock on the observation, not about how often it was
            // wanted, and a pinned memory is exempt because somebody chose it.
            && (self.strength.pinned
                || !crate::is_stale(self.temporal.observed_at, self.temporal.valid_to, now))
    }

    /// Note that this was recalled: reinforce it, and say so.
    pub fn touch(&mut self, now: Timestamp) {
        self.strength.touch(now);
    }

    /// Move it out of the live set, keeping everything about it.
    ///
    /// Not a delete, and there is no delete next to it. A memory below the floor is still the
    /// answer to a question nothing else answers, and R-Mem's archive costs a column.
    pub fn archive(&mut self, now: Timestamp) {
        if self.archived_at.is_none() {
            self.archived_at = Some(now);
        }
    }

    /// Draw an edge to another memory.
    pub fn link(&mut self, to: MemoryId, rel: LinkRelation, at: Timestamp) {
        if !self.links.iter().any(|l| l.to == to && l.rel == rel) {
            self.links.push(Link { to, rel, at });
        }
    }

    /// One line, for hashing, ranking and showing a person.
    #[must_use]
    pub fn text(&self) -> String {
        self.body.text()
    }

    /// How many distinct sessions witnessed this.
    ///
    /// The diversity number, exposed because `memo why` prints it and a person reading one
    /// witness list should not have to count.
    #[must_use]
    pub fn distinct_sessions(&self) -> usize {
        let mut seen: Vec<&str> = self.witnesses.iter().map(|w| w.session.as_str()).collect();
        seen.sort_unstable();
        seen.dedup();
        seen.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ScopeId, SessionId, WitnessId, WitnessKind};

    const NOW: Timestamp = 1_756_000_000;

    fn fact() -> Memory {
        Memory::new(
            MemoryId::minted(1, 1),
            Tier::Fact,
            ScopeId::global(),
            Body::fact("project", "test_command", "make test"),
            NOW,
        )
    }

    fn witness(kind: WitnessKind, session: &str) -> Witness {
        Witness::new(
            WitnessId::new(format!("w{session}")),
            kind,
            SessionId::new(session),
            ScopeId::global(),
            NOW,
        )
    }

    #[test]
    fn a_memory_with_no_evidence_believes_nothing() {
        assert_eq!(fact().confidence, 0.0);
    }

    #[test]
    fn attaching_evidence_is_the_only_way_confidence_moves() {
        let mut m = fact();
        m.witness(witness(WitnessKind::Imperative, "s1"), NOW);
        assert!(m.confidence > 0.35);
    }

    #[test]
    fn a_superseded_fact_is_still_found_but_no_longer_asserted() {
        let mut m = fact();
        m.witness(witness(WitnessKind::Imperative, "s1"), NOW);
        m.temporal.close(NOW);
        m.rescore(&[], NOW);
        assert!(!m.is_assertable(0.35, NOW, false));
        assert!(
            m.confidence > 0.0,
            "it is still known to have been believed"
        );
    }

    #[test]
    fn a_local_memory_is_withheld_from_a_remote_model() {
        let mut m = fact();
        m.witness(witness(WitnessKind::Imperative, "s1"), NOW);
        m.privacy = Privacy::Local;
        assert!(m.is_assertable(0.35, NOW, false));
        assert!(!m.is_assertable(0.35, NOW, true));
    }

    #[test]
    fn a_memory_faded_to_nothing_is_not_asserted() {
        // It is still findable, still explained, still exported. It is simply no longer
        // something to tell a model is true.
        let mut m = fact();
        m.witness(witness(WitnessKind::Imperative, "s1"), NOW);
        m.strength.importance = crate::Importance::Low;
        let long_after = NOW + 400 * 86_400;
        assert!(m.strength.at(long_after) > 0.0, "not exactly zero");
        assert!(!m.is_assertable(0.35, long_after, false));
    }

    #[test]
    fn an_archived_memory_is_never_asserted() {
        let mut m = fact();
        m.witness(witness(WitnessKind::Imperative, "s1"), NOW);
        m.archive(NOW);
        assert!(!m.is_assertable(0.35, NOW, false));
    }

    #[test]
    fn archiving_twice_keeps_the_first_time() {
        let mut m = fact();
        m.archive(NOW);
        m.archive(NOW + 999);
        assert_eq!(m.archived_at, Some(NOW));
    }

    #[test]
    fn diversity_counts_sessions_not_mentions() {
        let mut m = fact();
        m.witness(witness(WitnessKind::Repetition, "s1"), NOW);
        m.witness(witness(WitnessKind::Repetition, "s1"), NOW);
        m.witness(witness(WitnessKind::Repetition, "s2"), NOW);
        assert_eq!(m.distinct_sessions(), 2);
    }

    #[test]
    fn a_link_is_drawn_once() {
        let mut m = fact();
        let other = MemoryId::minted(2, 2);
        m.link(other.clone(), LinkRelation::Supersedes, NOW);
        m.link(other, LinkRelation::Supersedes, NOW + 5);
        assert_eq!(m.links.len(), 1);
    }

    #[test]
    fn a_memory_round_trips_as_json() {
        // `memo export` is this, a line at a time. A record that will not serialise is a
        // memory nobody can back up.
        let mut m = fact();
        m.witness(witness(WitnessKind::Cost, "s1"), NOW);
        let text = serde_json::to_string(&m).expect("encode");
        assert_eq!(serde_json::from_str::<Memory>(&text).expect("decode"), m);
    }
}
