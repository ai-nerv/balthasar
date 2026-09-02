//! What a memory actually says.
//!
//! One payload per tier. The fact variant is memvid's slot model — `(subject, predicate,
//! object)` — which is what makes "what is true now" and "what was true in March" both
//! answerable against a validity interval instead of a graph database.

/// Why a note was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteKind {
    /// A turn as it arrived from a harness.
    Observation,
    /// A summary written when a span left the context window.
    Summary,
    /// The model writing to itself.
    Scratch,
    /// A candidate that reached the hold floor but not the promotion floor, waiting for a
    /// second witness rather than dying with the session.
    Held,
    /// A durable claim that could not be reduced to a slot.
    ///
    /// "We deploy with fly" is a fact, but naming its subject and predicate takes either a
    /// person or a model, and memo requires neither to work. An unslotted claim is kept, found
    /// and asserted like any other; what it gives up is automatic contradiction detection,
    /// because nothing can tell it apart from the claim it replaces without reading it.
    Claim,
}

/// Where in a session's transcript something happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Span {
    /// First cursor covered, inclusive.
    pub from: u64,
    /// Last cursor covered, inclusive.
    pub to: u64,
}

/// How a distilled span of work ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The work was finished.
    Done,
    /// It was abandoned, or the session ended mid-flight.
    #[default]
    Open,
    /// It ended badly, which is often the more instructive case.
    Failed,
}

/// The tier-specific payload.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Body {
    /// A raw observation or a note. What scratch holds.
    Note {
        /// What it says.
        text: String,
        /// Why it was written.
        note: NoteKind,
    },
    /// A distilled span of a session. What episode holds.
    Episode {
        /// What happened, in prose.
        summary: String,
        /// Which part of the transcript it covers.
        span: Span,
        /// Which tools were involved, so a later question about a tool can find it.
        tools: Vec<String>,
        /// How it ended.
        outcome: Outcome,
    },
    /// A claim. What fact holds.
    Fact {
        /// What the claim is about.
        subject: String,
        /// Which property of it.
        predicate: String,
        /// And the value.
        object: String,
    },
    /// A procedure. What habit holds.
    Habit {
        /// When this applies.
        trigger: String,
        /// What to do.
        steps: Vec<String>,
        /// How many times it has been attempted.
        tried: u32,
        /// And how many of those worked. The ratio is why a habit is worth asserting.
        worked: u32,
    },
}

impl Body {
    /// A plain note.
    #[must_use]
    pub fn note(text: impl Into<String>, note: NoteKind) -> Self {
        Self::Note {
            text: text.into(),
            note,
        }
    }

    /// A claim, as three parts.
    #[must_use]
    pub fn fact(
        subject: impl Into<String>,
        predicate: impl Into<String>,
        object: impl Into<String>,
    ) -> Self {
        Self::Fact {
            subject: subject.into(),
            predicate: predicate.into(),
            object: object.into(),
        }
    }

    /// A distilled span of work.
    #[must_use]
    pub fn episode(
        summary: impl Into<String>,
        span: Span,
        tools: Vec<String>,
        outcome: Outcome,
    ) -> Self {
        Self::Episode {
            summary: summary.into(),
            span,
            tools,
            outcome,
        }
    }

    /// A procedure, learned from something that worked once.
    #[must_use]
    pub fn habit(trigger: impl Into<String>, steps: Vec<String>) -> Self {
        Self::Habit {
            trigger: trigger.into(),
            steps,
            tried: 1,
            worked: 1,
        }
    }

    /// The slot this occupies, when it occupies one.
    ///
    /// A fact answers exactly one `(subject, predicate)`, and that is the pair the store's
    /// partial unique index keys on. Nothing else has a slot, which is why nothing else can
    /// contradict.
    #[must_use]
    pub fn slot(&self) -> Option<(&str, &str)> {
        match self {
            Self::Fact {
                subject, predicate, ..
            } => Some((subject, predicate)),
            _ => None,
        }
    }

    /// The value in the slot, when there is one.
    #[must_use]
    pub fn object(&self) -> Option<&str> {
        match self {
            Self::Fact { object, .. } => Some(object),
            _ => None,
        }
    }

    /// One line of prose, for hashing, ranking, indexing and showing a person.
    ///
    /// Everything that has to treat memories uniformly goes through here, so a new body
    /// variant that forgets to render is a compile error rather than an empty search result.
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Self::Note { text, .. } => text.clone(),
            Self::Episode { summary, .. } => summary.clone(),
            Self::Fact {
                subject,
                predicate,
                object,
            } => format!("{subject} {predicate} {object}"),
            Self::Habit { trigger, steps, .. } => {
                format!("{trigger}: {}", steps.join(" then "))
            }
        }
    }

    /// How well a habit has actually worked, `None` before it has been tried.
    #[must_use]
    pub fn success_rate(&self) -> Option<f64> {
        match self {
            Self::Habit { tried, worked, .. } if *tried > 0 => {
                Some(f64::from(*worked) / f64::from(*tried))
            }
            _ => None,
        }
    }
}

/// When an episode's span is a single point.
impl Span {
    /// One cursor.
    #[must_use]
    pub fn at(cursor: u64) -> Self {
        Self {
            from: cursor,
            to: cursor,
        }
    }

    /// How many cursors this covers.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.to.saturating_sub(self.from) + 1
    }

    /// Whether it covers nothing, which a well-formed span never does.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.to < self.from
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_fact_occupies_a_slot() {
        // Nothing else can contradict, because nothing else claims a slot.
        assert_eq!(
            Body::fact("project", "test_command", "make test").slot(),
            Some(("project", "test_command"))
        );
        assert_eq!(Body::note("hello", NoteKind::Scratch).slot(), None);
        assert_eq!(Body::habit("tests", vec!["make test".into()]).slot(), None);
    }

    #[test]
    fn a_fact_renders_as_a_readable_line() {
        assert_eq!(
            Body::fact("project", "test_command", "make test").text(),
            "project test_command make test"
        );
    }

    #[test]
    fn a_habit_renders_its_steps_in_order() {
        let h = Body::habit(
            "run the tests",
            vec!["make test".into(), "read the failures".into()],
        );
        assert_eq!(h.text(), "run the tests: make test then read the failures");
    }

    #[test]
    fn a_habit_reports_how_well_it_has_worked() {
        let h = Body::habit("tests", vec!["make test".into()]);
        assert_eq!(h.success_rate(), Some(1.0));
        assert_eq!(Body::note("x", NoteKind::Scratch).success_rate(), None);
    }

    #[test]
    fn a_span_of_one_cursor_covers_one() {
        assert_eq!(Span::at(7).len(), 1);
        assert!(!Span::at(7).is_empty());
    }

    #[test]
    fn a_body_round_trips_as_json() {
        // The store keeps this column as JSON, so a variant that will not serialise is a
        // memory that cannot be written.
        let body = Body::episode(
            "did a thing",
            Span { from: 1, to: 9 },
            vec!["shell".into()],
            Outcome::Done,
        );
        let text = serde_json::to_string(&body).expect("encode");
        assert_eq!(serde_json::from_str::<Body>(&text).expect("decode"), body);
    }
}
