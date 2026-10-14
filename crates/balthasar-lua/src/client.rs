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

    /// Where the client would look for a session named `x`, with `env` as the environment and
    /// `host` as the `hexe` global, joined by `|`.
    fn looked_in(env: &str, host: &str) -> String {
        probe(&format!(
            r#"(function()
                local said = {{ {env} }}
                os.getenv = function(name) return said[name] end
                hexe = {host}
                local out = table.concat(client.sockets("x"), "|")
                hexe = nil
                return out
            end)()"#
        ))
    }

    #[test]
    fn the_uid_is_the_hosts_and_never_the_environment() {
        assert_eq!(looked_in(r#"UID = "4242""#, "nil"), "");
        assert_eq!(
            looked_in(r#"UID = "4242""#, "{ uid = 1000 }"),
            "/tmp/memory-1000/api@x.sock|/tmp/balthasar-1000/api@x.sock"
        );
        assert_eq!(
            looked_in("", "{ uid = function() return 7 end }"),
            "/tmp/memory-7/api@x.sock|/tmp/balthasar-7/api@x.sock"
        );
        assert_eq!(looked_in("", r#"{ uid = "1000" }"#), "");
        assert_eq!(looked_in("", "{ uid = 1.5 }"), "");
    }

    #[test]
    fn the_runtime_dir_is_used_only_when_the_host_says_it_is_private() {
        let at = |runtime: &str, stat: &str| {
            let first = looked_in(
                &format!("XDG_RUNTIME_DIR = {runtime:?}"),
                &format!("{{ uid = 1000, fs = {{ stat = {stat} }} }}"),
            );
            first.split('|').next().unwrap_or_default().to_owned()
        };
        let stat = |kind: &str, uid: u32, mode: u32| {
            format!("function() return {{ type = {kind:?}, uid = {uid}, mode = {mode} }} end")
        };
        assert_eq!(
            at("/run/user/1000/", &stat("directory", 1000, 0o700)),
            "/run/user/1000/memory/api@x.sock"
        );
        for (runtime, said) in [
            ("/run/user/1000", stat("directory", 1000, 0o755)),
            ("/run/user/1000", stat("directory", 1001, 0o700)),
            ("/run/user/1000", stat("symlink", 1000, 0o700)),
            ("/run/user/1000", "nil".to_owned()),
            ("run/user", stat("directory", 1000, 0o700)),
            ("", stat("directory", 1000, 0o700)),
        ] {
            assert_eq!(
                at(runtime, &said),
                "/tmp/memory-1000/api@x.sock",
                "{runtime:?} trusted though stat said {said}"
            );
        }
    }

    #[test]
    fn the_fallback_is_under_an_absolute_tmpdir_only() {
        assert_eq!(
            looked_in(r#"TMPDIR = "/scratch/""#, "{ uid = 1000 }"),
            "/scratch/memory-1000/api@x.sock|/scratch/balthasar-1000/api@x.sock"
        );
        assert_eq!(
            looked_in(r#"TMPDIR = "scratch""#, "{ uid = 1000 }"),
            "/tmp/memory-1000/api@x.sock|/tmp/balthasar-1000/api@x.sock"
        );
    }

    /// What `fetch` ran for tool `x`, given the host's uid, what `fs.stat` says of the descriptor
    /// and of its directory, and the program it names; `nothing` when it ran nothing.
    fn ran(uid: &str, file: (&str, u32, u32), dir: (&str, u32, u32), exec: &str) -> String {
        let (file_kind, file_uid, file_mode) = file;
        let (dir_kind, dir_uid, dir_mode) = dir;
        probe(&format!(
            r#"(function()
                os.getenv = function() return nil end
                local ran = "nothing"
                hexe = {{ uid = {uid},
                  run = function(cmd) ran = cmd
                    return {{ ok = true, stdout = '{{"ok":true,"n":1,"result":["x"]}}' }} end,
                  fs = {{ ls = function() return {{}} end,
                          read = function() return '{{"exec":{exec:?}}}' end,
                          stat = function(path)
                            if path == "/tmp/x-1000/x.tool" then
                              return {{ type = {file_kind:?}, uid = {file_uid}, mode = {file_mode} }}
                            elseif path == "/tmp/x-1000" then
                              return {{ type = {dir_kind:?}, uid = {dir_uid}, mode = {dir_mode} }}
                            end
                          end }} }}
                client.fetch({{ tool = "x" }}, "verbs")
                hexe = nil
                return ran
            end)()"#
        ))
    }

    #[test]
    fn a_descriptor_somebody_else_could_write_is_never_executed() {
        let file = ("file", 1000, 0o600);
        let dir = ("directory", 1000, 0o700);
        let exec = "/opt/x/tool";
        assert_eq!(ran("1000", file, dir, exec), "'/opt/x/tool' 'verbs'");
        assert_ne!(ran("1000", ("file", 1000, 0o644), dir, exec), "nothing");

        for (uid, file, dir, exec) in [
            ("1000", ("file", 1000, 0o620), dir, exec),
            ("1000", ("file", 1000, 0o602), dir, exec),
            ("1000", ("file", 1001, 0o600), dir, exec),
            ("1000", ("symlink", 1000, 0o600), dir, exec),
            ("1000", file, ("directory", 1000, 0o777), exec),
            ("1000", file, ("directory", 1001, 0o700), exec),
            ("nil", file, dir, exec),
            ("1000", file, dir, "tool"),
        ] {
            assert_eq!(
                ran(uid, file, dir, exec),
                "nothing",
                "ran with uid {uid}, file {file:?}, dir {dir:?}, exec {exec}"
            );
        }
    }

    #[test]
    fn a_descriptor_is_never_read_without_a_stat_to_check_it() {
        let answer = probe(
            r#"(function()
                os.getenv = function() return nil end
                local ran = "nothing"
                hexe = { uid = 1000, run = function(cmd) ran = cmd return { ok = true } end,
                  fs = { ls = function() return {} end,
                         read = function() return '{"exec":"/opt/x/tool"}' end } }
                client.fetch({ tool = "x" }, "verbs")
                hexe = nil
                return ran
            end)()"#,
        );
        assert_eq!(answer, "nothing");
    }

    #[test]
    fn the_client_reads_no_uid_from_the_environment() {
        assert!(!CLIENT.contains("\"UID\""), "the client names $UID");
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
