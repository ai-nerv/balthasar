//! What a configuration is lent.
//!
//! Small and deliberate. A config is a program, but it is somebody else's program running in
//! balthasar's process, so it gets the handful of things its handlers actually need and not a
//! standard library.

use crate::convert;
use luna::{Callback, CallbackReturn, Table, Value};
use std::cell::RefCell;
use std::rc::Rc;

/// Anything the config passed to `balthasar.log`.
pub type Logged = Rc<RefCell<Vec<String>>>;

/// Install `balthasar.json`, `balthasar.path`, `balthasar.git`, `balthasar.text`, `balthasar.log` and the few constants.
pub fn install<'gc>(ctx: luna::Context<'gc>, balthasar: Table<'gc>, logged: &Logged) {
    balthasar.set(ctx, "json", json_table(ctx)).ok();
    balthasar.set(ctx, "path", path_table(ctx)).ok();
    balthasar.set(ctx, "git", git_table(ctx)).ok();
    balthasar.set(ctx, "text", text_table(ctx)).ok();
    balthasar.set(ctx, "fs", fs_table(ctx)).ok();

    let held = Rc::clone(logged);
    let log = Callback::from_fn(&ctx, move |ctx, _exec, mut stack| {
        let message: Value = stack.consume(ctx)?;
        let text = match message {
            Value::String(s) => String::from_utf8_lossy(s.as_bytes()).into_owned(),
            other => {
                convert::to_json(ctx, other, 0).map_or_else(|| "?".to_owned(), |j| j.to_string())
            }
        };
        held.borrow_mut().push(text);
        stack.replace(ctx, ());
        Ok(CallbackReturn::Return)
    });
    balthasar.set(ctx, "log", log).ok();

    balthasar
        .set(ctx, "looks_like_secret", secret_check(ctx))
        .ok();

    for (name, value) in [
        ("home", std::env::var("HOME").unwrap_or_default()),
        ("data_home", data_home()),
        ("config_home", config_home()),
    ] {
        let text = luna::String::from_slice(&ctx, value.as_bytes());
        balthasar.set(ctx, name, text).ok();
    }
}

/// `balthasar.json.encode` and `balthasar.json.decode`, so a source adapter does not carry a parser.
fn json_table<'gc>(ctx: luna::Context<'gc>) -> Table<'gc> {
    let table = Table::new(&ctx);

    let encode = Callback::from_fn(&ctx, |ctx, _exec, mut stack| {
        let value: Value = stack.consume(ctx)?;
        let Some(json) = convert::to_json(ctx, value, 0) else {
            return Err(convert::raise(
                ctx,
                "balthasar.json.encode: cannot be written down",
            ));
        };
        let text = luna::String::from_slice(&ctx, json.to_string().as_bytes());
        stack.replace(ctx, text);
        Ok(CallbackReturn::Return)
    });
    table.set(ctx, "encode", encode).ok();

    let decode = Callback::from_fn(&ctx, |ctx, _exec, mut stack| {
        let value: Value = stack.consume(ctx)?;
        let Value::String(text) = value else {
            return Err(convert::raise(
                ctx,
                "balthasar.json.decode: expects a string",
            ));
        };
        // A line that will not parse answers nil rather than raising: a source adapter walks
        // somebody else's file, and one bad line should cost that line and not the ingest.
        match serde_json::from_slice::<serde_json::Value>(text.as_bytes()) {
            Ok(json) => stack.replace(ctx, convert::from_json(ctx, &json)),
            Err(_) => stack.replace(ctx, Value::Nil),
        }
        Ok(CallbackReturn::Return)
    });
    table.set(ctx, "decode", decode).ok();
    table
}

/// `balthasar.path.basename` and `balthasar.path.join`.
fn path_table<'gc>(ctx: luna::Context<'gc>) -> Table<'gc> {
    let table = Table::new(&ctx);

    let basename = Callback::from_fn(&ctx, |ctx, _exec, mut stack| {
        let value: Value = stack.consume(ctx)?;
        let Value::String(text) = value else {
            return Err(convert::raise(
                ctx,
                "balthasar.path.basename: expects a path",
            ));
        };
        let path = String::from_utf8_lossy(text.as_bytes()).into_owned();
        let leaf = std::path::Path::new(&path)
            .file_name()
            .map_or(path.clone(), |n| n.to_string_lossy().into_owned());
        stack.replace(ctx, luna::String::from_slice(&ctx, leaf.as_bytes()));
        Ok(CallbackReturn::Return)
    });
    table.set(ctx, "basename", basename).ok();
    table
}

