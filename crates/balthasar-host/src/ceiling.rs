//! What a peer may write, and what only the owner may.
//!
//! A memory layer with an open write socket is a memory-poisoning surface, and the literature
//! has both a name for that and papers about it. The answer here is attribution plus a ceiling
//! rather than a refusal: a peer may propose, and the ladder still decides.

use balthasar_ipc::Peer;
use balthasar_model::{ScopeId, WitnessKind};

/// Which door a call came through.
#[derive(Debug, Clone, PartialEq)]
pub enum Door {
    /// The CLI, the configuration, or the session that owns the store.
    Owner,
    /// A socket peer, identified by the kernel.
    Socket(Peer),
}

impl Door {
    /// Whether this door may pin.
    ///
    /// Pinning makes a belief permanent. A process that could do it could install one in an
    /// agent that nobody chose and nothing will decay.
    #[must_use]
    pub fn may_pin(&self) -> bool {
        matches!(self, Self::Owner)
    }

    /// Whether this door may claim an evaluation came from the person.
    ///
    /// A peer may report how its own action went. It may not sign that report as the user's
    /// judgment, because the two carry different weight in every policy that reads them and a
    /// peer that could forge the stronger one could manufacture its own authority.
    #[must_use]
    pub fn may_evaluate_as_user(&self) -> bool {
        matches!(self, Self::Owner)
    }

    /// Whether this door may write to the global store.
    ///
    /// A wrong project fact contaminates one project; a wrong global one contaminates every
    /// project. A peer writes to the scope it is working in.
    #[must_use]
    pub fn may_reach_global(&self) -> bool {
        matches!(self, Self::Owner)
    }

    /// Whether this door may remove a row permanently.
    #[must_use]
    pub fn may_purge(&self) -> bool {
        matches!(self, Self::Owner)
    }

    /// The strongest kind of evidence this door may claim.
    ///
    /// An imperative is "the person said so". A peer saying it would be a process forging the
    /// one witness that crosses the gate alone and pins on the way through.
    #[must_use]
    pub fn strongest(&self) -> WitnessKind {
        match self {
            Self::Owner => WitnessKind::Imperative,
            Self::Socket(_) => WitnessKind::Manual,
        }
    }

    /// The evidence kind a write through this door actually gets.
    #[must_use]
    pub fn witness_for(&self, asked: WitnessKind) -> WitnessKind {
        match self {
            Self::Owner => asked,
            // Not a refusal. A peer's proposal is worth something — it is simply not worth
            // what a person at a keyboard is worth, and the ladder is what tells them apart.
            Self::Socket(_) => WitnessKind::Manual,
        }
    }

    /// How a witness records who wrote it.
    #[must_use]
    pub fn who(&self) -> Option<String> {
        match self {
            Self::Owner => None,
            Self::Socket(peer) => Some(peer.named()),
        }
    }

    /// The scope a write may actually land in.
    ///
    /// A peer asking for global is given the project rather than refused: it wanted something
    /// remembered, and remembering it narrowly is closer to that than not at all.
    #[must_use]
    pub fn scope_for(&self, asked: &ScopeId, working_in: &ScopeId) -> ScopeId {
        if asked.is_global() && !self.may_reach_global() {
            return working_in.clone();
        }
        asked.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer() -> Door {
        Door::Socket(Peer {
            pid: 4021,
            uid: 1000,
            program: Some("harness".to_owned()),
        })
    }

    #[test]
    fn the_owner_may_do_everything() {
        let door = Door::Owner;
        assert!(door.may_pin() && door.may_reach_global() && door.may_purge());
        assert_eq!(door.strongest(), WitnessKind::Imperative);
    }

    #[test]
    fn a_peer_may_not_pin() {
        // A process that could pin could install a permanent belief nobody chose and nothing
        // will decay.
        assert!(!peer().may_pin());
    }

    #[test]
    fn a_peer_may_not_reach_the_global_store() {
        // A wrong project fact contaminates one project. A wrong global one contaminates all
        // of them.
        assert!(!peer().may_reach_global());
        let landed = peer().scope_for(&ScopeId::global(), &ScopeId::new("/w/thing"));
        assert_eq!(landed.as_str(), "/w/thing", "narrowed, not refused");
    }

    #[test]
    fn a_peer_may_not_forge_an_imperative() {
        // The one witness that crosses the gate alone and pins on the way through.
        assert_eq!(
            peer().witness_for(WitnessKind::Imperative),
            WitnessKind::Manual
        );
        assert_eq!(peer().strongest(), WitnessKind::Manual);
    }

    #[test]
    fn a_peers_write_is_still_worth_something() {
        // Attribution and a ceiling, not a refusal. It proposes; the ladder decides.
        assert!(WitnessKind::Manual.weight() > 0.0);
        assert!(!WitnessKind::Manual.crosses_alone(balthasar_model::floor::PROMOTE));
    }

    #[test]
    fn a_peer_may_not_purge() {
        assert!(!peer().may_purge());
    }

    #[test]
    fn every_write_by_a_peer_names_the_process() {
        // `balthasar why` has to be able to say which process believes something.
        let named = peer().who().expect("a name");
        assert!(
            named.contains("harness") && named.contains("4021"),
            "{named}"
        );
        assert_eq!(Door::Owner.who(), None);
    }

    #[test]
    fn the_owner_keeps_the_scope_it_asked_for() {
        let landed = Door::Owner.scope_for(&ScopeId::global(), &ScopeId::new("/w/thing"));
        assert!(landed.is_global());
    }
}
