//! Rows near the frame cap, observed and replayed over the real `serve` socket: one that fits a
//! reply comes back whole, and one that does not comes back cut with its raw record read after it
//! a piece at a time.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

mod serving;

use serving::Serving;

const MIB: usize = 1024 * 1024;

/// Every row of one replay page from `from`.
fn page(serving: &Serving, from: u64) -> Vec<serde_json::Value> {
    let reply = serving.ask(
        "replay",
        serde_json::json!(["s", { "from": from, "bytes": 4 * MIB }]),
    );
    assert_eq!(reply["ok"], true, "{}", reply["error"]);
    reply["result"].as_array().expect("rows").clone()
}

/// The raw record at `cursor`, read back a piece at a time.
fn raw_of(serving: &Serving, cursor: u64) -> String {
    let mut whole = String::new();
    let mut offset = Some(0_u64);
    while let Some(at) = offset {
        let reply = serving.ask(
            "replay",
            serde_json::json!(["s", { "from": cursor, "raw": at, "bytes": 4 * MIB }]),
        );
        assert_eq!(reply["ok"], true, "{}", reply["error"]);
        let piece = &reply["result"][0];
        assert_eq!(piece["offset"], at);
        whole.push_str(piece["raw"].as_str().expect("a piece"));
        offset = piece["next"].as_u64();
    }
    whole
}

/// An observed turn whose text is `text` and whose raw record carries it again, as a harness's does.
fn observed(serving: &Serving, cursor: u64, text: &str) -> serde_json::Value {
    let raw = serde_json::json!({ "entry": { "text": text } });
    let turn = serde_json::json!({
        "cursor": cursor, "role": "tool", "kind": "tool_result", "text": text, "raw": raw,
    });
    let said = serving.ask("observe", serde_json::json!(["s", turn]));
    assert_eq!(said["ok"], true, "{}", said["error"]);
    raw
}

#[test]
fn a_nine_mebibyte_row_comes_back_whole() {
    let serving = Serving::start("nine");
    let raw = observed(&serving, 1, &"a".repeat(9 * MIB / 2));
    observed(&serving, 2, "after");

    let first = page(&serving, 0);
    assert_eq!(first.len(), 1, "a row over the budget is a page of its own");
    assert!(first[0].get("truncated").is_none(), "it fits a frame");
    assert_eq!(first[0]["raw"], raw.to_string());
    assert_eq!(page(&serving, 2)[0]["text"], "after");
}

#[test]
fn a_fifteen_mebibyte_row_is_cut_and_its_raw_record_read_after_it() {
    let serving = Serving::start("fifteen");
    let text = "said \"this\"\n".repeat(15 * MIB / 30);
    let raw = observed(&serving, 1, &text);
    observed(&serving, 2, "after");

    let cut = page(&serving, 1);
    assert_eq!(cut.len(), 1);
    assert_eq!(
        cut[0]["truncated"], true,
        "escaped twice, it is over a frame"
    );
    assert!(cut[0].get("raw").is_none());
    let stored = raw.to_string();
    assert_eq!(cut[0]["raw_bytes"], stored.len());
    assert!(text.starts_with(cut[0]["text"].as_str().expect("text")));
    assert_eq!(
        raw_of(&serving, 1),
        stored,
        "every byte of the record comes back"
    );
    assert_eq!(
        page(&serving, 2)[0]["text"],
        "after",
        "and the stream goes on"
    );

    let none = serving.ask("replay", serde_json::json!(["s", { "from": 7, "raw": 0 }]));
    assert_eq!(
        none["ok"], false,
        "a turn that is not there has no raw record"
    );
}
