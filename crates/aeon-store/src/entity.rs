//! What a memory is *about*.
//!
//! Full-text search matches words; an entity index matches things. They come apart exactly
//! where it matters: `deployment` shares no token with `we deploy with fly`, but both are about
//! *fly*, and a query naming a rare thing is a much stronger signal than a query naming a
//! common one.
//!
//! Two ideas taken from elsewhere, and one deliberate departure.
//!
//! **Rarity weighting**, from mem0's entity store: an entity linked to two memories is far more
//! informative than one linked to a thousand. It is IDF, applied to things rather than words.
//!
//! **Query expansion**, from cortex: the entities in a query pull in memories that mention
//! them, bounded on both time and term count so the cost never scales with the store.
//!
//! **The departure.** In mem0 the entity signal can only *reorder* what vector search already
//! found — if the first stage missed a memory, nothing rescues it. Here entities may *add*
//! candidates, for the same reason the vector top-up exists: a gate with no way out of it is a
//! gate that decides what can never be found.

use crate::{Store, StoreError};
use aeon_model::MemoryId;
use rusqlite::params;

/// What kind of thing a name refers to.
///
/// Kept because it decides how much a match is worth: two memories about `src/lib.rs` are
/// about the same file, and two memories mentioning `Docker` might be about anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A path: `src/lib.rs`, `config/init.lua`.
    Path,
    /// Something in backticks, or something that looks like a command.
    Command,
    /// A capitalised name: `Postgres`, `Alice`, `Fly`.
    Proper,
    /// A `snake_case` or `camelCase` identifier.
    Symbol,
}

impl Kind {
    /// The column spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Path => "path",
            Self::Command => "command",
            Self::Proper => "proper",
            Self::Symbol => "symbol",
        }
    }

    /// How much a match on this kind counts, before rarity.
    ///
    /// A path or a command names one thing and names it exactly. A capitalised word might be a
    /// product, a person, or the first word of a sentence that got through.
    #[must_use]
    pub fn confidence(self) -> f64 {
        match self {
            Self::Path | Self::Command => 1.0,
            Self::Symbol => 0.8,
            Self::Proper => 0.6,
        }
    }
}

/// One thing a memory is about.
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    /// Lowercased, for matching.
    pub name: String,
    /// As it was written, for showing.
    pub display: String,
    /// What kind of thing.
    pub kind: Kind,
}

/// How many entities one memory may contribute.
///
/// A bound, so a memory that happens to be a wall of paths does not fill the index on its own.
const PER_MEMORY: usize = 12;

/// How many entities a query may be expanded on.
///
/// cortex's cap, and for its reason: the downstream SQL stays small however long the question.
const PER_QUERY: usize = 8;

/// Words that are capitalised for grammar rather than because they name anything.
const NOT_A_NAME: &[&str] = &[
    "the", "this", "that", "these", "those", "there", "then", "they", "them", "and", "but", "for",
    "with", "from", "into", "when", "where", "what", "which", "while", "after", "before", "here",
    "it", "its", "is", "was", "are", "were", "be", "been", "have", "has", "had", "do", "does",
    "did", "will", "would", "can", "could", "should", "must", "not", "no", "yes", "you", "your",
    "we", "our", "i", "my", "me", "he", "she", "his", "her", "a", "an", "of", "in", "on", "at",
    "to", "by", "as", "if", "or", "so", "run", "use", "used", "using", "make", "just", "only",
];

