//! What a config cannot reach.
//!
//! `Lua::full()` hands the VM the whole standard library, which includes `os.execute` and
//! `io.popen`. This is the one VM in the family that runs files it did not write — a project's
//! own `.balthasar.lua`, found by walking up from the working directory — so it is the one where
//! that matters most, and it was the only one without this list.
//!
//! **The ordering is why the list is the control.** [`crate::Engine::read`] runs an untrusted file
//! and *then* calls `refuse_declarations`. A check that runs after arbitrary Lua has already
//! executed can report what a file declared; it cannot undo what the file did. Refusing a
//! declaration is about what balthasar believes. This is about what a file can do on the way.
//!
//! Removed rather than never installed, because the alternative is assembling a standard library
//! by hand and quietly missing something the next luna release adds. A short list of what must
//! not be reachable is auditable; a long list of what may be is not.
//!
//! The same list every other Lua VM in the family carries, and deliberately identical in
//! substance: a config refused something here and allowed it there would make the boundary a
//! property of which program happened to read the file rather than of what a config may do.

use luna::{Lua, Value};

/// Globals a config must not have, and why each one is on the list.
///
/// `os.execute` and `io.popen` spawn — and balthasar declares distillers and extractors, which
/// name commands it runs itself through a seam that checks them. `os.remove`, `os.rename` and
/// `os.tmpname` write outside that seam. `os.exit` would let a config file end a serving daemon
/// mid-write. `io` goes wholesale: every remaining member of it opens a file.
const REMOVED: &[(&str, &str)] = &[
    ("os", "execute"),
    ("os", "exit"),
    ("os", "remove"),
    ("os", "rename"),
    ("os", "tmpname"),
    ("os", "setlocale"),
];

/// Globals removed entirely.
const REMOVED_TABLES: &[&str] = &["io", "package", "dofile", "loadfile", "require"];

/// Take away what a config must not be able to do.
pub fn apply(lua: &mut Lua) {
    lua.enter(|ctx| {
        for (table, field) in REMOVED {
            if let Value::Table(t) = ctx.get_global_value(table) {
                t.set(ctx, *field, Value::Nil).ok();
            }
        }
        for name in REMOVED_TABLES {
            ctx.set_global(name, Value::Nil);
        }
    });
}

#[cfg(test)]
mod tests {
    use crate::Engine;

    /// What one expression evaluates to inside a fresh engine.
    fn probe(expression: &str) -> String {
        let mut engine = Engine::new();
        engine
            .run(
                &format!("balthasar.answer = tostring({expression})"),
                "probe.lua",
            )
            .expect("run");
        engine.harvest();
        engine
            .config()
            .string("answer")
            .unwrap_or("<absent>")
            .to_owned()
    }

    #[test]
    fn a_config_cannot_spawn_a_process() {
        // balthasar runs distillers and extractors itself, through a seam that decides whether a
        // command may run. A config that could spawn would go round it.
        assert_eq!(probe("os.execute"), "nil");
        assert_eq!(probe("io"), "nil");
    }

    #[test]
    fn a_config_cannot_write_outside_the_store() {
        for expression in ["os.remove", "os.rename", "os.tmpname"] {
            assert_eq!(probe(expression), "nil", "{expression} is still reachable");
        }
    }

    #[test]
    fn a_config_cannot_end_a_serving_daemon() {
        assert_eq!(probe("os.exit"), "nil");
    }

    #[test]
    fn a_config_cannot_load_arbitrary_files() {
        for expression in ["dofile", "loadfile", "require", "package"] {
            assert_eq!(probe(expression), "nil", "{expression} is still reachable");
        }
    }

    #[test]
    fn what_a_config_legitimately_needs_still_works() {
        // The removals must not cost a config the things it is for. `load` in particular: the
        // family's client stubs are loaded chunks, and removing it would break `lua-api`.
        assert_ne!(probe("os.getenv"), "nil", "reading the environment is fine");
        assert_ne!(probe("os.time"), "nil");
        assert_ne!(
            probe("load"),
            "nil",
            "the family's clients are loaded chunks"
        );
        assert_ne!(probe("string.format"), "nil");
        assert_ne!(probe("table.concat"), "nil");
    }
}
