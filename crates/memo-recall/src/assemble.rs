//! Building what a model is actually told.
//!
//! The order is the design: allocate by weight, sort within a section, drop restatements, drop
//! anything under the floor, redact, and hand unspent room to the next section.

use crate::{Section, budget};
use memo_model::{Memory, ScopeId, Timestamp};
use memo_store::{Recall, Store, Weights};

/// Where the assembled context is going.
///
/// A local llama.cpp and somebody's API are not the same boundary, and pretending they are is
/// how a memory marked local ends up in a request to a third party.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    /// A model on this machine.
    Local,
    /// A model somewhere else.
    Remote,
}

impl Bound {
    /// Whether this boundary leaves the machine.
    #[must_use]
    pub fn is_remote(self) -> bool {
        matches!(self, Self::Remote)
    }
}

/// What to assemble, and how much room there is.
#[derive(Debug, Clone)]
pub struct Ask {
    /// The turn in hand, for sections that score against it.
    pub turn: String,
    /// How many tokens memory may claim.
    pub tokens: usize,
    /// Where it is going.
    pub bound: Bound,
    /// The confidence below which nothing is asserted.
    pub floor: f64,
    /// How retrieval ranks.
    pub weights: Weights,
    /// The moment to score against.
    pub now: Timestamp,
    /// Which project's entity index to consult.
    pub scope: String,
}

/// One rendered section.
#[derive(Debug, Clone, PartialEq)]
pub struct Rendered {
    /// Which section.
    pub id: String,
    /// Its lines, in the order they will be read.
    pub lines: Vec<String>,
    /// What it cost.
    pub tokens: usize,
}

/// Everything a model would be told.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Context {
    /// The sections that had anything to say.
    pub sections: Vec<Rendered>,
    /// What the whole thing costs.
    pub tokens: usize,
    /// How many memories were dropped for being restatements.
    pub deduplicated: usize,
    /// How many were withheld at the boundary.
    pub redacted: usize,
}

impl Context {
    /// Whether anything is being said at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }

    /// The whole thing as prose, which is what a harness injects.
    #[must_use]
    pub fn text(&self) -> String {
        let mut out = String::new();
        for section in &self.sections {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("## {}\n", section.id));
            for line in &section.lines {
                out.push_str(line);
                out.push('\n');
            }
        }
        out
    }
}

/// How near a restatement has to be before it is dropped.
const DUPLICATE: f64 = 0.8;

/// How many candidates a section may consider before the budget decides.
const PER_SECTION: usize = 40;

/// How much of the turn a memory must answer to be worth injecting.
///
/// More than half. A model shown a near-miss will use it, and "the staging box is at 10.0.0.7"
/// in answer to a question about production is worse than saying nothing — which is the whole
/// reason a memory layer can abstain at all.
const RELEVANT_ENOUGH: f64 = 0.55;

/// Assemble the context for one turn.
///
/// `redact` is asked about every line and may rewrite or withhold it. Privacy is enforced here,
/// at the boundary where memory leaves, rather than in the store — which always answers its
/// owner faithfully.
pub fn assemble(
    stores: &[(Store, bool)],
    sections: &[Section],
    ask: &Ask,
    mut redact: impl FnMut(&str, &Memory) -> Option<String>,
) -> Result<Context, memo_store::StoreError> {
    let mut context = Context::default();
    let weights: Vec<f64> = sections.iter().map(|s| s.weight).collect();
    let mut per_weight = budget::share(ask.tokens * budget::CHARS_PER_TOKEN, &weights);
    let mut seen: Vec<String> = Vec::new();

    for section in sections {
        let allowance = (per_weight * section.weight) as usize;
        if allowance == 0 {
            continue;
        }

        let candidates = gather(stores, section, ask)?;
        let mut lines = Vec::new();
        let mut spent = 0_usize;

        for memory in candidates {
            if let Some(limit) = section.limit
                && lines.len() >= limit
            {
                break;
            }

            let text = memory.text();
            if seen
                .iter()
                .any(|held| budget::near_duplicate(held, &text, DUPLICATE))
            {
                context.deduplicated += 1;
                continue;
            }
            let Some(text) = redact(&text, &memory) else {
                context.redacted += 1;
                continue;
            };

            let line = format!("- {text}");
            let Some(line) = budget::fit(&line, allowance.saturating_sub(spent)) else {
                break;
            };
            spent += line.len() + 1;
            seen.push(text);
            lines.push(line);
        }

        if lines.is_empty() {
            continue;
        }
        let tokens = budget::tokens(&lines.join("\n"));
        context.tokens += tokens;
        context.sections.push(Rendered {
            id: section.id.clone(),
            lines,
            tokens,
        });

        // Whatever this section did not use goes to the ones after it, rather than being lost
        // to a section that had four lines to say and room for forty.
        let unspent = allowance.saturating_sub(spent);
        let remaining: f64 = sections
            .iter()
            .skip_while(|s| s.id != section.id)
            .skip(1)
            .map(|s| s.weight)
            .sum();
        if remaining > 0.0 {
            per_weight += unspent as f64 / remaining;
        }
    }
    Ok(context)
}

/// The memories a section would take, best first.
fn gather(
    stores: &[(Store, bool)],
    section: &Section,
    ask: &Ask,
) -> Result<Vec<Memory>, memo_store::StoreError> {
    let floor = section.floor(ask.floor);
    let mut found = Vec::new();

    for (store, near) in stores {
        let mut recall = Recall::of(
            section.query.as_deref().map_or_else(String::new, |q| {
                if q == "turn" {
                    ask.turn.clone()
                } else {
                    q.to_owned()
                }
            }),
            ask.now,
        );
        recall.limit = PER_SECTION;
        recall.tiers.clone_from(&section.tiers);
        recall.floor = floor;
        recall.remote = ask.bound.is_remote();
        recall.weights = ask.weights;
        recall.near = *near;
        recall.scope_name = ask.scope.clone();
        // Only where the section is answering a turn. A section that takes whatever is most
        // salient is not answering a question and has nothing to be relevant to.
        if section.query.is_some() {
            recall.relevance = RELEVANT_ENOUGH;
        }

        for hit in store.recall(&recall)? {
            if section.takes(&hit.memory, *near)
                && hit
                    .memory
                    .is_assertable(floor, ask.now, ask.bound.is_remote())
            {
                found.push((hit.score, hit.memory));
            }
        }
    }

    if section.preserve_order {
        // Chronological, oldest first. Sorting episodes by salience puts last week above this
        // morning, which tells a reader nothing about what happened.
        found.sort_by_key(|(_, memory)| memory.temporal.when());
    } else {
        found.sort_by(|a, b| b.0.total_cmp(&a.0));
    }
    Ok(found.into_iter().map(|(_, memory)| memory).collect())
}

/// Open the stores a scope reads from, project first.
pub fn stores_for(
    scope: &ScopeId,
    open: impl Fn(&ScopeId) -> Result<Store, memo_store::StoreError>,
) -> Result<Vec<(Store, bool)>, memo_store::StoreError> {
    if scope.is_global() {
        return Ok(vec![(open(scope)?, true)]);
    }
    Ok(vec![
        (open(scope)?, true),
        (open(&ScopeId::global())?, false),
    ])
}
