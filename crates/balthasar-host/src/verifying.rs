//! Claims of success a summary makes that the turns it covers do not back.

use balthasar_store::Turn;

/// What a summary says when it claims a check passed.
const CLAIMS: &[&str] = &[
    "tests pass",
    "tests passed",
    "test passes",
    "build succeeds",
    "build succeeded",
    "builds cleanly",
    "all green",
    "verified",
    "confirmed",
    "works now",
    "is fixed",
    "now passes",
];

/// What a tool call that tests or builds names.
const CHECKS: &[&str] = &[
    "test", "build", "check", "clippy", "verify", "pytest", "jest", "tsc", "lint",
];

/// How a line this adds begins, so a later summary of this one can leave it out.
pub(crate) const PREFIX: &str = "Unverified: the summary says";

/// The most claims one summary is flagged for, and how much of each is quoted.
const MOST: usize = 5;
const QUOTED: usize = 160;

/// A line for every success claim in `summary` that the last test or build in `rows` does not
/// back: none ran, or the last one failed.
pub(crate) fn unverified(summary: &str, rows: &[Turn]) -> Vec<String> {
    let last = rows
        .iter()
        .rev()
        .find(|turn| turn.is_tool() && checks(turn));
    if last.is_some_and(|turn| !turn.error && turn.ok != Some(false)) {
        return Vec::new();
    }
    summary
        .split(['.', '!', ';', '\n'])
        .map(str::trim)
        .filter(|sentence| {
            let lower = sentence.to_lowercase();
            CLAIMS.iter().any(|claim| lower.contains(claim))
        })
        .take(MOST)
        .map(|sentence| {
            let quoted: String = sentence.chars().take(QUOTED).collect();
            format!("{PREFIX} \"{quoted}\" but no test or build in this span ended ok.")
        })
        .collect()
}

/// Whether a tool call tests or builds something.
fn checks(turn: &Turn) -> bool {
    let named = format!(
        "{} {}",
        turn.tool.as_deref().unwrap_or_default(),
        turn.args.as_deref().unwrap_or_default()
    )
    .to_lowercase();
    CHECKS.iter().any(|word| named.contains(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ran(command: &str, failed: bool) -> Turn {
        Turn {
            role: "tool".to_owned(),
            kind: "tool_result".to_owned(),
            tool: Some("shell".to_owned()),
            args: Some(format!("{{\"command\":\"{command}\"}}")),
            error: failed,
            ..Turn::default()
        }
    }

    const SUMMARY: &str = "Fixed the parser. The tests pass now. Next is the docs.";

    #[test]
    fn a_claim_backed_by_an_ok_run_is_left_alone() {
        assert!(unverified(SUMMARY, &[ran("cargo test", false)]).is_empty());
    }

    #[test]
    fn a_claim_after_a_killed_run_is_marked() {
        let rows = [ran("cargo test", false), ran("cargo test", true)];
        let said = unverified(SUMMARY, &rows);
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].contains("The tests pass now"), "{said:?}");
    }

    #[test]
    fn a_claim_with_no_run_is_marked() {
        assert_eq!(unverified(SUMMARY, &[ran("ls", false)]).len(), 1);
        assert!(unverified("Read the code.", &[]).is_empty());
    }
}
