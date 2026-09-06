//! `balthasar needs` and `balthasar configure` — being driven by a coordinator.
//!
//! balthasar reads its own configuration and always will. This is the other way in, for when
//! something is coordinating it: it starts one, asks what it takes, and says. Two
//! configurations that have to agree are one that will not, so under a coordinator there is one.
//!
//! Both answer in the family's shape — `{"ok":true,"n":N,"result":[…]}` — in JSON or in CBOR, so
//! the caller needs no second parser to find out that a call was refused.

use balthasar_lua::setup;
use clap::Parser;
use std::io::Write;

/// What a coordinator may tell this balthasar.
#[derive(Debug, Parser)]
pub struct NeedsArgs {
    /// Answer in JSON. The default, and accepted so every sibling takes the same flags.
    #[arg(long)]
    pub json: bool,
    /// Answer in CBOR rather than JSON.
    #[arg(long)]
    pub cbor: bool,
}

/// Take a chunk of config Lua on stdin.
#[derive(Debug, Parser)]
pub struct ConfigureArgs {
    /// Answer in JSON. The default, and accepted so every sibling takes the same flags.
    #[arg(long)]
    pub json: bool,
    /// Answer in CBOR rather than JSON.
    #[arg(long)]
    pub cbor: bool,
    /// Forget what a coordinator said, rather than adding to it.
    #[arg(long)]
    pub forget: bool,
}

/// Print what balthasar wants to be told.
pub fn needs(args: &NeedsArgs) -> anyhow::Result<()> {
    let mut out = std::io::stdout().lock();
    reply(&mut out, args.cbor, &setup::needs());
    Ok(())
}

/// Read config Lua on stdin, apply it, and say what it did.
pub fn configure(args: &ConfigureArgs) -> anyhow::Result<()> {
    let mut out = std::io::stdout().lock();
    if args.forget {
        setup::forget();
        reply(&mut out, args.cbor, &[setup::Applied::default()]);
        return Ok(());
    }

    let mut source = String::new();
    if let Err(why) = std::io::Read::read_to_string(&mut std::io::stdin().lock(), &mut source) {
        refuse(&mut out, args.cbor, &why.to_string());
        return Ok(());
    }
    match setup::configure(&source) {
        Ok(applied) => reply(&mut out, args.cbor, &[applied]),
        // A chunk that will not run is a refusal, not a crash: the coordinator sent something,
        // and what it needs back is which part was wrong. The exit stays zero, because a
        // non-zero exit is how a program says it did not run.
        Err(why) => refuse(&mut out, args.cbor, &why.to_string()),
    }
    Ok(())
}

fn reply<T: serde::Serialize>(out: &mut impl Write, cbor: bool, values: &[T]) {
    let body = serde_json::json!({
        "ok": true,
        "family": balthasar_ipc::FAMILY,
        "n": values.len(),
        "result": values
            .iter()
            .map(|v| serde_json::to_value(v).unwrap_or(serde_json::Value::Null))
            .collect::<Vec<_>>(),
    });
    emit(out, cbor, &body);
}

fn refuse(out: &mut impl Write, cbor: bool, why: &str) {
    emit(
        out,
        cbor,
        &serde_json::json!({
            "ok": false,
            "family": balthasar_ipc::FAMILY,
            "error": why,
            "fault": "refused",
        }),
    );
}

/// Write one body in the encoding the caller asked for.
///
/// Shared with the one-shot door in `serve.rs`, so that "which encodings does balthasar answer
/// in" has one answer rather than one per subcommand.
///
/// A body that will not encode is answered in JSON instead of not at all. Every body here is a
/// `serde_json::Value` and CBOR can carry all of them, so this is unreachable — but the failing
/// branch used to write *nothing* and exit zero, which a caller cannot tell from a reply it
/// simply did not receive.
pub(crate) fn emit(out: &mut impl Write, cbor: bool, body: &serde_json::Value) {
    if cbor {
        let mut bytes = Vec::new();
        if ciborium::into_writer(body, &mut bytes).is_ok() {
            let _ = out.write_all(&bytes);
            return;
        }
    }
    let _ = writeln!(out, "{body}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_is_the_familys_shape() {
        let mut out = Vec::new();
        reply(&mut out, false, &["a"]);
        let value: serde_json::Value = serde_json::from_slice(&out).expect("decode");
        assert_eq!(value["ok"], serde_json::json!(true));
        assert_eq!(value["n"], serde_json::json!(1));
        assert!(value["result"].is_array(), "result is a list");
    }

    #[test]
    fn a_refusal_is_a_reply_rather_than_an_exit() {
        let mut out = Vec::new();
        refuse(&mut out, false, "no");
        let value: serde_json::Value = serde_json::from_slice(&out).expect("decode");
        assert_eq!(value["ok"], serde_json::json!(false));
        assert_eq!(value["fault"], serde_json::json!("refused"));
    }

    #[test]
    fn a_one_shot_reply_says_which_wire_it_is() {
        // The socket door has carried this since the version check landed; these did not, so
        // the only replies in the family with no version on them were the ones a coordinator
        // reads first. A missing `family` is taken for a peer older than the check, which is
        // exactly the wrong thing to say about the current build.
        let mut out = Vec::new();
        reply(&mut out, false, &["a"]);
        let value: serde_json::Value = serde_json::from_slice(&out).expect("decode");
        assert_eq!(value["family"], serde_json::json!(balthasar_ipc::FAMILY));

        let mut refused = Vec::new();
        refuse(&mut refused, false, "no");
        let value: serde_json::Value = serde_json::from_slice(&refused).expect("decode");
        assert_eq!(
            value["family"],
            serde_json::json!(balthasar_ipc::FAMILY),
            "a refusal too"
        );
    }

    #[test]
    fn both_encodings_carry_the_same_answer() {
        let needs = setup::needs();
        let (mut json, mut cbor) = (Vec::new(), Vec::new());
        reply(&mut json, false, &needs);
        reply(&mut cbor, true, &needs);
        let from_json: serde_json::Value = serde_json::from_slice(&json).expect("json");
        let from_cbor: serde_json::Value = ciborium::from_reader(cbor.as_slice()).expect("cbor");
        assert_eq!(from_json, from_cbor, "one shape, two encodings");
    }
}
