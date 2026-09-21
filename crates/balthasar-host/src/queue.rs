//! Small facts about the job queue that both the jobs and the notes consult: whether a job is still
//! on its way, whether the harness can run a role, and where the JSON is in what a model wrote.

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
    let limit = job.spec["timeout_ms"].as_u64().unwrap_or(20_000) / 1_000 + 60;
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
}
