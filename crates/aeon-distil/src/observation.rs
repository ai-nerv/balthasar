//! What a harness's transcript looks like once aeon has it.
//!
//! aeon defines this shape and a harness converts to it, in Lua. That is the whole of the
//! independence commitment: the moment this crate knew one harness's own record type it would
//! be a component of that harness wearing a socket.

use aeon_model::Timestamp;
use serde::{Deserialize, Serialize};

/// Who said it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The person.
    #[default]
    User,
    /// The model.
    Assistant,
    /// A tool answering.
    Tool,
}

/// What kind of turn it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Ordinary text.
    #[default]
    Prose,
    /// The model's reasoning.
    Thinking,
    /// A tool being asked for.
    ToolCall,
    /// A tool's answer.
    ToolResult,
    /// A summary standing in for turns that left the window.
    Summary,
}

/// One turn, as aeon sees it.
///
/// Every field but `text` is optional, because a harness that cannot supply one should be able
/// to leave it out rather than invent it. An adapter that lies about a duration is worse than
/// one that says nothing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    /// Where in the transcript, so evidence can point at it.
    #[serde(default)]
    pub cursor: Option<u64>,
    /// Who said it.
    #[serde(default)]
    pub role: Role,
    /// What kind.
    #[serde(default)]
    pub kind: Kind,
    /// What was said, or what the tool answered.
    #[serde(default)]
    pub text: String,
    /// Which tool, when this is a call or its result.
    #[serde(default)]
    pub tool: Option<String>,
    /// What the tool was asked for.
    #[serde(default)]
    pub args: Option<serde_json::Value>,
    /// Whether the tool succeeded. The cost signal rides in on this.
    #[serde(default)]
    pub ok: Option<bool>,
    /// How long it took, when the harness records that.
    #[serde(default)]
    pub ms: Option<u64>,
    /// What the turn cost in tokens.
    #[serde(default)]
    pub tokens: Option<u64>,
    /// When it happened, when the harness records that.
    #[serde(default)]
    pub at: Option<Timestamp>,
}

impl Observation {
    /// Whether this is a tool that failed.
    #[must_use]
    pub fn failed(&self) -> bool {
        self.role == Role::Tool && self.ok == Some(false)
    }

    /// Whether this is a tool that worked.
    #[must_use]
    pub fn worked(&self) -> bool {
        self.role == Role::Tool && self.ok == Some(true)
    }

    /// The command a shell-shaped call ran, when it can be told.
    #[must_use]
    pub fn command(&self) -> Option<&str> {
        self.args.as_ref()?.get("command")?.as_str()
    }

    /// The path a file-shaped call touched, when it can be told.
    #[must_use]
    pub fn path(&self) -> Option<&str> {
        self.args.as_ref()?.get("path")?.as_str()
    }
}

/// What a source said about one session before its turns arrived.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    /// The harness's own identity for the session.
    pub id: String,
    /// Where it was run.
    #[serde(default)]
    pub cwd: String,
    /// When it started.
    #[serde(default)]
    pub opened: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_with_only_text_is_a_valid_turn() {
        // Every field but `text` is optional on purpose: an adapter that invents a duration
        // is worse than one that says nothing.
        let observation: Observation = serde_json::from_str(r#"{"text":"hello"}"#).expect("decode");
        assert_eq!(observation.text, "hello");
        assert_eq!(observation.role, Role::User);
        assert_eq!(observation.ms, None);
    }

    #[test]
    fn a_failed_tool_is_told_from_one_that_worked() {
        let failed = Observation {
            role: Role::Tool,
            ok: Some(false),
            ..Observation::default()
        };
        assert!(failed.failed() && !failed.worked());
    }

    #[test]
    fn a_tool_that_did_not_say_is_neither() {
        // `ok` absent means the harness does not record it, which is not the same as failure.
        let quiet = Observation {
            role: Role::Tool,
            ..Observation::default()
        };
        assert!(!quiet.failed() && !quiet.worked());
    }

    #[test]
    fn a_command_is_read_out_of_the_arguments() {
        let call = Observation {
            args: Some(serde_json::json!({ "command": "make test" })),
            ..Observation::default()
        };
        assert_eq!(call.command(), Some("make test"));
        assert_eq!(Observation::default().command(), None);
    }
}
