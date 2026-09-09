//! What a harness's transcript looks like once balthasar has it.
//!
//! balthasar defines this shape and a harness converts to it, in Lua.

use balthasar_model::Timestamp;
use serde::{Deserialize, Serialize};

/// Who said it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    #[default]
    User,
    Assistant,
    Tool,
    /// Somebody else — a sibling session, or a role this build does not know.
    ///
    /// Unknown roles land here rather than on [`Role::User`], whose turns mint the strongest
    /// witness there is.
    Other,
}

/// What kind of turn it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    #[default]
    Prose,
    Thinking,
    ToolCall,
    ToolResult,
    /// A summary standing in for turns that left the window.
    Summary,
    /// The person's own turn, where a harness names it rather than leaving it prose.
    User,
    /// Another session speaking, carried into this one.
    From,
    /// A branch point — a count of what it keeps, not something anybody said.
    Branch,
}

impl Kind {
    /// Whether a turn of this kind can carry an instruction from the person.
    #[must_use]
    pub fn can_instruct(self) -> bool {
        !matches!(self, Self::From | Self::Branch)
    }
}

/// One turn, as balthasar sees it.
///
/// Every field but `text` is optional, so a harness that cannot supply one leaves it out.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    /// Where in the transcript, so evidence can point at it.
    #[serde(default)]
    pub cursor: Option<u64>,
    #[serde(default)]
    pub role: Role,
    #[serde(default)]
    pub kind: Kind,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub args: Option<serde_json::Value>,
    /// Whether the tool succeeded. The cost signal rides in on this.
    #[serde(default)]
    pub ok: Option<bool>,
    #[serde(default)]
    pub ms: Option<u64>,
    #[serde(default)]
    pub tokens: Option<u64>,
    #[serde(default)]
    pub at: Option<Timestamp>,
}

impl Observation {
    #[must_use]
    pub fn failed(&self) -> bool {
        self.role == Role::Tool && self.ok == Some(false)
    }

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
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub opened: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_with_only_text_is_a_valid_turn() {
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
