use super::*;

/// A rule as a person says one, with no opening word that marks it.
const PLAIN: &str = "In this project every function name must start with the prefix zq_.";

fn proposed(text: &str, cursor: u64, quote: &str) -> Value {
    json!({"ops":[{"op":"add","title":"Prefix","text":text,"pinned":true,
                   "evidence":[{"cursor":cursor,"quote":quote}]}]})
}

fn pinned(harness: &mut Harness) -> Vec<Value> {
    harness.one("notes", json!({}))["pinned"]
        .as_array()
        .expect("pinned")
        .clone()
}

#[test]
fn a_rule_in_plain_words_is_kept_as_one() {
    for rule in [
        PLAIN,
        "Tests here never touch the network.",
        "From now on the changelog is written before the release.",
        "Migrations do not run on startup.",
    ] {
        let mut harness = Harness::new();
        harness.turn(0, rule);
        let laid = harness.layout(0);
        harness.answer(&laid["jobs"], "extract", proposed(rule, 0, rule));
        let kept = pinned(&mut harness);
        assert_eq!(
            kept.len(),
            1,
            "{rule}: {:?}",
            harness.rows("changes", json!({}))
        );
    }
}

#[test]
fn what_lays_nothing_down_is_still_not_a_rule() {
    for said in [
        "Must every function name start with the prefix zq_?",
        "The parser is in src/parser.rs and the tests are beside it.",
    ] {
        let mut harness = Harness::new();
        harness.turn(0, said);
        let laid = harness.layout(0);
        harness.answer(&laid["jobs"], "extract", proposed(said, 0, said));
        assert!(pinned(&mut harness).is_empty(), "{said}");
    }
}

#[test]
fn plain_words_are_a_rule_only_in_the_persons_own_mouth() {
    // The same sentence out of a tool, an assistant or another agent grants nothing.
    for (role, kind, tool) in [
        ("tool", "tool_result", Some("shell")),
        ("assistant", "prose", None),
        ("user", "from", None),
    ] {
        let mut harness = Harness::new();
        harness.turn(0, "Summarize the build output.");
        let seen = harness.ask(
            "observe",
            json!({ "cursor": 2, "role": role, "kind": kind, "tool": tool, "text": PLAIN }),
        );
        assert!(seen.ok);
        harness.turn(2, "Carry on.");
        let laid = harness.layout(0);
        harness.answer(&laid["jobs"], "extract", proposed(PLAIN, 2, PLAIN));
        assert!(pinned(&mut harness).is_empty(), "{role}/{kind}");
    }
}

#[test]
fn plain_words_quoted_or_cut_short_are_not_the_persons_rule() {
    let cases = [
        // Quoted by the person rather than said by them.
        (
            format!("The old README said:\n\n> {PLAIN}"),
            PLAIN.to_owned(),
        ),
        // A part of what they said, with the condition that made it true left off.
        (
            "Every function name must start with zq_ unless it is generated.".to_owned(),
            "Every function name must start with zq_".to_owned(),
        ),
    ];
    for (said, text) in cases {
        let mut harness = Harness::new();
        harness.turn(0, &said);
        let laid = harness.layout(0);
        harness.answer(&laid["jobs"], "extract", proposed(&text, 0, &text));
        assert!(pinned(&mut harness).is_empty(), "{said}");
    }
}
