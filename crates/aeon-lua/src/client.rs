//! The client library other programs load to talk to a running aeon.
//!
//! Plain Lua, and *copied* from the family rather than written: oslo ships `client.lua`, hexe
//! ships `hexe.lua`, and this is the same file with aeon's identity and aeon's verbs. The
//! framing, the reply shape and the discovery order are shared on purpose, so a fix to any of
//! them reaches every sibling.
//!
//! What it cannot do itself is open a socket. That arrives as the chunk's argument:
//!
//! ```lua
//! load(src)(transport)   -- transport.connect(path, timeout_ms) -> handle
//!                        -- handle:send(bytes) / handle:recv(n) / handle:close()
//! ```

/// The client library, as a program receives it from `aeon lua-api`.
pub const CLIENT: &str = include_str!("../lua/aeon.lua");

#[cfg(test)]
mod tests {
    use super::CLIENT;
    use crate::Engine;

    /// Load the client in aeon's own VM and ask it something.
    fn probe(script: &str) -> String {
        let mut engine = Engine::new();
        let source = format!(
            r#"
            local chunk = assert(load({CLIENT:?}, "aeon.lua"))
            local client = chunk(nil)
            aeon.answer = tostring({script})
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
    fn the_stub_loads_in_aeons_own_vm() {
        // The family's claim is that the file is copied, not ported. A client that only ran in
        // the tool that wrote it would make that false the moment a sibling tried it.
        assert_eq!(probe("client._NAME"), "aeon");
    }

    #[test]
    fn the_stub_declares_a_protocol_version() {
        assert_eq!(probe("client._VERSION"), "1");
    }

    #[test]
    fn the_stub_offers_both_verbs() {
        // Two, because a lifetime is not an implementation detail: `connect` is a channel you
        // hold, `fetch` is one question with nothing held.
        assert_eq!(probe("type(client.connect)"), "function");
        assert_eq!(probe("type(client.fetch)"), "function");
    }

    #[test]
    fn the_stub_carries_its_own_json() {
        // It runs inside somebody else's VM, so it cannot reach aeon's. A client that needed
        // the host to have a JSON module would be one most hosts could not load.
        assert_eq!(probe("client._NAME"), "aeon");
        assert!(CLIENT.contains("local function decode"), "no decoder");
        assert!(CLIENT.contains("local function frame"), "no framing");
    }

    #[test]
    fn the_stub_holds_its_connection() {
        // aeon is asked several times per turn, which is not how a control socket that gets
        // polled occasionally is used. Reconnecting per call would pay for a connect it did
        // not need.
        assert!(
            CLIENT.contains("if not self.handle then return nil, \"this connection is closed\""),
            "the stub does not hold a handle"
        );
    }

    #[test]
    fn the_stub_speaks_the_familys_reply_shape() {
        // Two tools disagreeing here fail silently: a client that unpacks reads a bare-value
        // server as having returned nothing at all.
        assert!(CLIENT.contains("reply.result"), "no result list");
        assert!(CLIENT.contains("reply.n"), "no count");
        assert!(CLIENT.contains("table.unpack"), "does not unpack");
    }

    #[test]
    fn the_stub_looks_for_every_sibling_rather_than_only_itself() {
        // A lookup that knew only its own name sends discovery down the `io.popen` path on
        // exactly the hosts that refuse it -- which reads as "nothing is running".
        //
        // The property is tested rather than the names: which siblings exist is the stub's
        // business, and the stub is Lua. Naming one here would put a harness in aeon's Rust,
        // which `gate-independent` is right to refuse however harmless the mention.
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
            line.contains("\"aeon\""),
            "aeon is not in its own family list"
        );
    }

    #[test]
    fn the_surface_carries_nothing_that_causes_work() {
        // A socket that runs commands is remote code execution wearing a friendlier name.
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