/// `balthasar.git.common_dir`, so a scope handler can put every worktree in one memory.
fn git_table<'gc>(ctx: luna::Context<'gc>) -> Table<'gc> {
    let table = Table::new(&ctx);

    let common = Callback::from_fn(&ctx, |ctx, _exec, mut stack| {
        let value: Value = stack.consume(ctx)?;
        let Value::String(text) = value else {
            return Err(convert::raise(
                ctx,
                "balthasar.git.common_dir: expects a path",
            ));
        };
        let path = String::from_utf8_lossy(text.as_bytes()).into_owned();
        match common_dir(std::path::Path::new(&path)) {
            Some(root) => {
                let found = root.to_string_lossy().into_owned();
                stack.replace(ctx, luna::String::from_slice(&ctx, found.as_bytes()));
            }
            None => stack.replace(ctx, Value::Nil),
        }
        Ok(CallbackReturn::Return)
    });
    table.set(ctx, "common_dir", common).ok();
    table
}

/// `balthasar.text.tail`, for a mask handler trimming a wall of output to its last few lines.
fn text_table<'gc>(ctx: luna::Context<'gc>) -> Table<'gc> {
    let table = Table::new(&ctx);

    let tail = Callback::from_fn(&ctx, |ctx, _exec, mut stack| {
        let (value, count): (Value, Value) = stack.consume(ctx)?;
        let Value::String(text) = value else {
            return Err(convert::raise(ctx, "balthasar.text.tail: expects a string"));
        };
        let wanted = match count {
            Value::Integer(n) if n > 0 => n as usize,
            Value::Number(n) if n > 0.0 => n as usize,
            _ => 20,
        };
        let body = String::from_utf8_lossy(text.as_bytes()).into_owned();
        let lines: Vec<&str> = body.lines().collect();
        let kept = lines[lines.len().saturating_sub(wanted)..].join("\n");
        stack.replace(ctx, luna::String::from_slice(&ctx, kept.as_bytes()));
        Ok(CallbackReturn::Return)
    });
    table.set(ctx, "tail", tail).ok();
    table
}

/// `balthasar.fs.glob`, `balthasar.fs.read` and `balthasar.fs.ls`.
///
/// Read-only, and deliberately so. A source adapter walks somebody else's transcripts; it has
/// no reason to write, and a config that could would be a config that could be handed a path.
fn fs_table<'gc>(ctx: luna::Context<'gc>) -> Table<'gc> {
    let table = Table::new(&ctx);

    let glob = Callback::from_fn(&ctx, |ctx, _exec, mut stack| {
        let value: Value = stack.consume(ctx)?;
        let Value::String(text) = value else {
            return Err(convert::raise(ctx, "balthasar.fs.glob: expects a pattern"));
        };
        let pattern = String::from_utf8_lossy(text.as_bytes()).into_owned();
        let found = Table::new(&ctx);
        for (index, path) in glob_paths(&pattern).into_iter().enumerate() {
            let entry = luna::String::from_slice(&ctx, path.as_bytes());
            found.set(ctx, index as i64 + 1, entry).ok();
        }
        stack.replace(ctx, found);
        Ok(CallbackReturn::Return)
    });
    table.set(ctx, "glob", glob).ok();

    let read = Callback::from_fn(&ctx, |ctx, _exec, mut stack| {
        let value: Value = stack.consume(ctx)?;
        let Value::String(text) = value else {
            return Err(convert::raise(ctx, "balthasar.fs.read: expects a path"));
        };
        let path = String::from_utf8_lossy(text.as_bytes()).into_owned();
        match std::fs::read_to_string(&path) {
            Ok(body) => stack.replace(ctx, luna::String::from_slice(&ctx, body.as_bytes())),
            Err(_) => stack.replace(ctx, Value::Nil),
        }
        Ok(CallbackReturn::Return)
    });
    table.set(ctx, "read", read).ok();
    table
}

