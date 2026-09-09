//! `balthasar needs` and `balthasar configure` — being driven by a coordinator.
//!
//! Both answer in the family's shape — `{"ok":true,"n":N,"result":[…]}` — in JSON or in CBOR.

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

/// Acknowledge the installed packages.
#[derive(Debug, Parser)]
pub struct AcknowledgeArgs {
    /// Answer in JSON. The default, and accepted so every sibling takes the same flags.
    #[arg(long)]
    pub json: bool,
    /// Answer in CBOR rather than JSON.
    #[arg(long)]
    pub cbor: bool,
}

/// Acknowledge the installed packages, so their declarations may run.
///
/// A package under `site/pack/` runs once acknowledged, and stops the moment it changes.
pub fn acknowledge(args: &AcknowledgeArgs) -> anyhow::Result<()> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut out = std::io::stdout().lock();
    match crate::loaded::trust(&cwd) {
        Ok(files) => {
            let taken: Vec<serde_json::Value> = files
                .iter()
                .map(|path| serde_json::json!({ "acknowledged": path.display().to_string() }))
                .collect();
            reply(&mut out, args.cbor, &taken);
        }
        Err(why) => refuse(&mut out, args.cbor, &why),
    }
    Ok(())
}
/// Read config Lua on stdin, apply it, and say what it did.
pub fn configure(args: &ConfigureArgs) -> anyhow::Result<()> {
    let mut out = std::io::stdout().lock();
    if args.forget {
        match setup::forget() {
            Ok(()) => reply(&mut out, args.cbor, &[setup::Applied::default()]),
            Err(why) => refuse(&mut out, args.cbor, &why.to_string()),
        }
        return Ok(());
    }

    let mut source = String::new();
    if let Err(why) = std::io::Read::read_to_string(&mut std::io::stdin().lock(), &mut source) {
        refuse(&mut out, args.cbor, &why.to_string());
        return Ok(());
    }
    match setup::configure(&source) {
        Ok(applied) => reply(&mut out, args.cbor, &[applied]),
        // A chunk that will not run is a refusal, not a crash; the exit stays zero.
        Err(why) => refuse(&mut out, args.cbor, &why.to_string()),
    }
    Ok(())
}

/// The revision of the *registrar* surface — what a third party writes against.
///
/// It goes up when something already published stops working; adding a registrar, a field or a
/// handler does not move it. Reported on `verbs`, beside `family`. See EXTENDING.md.
const SURFACE: u16 = 1;
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
/// A body that will not encode is answered in JSON instead of not at all.
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

/// What `verbs` takes.
#[derive(Debug, Parser)]
pub struct VerbsArgs {
    /// Answer in JSON. The default, and accepted so every sibling takes the same flags.
    #[arg(long)]
    pub json: bool,
    /// Answer in CBOR rather than JSON.
    #[arg(long)]
    pub cbor: bool,
}

/// Every verb this program answers, on each of its doors.
///
/// The command line half is read off clap rather than listed; the socket half is
/// [`balthasar_host::SURFACE`], written out by hand so the file can be read.
pub fn verbs(args: &VerbsArgs) -> anyhow::Result<()> {
    use clap::CommandFactory;

    let mut listed: Vec<serde_json::Value> = crate::Cli::command()
        .get_subcommands()
        .map(|sub| {
            serde_json::json!({
                "verb": sub.get_name(),
                "about": sub.get_about().map(|a| a.to_string()).unwrap_or_default(),
                "door": "cli",
            })
        })
        .collect();
    listed.extend(balthasar_host::SURFACE.iter().map(|verb| {
        serde_json::json!({
            // `name` rides along for one revision because the socket has always answered with it.
            "verb": verb.name,
            "name": verb.name,
            "about": verb.about,
            "writes": verb.writes,
            "door": "socket",
        })
    }));

    let body = serde_json::json!({
        "ok": true,
        "family": balthasar_ipc::FAMILY,
        "surface": SURFACE,
        "n": listed.len(),
        "result": listed,
    });
    let mut out = std::io::stdout().lock();
    emit(&mut out, args.cbor, &body);
    Ok(())
}
