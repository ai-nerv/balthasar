//! The client library other programs load to talk to a running balthasar.
//!
//! Plain Lua, copied from the family. It cannot open a socket itself; that arrives as the
//! chunk's argument:
//!
//! ```lua
//! load(src)(transport)   -- transport.connect(path, timeout_ms) -> handle
//!                        -- handle:send(bytes) / handle:recv(n) / handle:close()
//! ```

/// The client library, as a program receives it from `balthasar lua-api`.
pub const CLIENT: &str = include_str!("../lua/balthasar.lua");

#[cfg(test)]
mod tests {
    use super::CLIENT;
    use crate::Engine;

    /// Load the client in balthasar's own VM and ask it something.
    fn probe(script: &str) -> String {
        let mut engine = Engine::new();
        let source = format!(
            r#"
            local chunk = assert(load({CLIENT:?}, "balthasar.lua"))
            local client = chunk(nil)
            balthasar.answer = tostring({script})
            "#
        );
        engine
            .run(&source, "probe.lua")
            .expect("the client must load");
        engine.harvest();
        engine
            .config()
            .string("answer")
            .expect("an answer")
            .to_owned()
    }

    #[test]
    fn the_stub_loads_in_memos_own_vm() {
        assert_eq!(probe("client._NAME"), "balthasar");
    }

    #[test]
    fn the_stub_declares_a_protocol_version() {
        assert_eq!(probe("client._VERSION"), "1");
    }

    #[test]
    fn the_stub_offers_both_verbs() {
        // `connect` is a channel you hold; `fetch` is one question with nothing held.
        assert_eq!(probe("type(client.connect)"), "function");
        assert_eq!(probe("type(client.fetch)"), "function");
    }

    #[test]
    fn the_stub_carries_its_own_json() {
        assert_eq!(probe("client._NAME"), "balthasar");
        assert!(CLIENT.contains("local function decode"), "no decoder");
        assert!(CLIENT.contains("local function frame"), "no framing");
    }

    #[test]
    fn the_stub_holds_its_connection() {
        // balthasar is asked several times per turn; reconnecting per call would pay for a
        // connect it did not need.
        assert!(
            CLIENT.contains("if not self.handle then return nil, \"this connection is closed\""),
            "the stub does not hold a handle"
        );
    }

    #[test]
    fn the_stub_speaks_the_familys_reply_shape() {
        // A client that unpacks reads a bare-value server as having returned nothing at all.
        assert!(CLIENT.contains("reply.result"), "no result list");
        assert!(CLIENT.contains("reply.n"), "no count");
        assert!(CLIENT.contains("table.unpack"), "does not unpack");
    }

    #[test]
    fn the_stub_looks_for_every_sibling_rather_than_only_itself() {
        // A lookup that knew only its own name sends discovery down the `io.popen` path on
        // exactly the hosts that refuse it. The names are read rather than written down here:
        // `gate-independent` refuses a harness named in balthasar's Rust.
        let line = CLIENT
            .lines()
            .find(|line| line.trim_start().starts_with("local HOSTS ="))
            .expect("the stub declares a family list");
        let named = line.matches('"').count() / 2;
        assert!(
            named >= 3,
            "the family list has only {named} entries: {line}"
        );
        assert!(
            line.contains("\"balthasar\""),
            "balthasar is not in its own family list"
        );
    }

    #[test]
    fn the_surface_carries_nothing_that_causes_work() {
        for forbidden in ["\"prompt\"", "\"run\"", "\"eval\"", "\"purge\""] {
            assert!(
                !CLIENT.contains(&format!("  {forbidden},")),
                "{forbidden} is on the surface"
            );
        }
    }

    #[test]
    fn the_surface_is_spelled_out_rather_than_discovered() {
        for verb in [
            "recall", "why", "sessions", "remember", "forget", "verbs", "status",
        ] {
            assert!(CLIENT.contains(&format!("\"{verb}\"")), "{verb} is missing");
        }
    }
}