/// Every path matching a pattern of the form `dir/prefix*suffix`.
///
/// One wildcard, in the last component. A full glob engine is a dependency, and every pattern a
/// source adapter has needed so far is a directory and an extension.
#[must_use]
pub fn glob_paths(pattern: &str) -> Vec<String> {
    let Some((dir, leaf)) = pattern.rsplit_once('/') else {
        return Vec::new();
    };
    let (prefix, suffix) = match leaf.split_once('*') {
        Some(split) => split,
        // No wildcard: it names one file, and it either exists or it does not.
        None => {
            return if std::path::Path::new(pattern).is_file() {
                vec![pattern.to_owned()]
            } else {
                Vec::new()
            };
        }
    };

    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            (name.starts_with(prefix)
                && name.ends_with(suffix)
                && name.len() >= prefix.len() + suffix.len())
            .then(|| e.path().to_string_lossy().into_owned())
        })
        .collect();
    // Sorted, so an ingest reads sessions in a stable order and a report is reproducible.
    found.sort();
    found
}

/// `balthasar.looks_like_secret`, so the shipped gate does not need a regex engine in Lua.
fn secret_check<'gc>(ctx: luna::Context<'gc>) -> Callback<'gc> {
    Callback::from_fn(&ctx, |ctx, _exec, mut stack| {
        let value: Value = stack.consume(ctx)?;
        let Value::String(text) = value else {
            stack.replace(ctx, false);
            return Ok(CallbackReturn::Return);
        };
        let body = String::from_utf8_lossy(text.as_bytes());
        stack.replace(ctx, looks_like_secret(&body));
        Ok(CallbackReturn::Return)
    })
}

/// Whether a string is probably a credential.
///
/// Deliberately crude and deliberately eager. A false positive costs one memory that could have
/// been remembered; a false negative puts a key in a store that never deletes.
#[must_use]
pub fn looks_like_secret(text: &str) -> bool {
    const MARKERS: &[&str] = &[
        "PRIVATE KEY",
        "BEGIN OPENSSH",
        "aws_secret_access_key",
        "Authorization: Bearer",
    ];
    if MARKERS.iter().any(|m| text.contains(m)) {
        return true;
    }
    // A long unbroken run of key-shaped characters after a known prefix.
    for prefix in ["sk-", "ghp_", "gho_", "xoxb-", "AKIA"] {
        if let Some(at) = text.find(prefix) {
            let rest = &text[at + prefix.len()..];
            let run = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .count();
            if run >= 16 {
                return true;
            }
        }
    }
    false
}

/// The repository a directory is in, following a worktree's pointer file.
fn common_dir(from: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut at = from;
    loop {
        let dot = at.join(".git");
        if dot.is_dir() {
            return Some(at.to_owned());
        }
        if dot.is_file() {
            let pointer = std::fs::read_to_string(&dot).ok()?;
            let target = pointer.trim().strip_prefix("gitdir:")?.trim();
            return std::path::PathBuf::from(target)
                .ancestors()
                .find(|a| a.file_name().is_some_and(|n| n == ".git"))
                .and_then(std::path::Path::parent)
                .map(std::path::Path::to_owned)
                .or_else(|| Some(at.to_owned()));
        }
        at = at.parent()?;
    }
}

/// `$XDG_DATA_HOME`, or the usual place.
#[must_use]
pub fn data_home() -> String {
    std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| format!("{}/.local/share", std::env::var("HOME").unwrap_or_default()))
}

/// `$XDG_CONFIG_HOME`, or the usual place.
#[must_use]
pub fn config_home() -> String {
    std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| format!("{}/.config", std::env::var("HOME").unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_recognised_by_its_shape() {
        assert!(looks_like_secret(
            "export TOKEN=sk-abcdefghijklmnopqrstuvwxyz"
        ));
        assert!(looks_like_secret("-----BEGIN OPENSSH PRIVATE KEY-----"));
        assert!(looks_like_secret("ghp_0123456789abcdefghij"));
    }

    #[test]
    fn a_glob_matches_by_prefix_and_extension() {
        let dir = std::env::temp_dir().join("balthasar-glob-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        for name in ["a.jsonl", "b.jsonl", "notes.md"] {
            std::fs::write(dir.join(name), "x").expect("write");
        }
        let found = glob_paths(&format!("{}/*.jsonl", dir.display()));
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(
            found[0].ends_with("a.jsonl"),
            "sorted, so a report is reproducible"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_glob_over_a_directory_that_is_not_there_finds_nothing() {
        // A fresh install has no transcripts. That is not an error.
        assert!(glob_paths("/no/such/place/*.jsonl").is_empty());
    }

    #[test]
    fn ordinary_prose_is_not_a_key() {
        // A false positive costs one memory. This still has to be usable.
        assert!(!looks_like_secret("we deploy with fly, never heroku"));
        assert!(!looks_like_secret("the sk- prefix means a secret key"));
        assert!(!looks_like_secret("run make test before pushing"));
    }
}
