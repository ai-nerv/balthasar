use super::*;

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
fn a_day_is_written_as_a_date() {
    assert_eq!(day(0), "1970-01-01");
    assert_eq!(day(1_756_000_000), "2025-08-24");
    assert_eq!(day(951_782_400), "2000-02-29");
}

#[test]
fn memory_settings_fall_back_to_the_shipped_ones() {
    assert_eq!(Keeping::read(None), Keeping::default());
    let said = json!({ "review": true, "extract_every": 3, "tidy_every": 0 });
    let keeping = Keeping::read(Some(&said));
    assert!(keeping.review);
    assert_eq!(keeping.extract_every, 3);
    assert_eq!(
        keeping.tidy_every, 10,
        "zero would tidy after every extraction"
    );
}