/// Pull the things a piece of text is about out of it.
///
/// No model. Backticks, paths, identifiers and capitalised words carry most of what a coding
/// agent's memories are about, and all four are visible without one.
#[must_use]
pub fn extract(text: &str) -> Vec<Entity> {
    let mut out: Vec<Entity> = Vec::new();

    // Backticks first, and their contents are not re-scanned: `make test` is one command, not
    // the word "make" and the word "test".
    let mut rest = String::with_capacity(text.len());
    let mut in_ticks = false;
    let mut held = String::new();
    for c in text.chars() {
        if c == '`' {
            if in_ticks {
                push(&mut out, held.trim(), Kind::Command);
                held.clear();
            }
            in_ticks = !in_ticks;
            rest.push(' ');
            continue;
        }
        if in_ticks {
            held.push(c);
        } else {
            rest.push(c);
        }
    }

    for word in rest.split_whitespace() {
        let word = word.trim_matches(|c: char| {
            !c.is_alphanumeric() && c != '/' && c != '.' && c != '_' && c != '-'
        });
        if word.len() < 2 || out.len() >= PER_MEMORY {
            continue;
        }

        // A path names one file and names it exactly.
        if word.contains('/') && !word.starts_with("//") {
            push(&mut out, word, Kind::Path);
            continue;
        }
        // A dotted name with an extension, `Cargo.toml`.
        if word.contains('.') && !word.ends_with('.') && word.split('.').count() == 2 {
            let tail = word.rsplit('.').next().unwrap_or("");
            if (2..=5).contains(&tail.len()) && tail.chars().all(|c| c.is_ascii_alphabetic()) {
                push(&mut out, word, Kind::Path);
                continue;
            }
        }
        // An identifier.
        if word.contains('_') || has_inner_capital(word) {
            push(&mut out, word, Kind::Symbol);
            continue;
        }
        // A capitalised word that is not capitalised for grammar.
        let mut chars = word.chars();
        if chars.next().is_some_and(char::is_uppercase)
            && chars.clone().any(|c| c.is_lowercase())
            && !NOT_A_NAME.contains(&word.to_lowercase().as_str())
        {
            push(&mut out, word, Kind::Proper);
        }
    }
    out
}

/// Add one, keeping it unique by normalised name.
fn push(out: &mut Vec<Entity>, raw: &str, kind: Kind) {
    let display = raw
        .trim()
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '/' && c != '.');
    if display.len() < 2 || out.len() >= PER_MEMORY {
        return;
    }
    let name = display.to_lowercase();
    if NOT_A_NAME.contains(&name.as_str()) || out.iter().any(|e| e.name == name) {
        return;
    }
    out.push(Entity {
        name,
        display: display.to_owned(),
        kind,
    });
}

/// `camelCase`, but not `ALLCAPS`.
fn has_inner_capital(word: &str) -> bool {
    let mut seen_lower = false;
    for c in word.chars() {
        if c.is_lowercase() {
            seen_lower = true;
        } else if c.is_uppercase() && seen_lower {
            return true;
        }
    }
    false
}

/// How much a match on an entity is worth, given how many memories mention it.
///
/// Rarity weighted, mem0's idea: an entity linked to two memories is far more informative than
/// one linked to a thousand. `1 / (1 + ln(1 + n))` — one memory scores `0.59`, ten `0.29`, a
/// thousand `0.13`. It never reaches zero, because a common entity is still a weak signal
/// rather than no signal.
#[must_use]
pub fn rarity(linked: u32) -> f64 {
    1.0 / (1.0 + f64::from(linked).ln_1p())
}

