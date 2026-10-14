//! Small facts about the job queue that both the jobs and the notes consult: whether a job is still
//! on its way, whether the harness can run a role, where the JSON is in what a model wrote, and
//! how one field of it is set.

use balthasar_model::Timestamp;
use balthasar_store::Turn;
use balthasar_store::{Job, JobState};
use serde_json::Value;

/// Whether a job of `kind` is still on its way.
pub(crate) fn pending(jobs: &[Job], kind: &str, now: Timestamp) -> bool {
    jobs.iter().any(|job| {
        job.kind == kind
            && (job.state == JobState::Queued
                || (job.state == JobState::Issued && !stale(job, now)))
    })
}

/// Whether an issued job has waited longer than it could take.
pub(crate) fn stale(job: &Job, now: Timestamp) -> bool {
    let limit = job
        .spec
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(20_000)
        / 1_000
        + 60;
    job.state == JobState::Issued
        && job
            .issued
            .is_some_and(|at| now.saturating_sub(at) > i64::try_from(limit).unwrap_or(i64::MAX))
}

/// The JSON object in what a model wrote, fences and chatter around it or not.
pub(crate) fn json_in(text: &str) -> Option<Value> {
    let (start, end) = (text.find('{')?, text.rfind('}')?);
    serde_json::from_str(text.get(start..=end)?).ok()
}

/// `value[key]`: the field, or a null when `value` has none.
pub(crate) fn field_of<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or(&Value::Null)
}

/// `value[key] = to` for an object, or for a null that becomes one; anything else is left as it
/// is rather than panicking as `IndexMut` does.
pub(crate) fn set(value: &mut Value, key: &str, to: Value) {
    if value.is_null() {
        *value = Value::Object(serde_json::Map::new());
    }
    if let Value::Object(fields) = value {
        fields.insert(key.to_owned(), to);
    }
}

/// One turn as a helper reads it, with what a configuration withholds taken out.
///
/// A withheld row keeps its place and its cursor and loses its words, so a span stays contiguous
/// and nothing downstream can mistake the gap for a row that was read.
pub(crate) fn guarded(turn: &Turn, limit: usize, hooks: &mut dyn crate::Hooks) -> (String, bool) {
    match hooks.withhold(turn) {
        Some(text) if text == turn.text => (line(turn, limit), false),
        Some(text) => {
            let mut shown = turn.clone();
            shown.text = text;
            shown.stub = None;
            (line(&shown, limit), false)
        }
        None => (format!("[{}] (withheld)\n", turn.cursor), true),
    }
}

/// One turn as a helper reads it.
pub(crate) fn line(turn: &Turn, limit: usize) -> String {
    let who = match (turn.role.as_str(), turn.kind.as_str(), turn.tool.as_deref()) {
        (_, _, Some(tool)) => format!("tool ({tool})"),
        ("user", "from", _) => "another agent".to_owned(),
        ("user", _, _) => "person".to_owned(),
        (role, _, _) => role.to_owned(),
    };
    let text: String = match &turn.stub {
        Some(stub) if turn.text.len() > limit => stub.clone(),
        _ => turn.text.chars().take(limit).collect(),
    };
    format!("[{}] {who}: {text}\n", turn.cursor)
}

/// Whether the harness said it can run jobs for `role`.
pub(crate) fn can_run(helpers: &[String], role: &str) -> bool {
    helpers.iter().any(|h| h == role)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_models_json_is_found_inside_its_chatter() {
        let said = "Sure!\n```json\n{\"chosen\": [\"m-1\"], \"notes\": []}\n```";
        assert_eq!(json_in(said).expect("json")["chosen"][0], "m-1");
        assert!(json_in("no json here").is_none());
    }

    #[test]
    fn a_helper_is_told_who_said_each_line() {
        let said = |role: &str, kind: &str, tool: Option<&str>| {
            let turn = Turn {
                cursor: 4,
                role: role.into(),
                kind: kind.into(),
                text: "we use uv".into(),
                tool: tool.map(str::to_owned),
                ..Turn::default()
            };
            line(&turn, 100)
        };
        assert_eq!(said("user", "user", None), "[4] person: we use uv\n");
        assert_eq!(said("user", "from", None), "[4] another agent: we use uv\n");
        assert_eq!(
            said("assistant", "prose", None),
            "[4] assistant: we use uv\n"
        );
        assert_eq!(
            said("tool", "tool_result", Some("shell")),
            "[4] tool (shell): we use uv\n"
        );
    }

    #[test]
    fn a_role_runs_only_when_the_harness_named_it() {
        let named = ["memory".to_owned()];
        assert!(can_run(&named, "memory"));
        assert!(!can_run(&named, "safety"));
    }

    #[test]
    fn setting_a_field_on_something_that_is_not_an_object_does_not_panic() {
        let mut object = serde_json::json!({"a": 1});
        set(&mut object, "b", Value::from(2));
        assert_eq!(object, serde_json::json!({"a": 1, "b": 2}));

        let mut null = Value::Null;
        set(&mut null, "b", Value::from(2));
        assert_eq!(null, serde_json::json!({"b": 2}));

        let mut list = serde_json::json!([1]);
        set(&mut list, "b", Value::from(2));
        assert_eq!(list, serde_json::json!([1]));
    }
}