impl Store {
    /// Index what a memory is about.
    ///
    /// Called on write. Replacing rather than adding, so re-indexing after a better extractor
    /// does not leave the old names behind.
    pub fn index_entities(
        &mut self,
        id: &MemoryId,
        scope: &str,
        text: &str,
    ) -> Result<usize, StoreError> {
        let found = extract(text);
        let tx = self.db_mut().transaction()?;
        tx.execute("DELETE FROM entity WHERE memory = ?1", params![id.as_str()])?;
        for entity in &found {
            tx.execute(
                "INSERT OR IGNORE INTO entity (scope, name, display, memory, kind) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    scope,
                    entity.name,
                    entity.display,
                    id.as_str(),
                    entity.kind.as_str()
                ],
            )?;
        }
        tx.commit()?;
        Ok(found.len())
    }

    /// Memories a query's entities point at, with what each match is worth.
    ///
    /// Bounded on the number of entities and on how many memories any one of them may pull in,
    /// so the cost of a question never scales with the size of the store. Answers an empty map
    /// rather than an error when it runs out of budget: an error channel would be an oracle for
    /// how much is held.
    pub fn by_entity(
        &self,
        scope: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(MemoryId, f64)>, StoreError> {
        let asked = extract(query);
        if asked.is_empty() {
            return Ok(Vec::new());
        }

        let mut scored: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
        for entity in asked.iter().take(PER_QUERY) {
            let linked: u32 = self
                .db()
                .query_row(
                    "SELECT count(*) FROM entity WHERE scope = ?1 AND name = ?2",
                    params![scope, entity.name],
                    |r| r.get::<_, i64>(0),
                )?
                .max(0) as u32;
            if linked == 0 {
                continue;
            }
            let worth = rarity(linked) * entity.kind.confidence();

            let mut statement = self
                .db()
                .prepare("SELECT memory FROM entity WHERE scope = ?1 AND name = ?2 LIMIT ?3")?;
            let hits = statement
                .query_map(params![scope, entity.name, limit as i64], |r| {
                    r.get::<_, String>(0)
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for id in hits {
                *scored.entry(id).or_insert(0.0) += worth;
            }
        }

        let mut out: Vec<(MemoryId, f64)> = scored
            .into_iter()
            // Saturated, so a memory matching five entities does not outrank everything by
            // arithmetic alone. Every other signal lives in 0..1 and this one has to as well.
            .map(|(id, sum)| (MemoryId::new(id), sum / (sum + 1.0)))
            .collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1));
        out.truncate(limit);
        Ok(out)
    }

    /// Every entity one memory is about, for `aeon why`.
    pub fn entities_of(&self, id: &MemoryId) -> Result<Vec<Entity>, StoreError> {
        let mut statement = self
            .db()
            .prepare("SELECT name, display, kind FROM entity WHERE memory = ?1 ORDER BY name")?;
        let found = statement
            .query_map(params![id.as_str()], |r| {
                Ok(Entity {
                    name: r.get(0)?,
                    display: r.get(1)?,
                    kind: match r.get::<_, String>(2)?.as_str() {
                        "path" => Kind::Path,
                        "command" => Kind::Command,
                        "symbol" => Kind::Symbol,
                        _ => Kind::Proper,
                    },
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(text: &str) -> Vec<String> {
        extract(text).into_iter().map(|e| e.name).collect()
    }

    #[test]
    fn a_backticked_span_is_one_command() {
        // `make test` is one thing. Splitting it into "make" and "test" would index two words
        // that name nothing and lose the thing that was actually said.
        let found = extract("run `make test` to check");
        assert!(
            found
                .iter()
                .any(|e| e.name == "make test" && e.kind == Kind::Command)
        );
        assert!(!found.iter().any(|e| e.name == "test"));
    }

    #[test]
    fn a_path_is_a_path() {
        assert!(names("the bug is in src/lib.rs somewhere").contains(&"src/lib.rs".to_owned()));
        assert!(names("check Cargo.toml first").contains(&"cargo.toml".to_owned()));
    }

    #[test]
    fn an_identifier_is_kept() {
        let found = names("the memory_slot_live index refuses it");
        assert!(found.contains(&"memory_slot_live".to_owned()));
    }

    #[test]
    fn a_proper_noun_is_kept() {
        assert!(names("we moved from Heroku to Fly").contains(&"heroku".to_owned()));
    }

    #[test]
    fn a_word_capitalised_for_grammar_is_not_a_name() {
        // Otherwise the first word of every sentence becomes a thing the memory is about.
        let found = names("The build takes forty seconds. When it fails, retry.");
        assert!(!found.contains(&"the".to_owned()), "{found:?}");
        assert!(!found.contains(&"when".to_owned()), "{found:?}");
    }

    #[test]
    fn nothing_is_indexed_twice() {
        let found = names("Postgres and Postgres again, plus postgres");
        assert_eq!(found.iter().filter(|n| *n == "postgres").count(), 1);
    }

    #[test]
    fn a_wall_of_paths_cannot_fill_the_index_on_its_own() {
        let wall: String = (0..80).map(|n| format!("src/f{n}.rs ")).collect();
        assert!(extract(&wall).len() <= PER_MEMORY);
    }

    #[test]
    fn a_rare_entity_is_worth_more_than_a_common_one() {
        // mem0's idea: a memory linked to one of two "Paris" memories is likelier to be the
        // one you want than one of a thousand.
        assert!(rarity(1) > rarity(10));
        assert!(rarity(10) > rarity(1000));
    }

    #[test]
    fn a_common_entity_is_still_a_signal() {
        // Weak, never absent. Zero would make a popular thing invisible rather than ordinary.
        assert!(rarity(100_000) > 0.0);
    }

    #[test]
    fn rarity_stays_inside_the_range_every_other_signal_uses() {
        for n in [1_u32, 2, 50, 10_000] {
            assert!((0.0..=1.0).contains(&rarity(n)), "{n}");
        }
    }

    #[test]
    fn a_path_counts_for_more_than_a_capitalised_word() {
        // A path names one file exactly. A capitalised word might be a product, a person, or
        // a sentence opener that got through.
        assert!(Kind::Path.confidence() > Kind::Proper.confidence());
    }

    #[test]
    fn text_about_nothing_yields_nothing() {
        assert!(extract("it does the thing when we run it").is_empty());
    }
}
