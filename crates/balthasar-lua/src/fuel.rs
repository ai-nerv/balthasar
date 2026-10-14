//! The instruction budget every chunk runs under.
//!
//! luna hands control back between slices of an executor, so the host decides how many slices a
//! chunk gets. A chunk that spends its budget is stopped and reported, never left to stall the
//! program that ran it.

use luna::{Fuel, Lua, StashedExecutor};

/// Instructions in one slice, between which the budget and the memory ceiling are checked.
const SLICE: i32 = 4096;

/// The most memory one VM may hold, in bytes. The shipped configuration, client and examples hold
/// 150 KB; a policy over 20,000 rows holds 41 MB; a call at the 16 MiB frame cap peaks at 73 MB.
pub const MEMORY: usize = 256 * 1024 * 1024;

/// How many instructions a chunk may spend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// A whole configuration file.
    pub file: u64,
    /// One hook call.
    pub hook: u64,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            file: 100_000_000,
            hook: 10_000_000,
        }
    }
}

/// Run `executor` until it finishes, spends `fuel`, or outgrows the VM's memory ceiling.
///
/// # Errors
/// Names which limit stopped it, or that it waited on something a configuration cannot.
pub fn drive(lua: &mut Lua, executor: &StashedExecutor, fuel: u64) -> Result<(), String> {
    let mut spent: u64 = 0;
    loop {
        let mut slice = Fuel::with(SLICE);
        let finished = lua
            .enter(|ctx| ctx.fetch(executor).step(ctx, &mut slice))
            .map_err(|why| why.to_string())?;
        if let Some(ceiling) = over_memory(lua) {
            lua.enter(|ctx| ctx.fetch(executor).stop(&ctx));
            return Err(format!(
                "stopped after holding more than its {} MiB of memory",
                ceiling >> 20
            ));
        }
        if finished {
            return Ok(());
        }
        if lua.enter(|ctx| ctx.fetch(executor).is_waiting()) {
            lua.enter(|ctx| ctx.fetch(executor).stop(&ctx));
            return Err("it waited on something a configuration cannot wait on".to_owned());
        }
        let used = i64::from(SLICE) - i64::from(slice.remaining());
        spent = spent.saturating_add(u64::try_from(used.max(1)).unwrap_or(1));
        if spent > fuel {
            lua.enter(|ctx| ctx.fetch(executor).stop(&ctx));
            return Err(format!(
                "stopped after spending its budget of {fuel} instructions"
            ));
        }
    }
}

/// The VM's ceiling, when it still holds more than that after a full collection.
fn over_memory(lua: &mut Lua) -> Option<usize> {
    let ceiling = lua.memory_limit()?;
    if lua.total_memory() <= ceiling {
        return None;
    }
    lua.gc_collect();
    (lua.total_memory() > ceiling).then_some(ceiling)
}

#[cfg(test)]
mod tests {
    use crate::Engine;

    #[test]
    fn a_file_that_loops_is_stopped_and_named() {
        let mut engine = Engine::new();
        engine.budget(1_000_000, 1_000_000);
        let why = engine
            .run("while true do end", "loop.lua")
            .expect_err("a loop must be stopped");
        let said = why.to_string();
        assert!(said.contains("loop.lua"), "{said}");
        assert!(said.contains("budget"), "{said}");
    }

    #[test]
    fn a_loop_inside_pcall_is_stopped_all_the_same() {
        let mut engine = Engine::new();
        engine.budget(1_000_000, 1_000_000);
        assert!(
            engine
                .run("pcall(function() while true do end end)", "caught.lua")
                .is_err()
        );
    }

    #[test]
    fn a_hook_that_loops_is_stopped_and_reported_as_failed() {
        let mut engine = Engine::new();
        engine
            .run(
                "balthasar.on.scope(function(cwd) while true do end end)",
                "init.lua",
            )
            .expect("registering runs");
        engine.budget(1_000_000, 1_000_000);
        assert_eq!(engine.ask("scope", &[serde_json::json!("/w")]), None);
        let log = engine.config().log;
        assert!(
            log.iter().any(|line| line.contains("hook failed")),
            "{log:?}"
        );
    }

    #[test]
    fn the_engine_still_answers_after_a_hook_was_stopped() {
        let mut engine = Engine::new();
        engine
            .run(
                "balthasar.on.scope(function(cwd) if cwd == '/loop' then while true do end end \
                 return { id = 'fine' } end)",
                "init.lua",
            )
            .expect("registering runs");
        engine.budget(1_000_000, 1_000_000);
        assert_eq!(engine.ask("scope", &[serde_json::json!("/loop")]), None);
        assert_eq!(
            engine.ask("scope", &[serde_json::json!("/w")]),
            Some(serde_json::json!({ "id": "fine" }))
        );
    }

    #[test]
    fn every_vm_is_held_to_the_memory_ceiling() {
        assert_eq!(Engine::new().ceiling(), Some(super::MEMORY));
        assert_eq!(Engine::project().ceiling(), Some(super::MEMORY));
    }

    #[test]
    fn the_shipped_configuration_fits_far_inside_the_memory_ceiling() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut engine = Engine::new();
        engine
            .read(&[(root.join("config/init.lua"), true)])
            .expect("loads");
        for plugin in ["shell-history.lua", "build-commands.lua"] {
            let path = root.join("examples/plugin").join(plugin);
            engine.run_file(&path).expect(plugin);
        }
        let client = format!(
            "assert(load({:?}, 'balthasar.lua'))(balthasar.stream)",
            crate::CLIENT
        );
        engine.run(&client, "client.lua").expect("client");
        let held = engine.held();
        assert!(held * 256 < super::MEMORY, "{held} bytes held");
    }

    #[test]
    fn a_call_at_the_frame_cap_fits_inside_the_memory_ceiling() {
        let mut engine = Engine::new();
        engine
            .run(
                "local text = string.rep('x', 15 * 1024 * 1024)\n\
                 local framed = balthasar.json.encode({ call = 'observe', args = { { text = text } } })\n\
                 balthasar.framed = #framed > 15 * 1024 * 1024",
                "frame.lua",
            )
            .expect("a call the wire carries is one a VM may build");
        engine.harvest();
        assert_eq!(engine.config().boolean("framed"), Some(true));
    }

    #[test]
    fn a_file_that_hoards_memory_is_stopped_and_named() {
        for mut engine in [Engine::new(), Engine::project()] {
            engine.hold_to(8 << 20);
            let why = engine
                .run(
                    "local kept = {} for i = 1, 1e7 do kept[i] = ('x'):rep(64) .. i end",
                    "hoard.lua",
                )
                .expect_err("a hoard must be stopped");
            let said = why.to_string();
            assert!(said.contains("hoard.lua"), "{said}");
            assert!(said.contains("8 MiB of memory"), "{said}");
            engine
                .run("local after = 1", "after.lua")
                .expect("the VM runs again once the hoard is gone");
        }
    }

    #[test]
    fn a_hook_that_hoards_memory_fails_alone_and_the_engine_still_answers() {
        let mut engine = Engine::new();
        engine
            .run(
                "balthasar.on.scope(function(cwd) if cwd == '/hoard' then local kept = {} \
                 for i = 1, 1e7 do kept[i] = ('x'):rep(64) .. i end end \
                 return { id = 'fine' } end)",
                "init.lua",
            )
            .expect("registering runs");
        engine.hold_to(8 << 20);
        assert_eq!(engine.ask("scope", &[serde_json::json!("/hoard")]), None);
        let log = engine.config().log;
        assert!(log.iter().any(|line| line.contains("memory")), "{log:?}");
        assert_eq!(
            engine.ask("scope", &[serde_json::json!("/w")]),
            Some(serde_json::json!({ "id": "fine" }))
        );
    }

    #[test]
    fn a_chunk_that_finishes_inside_one_slice_is_held_to_the_ceiling_all_the_same() {
        let mut engine = Engine::new();
        engine.hold_to(8 << 20);
        let why = engine
            .run(
                "balthasar.big = string.rep('x', 16 * 1024 * 1024)",
                "one.lua",
            )
            .expect_err("one allocation over the ceiling");
        assert!(why.to_string().contains("MiB of memory"), "{why}");
    }

    #[test]
    fn garbage_is_collected_before_the_ceiling_is_judged() {
        let mut engine = Engine::new();
        engine.hold_to(8 << 20);
        engine.pause_collector();
        engine
            .run(
                "for i = 1, 50000 do local s = ('x'):rep(1024) .. i end",
                "churn.lua",
            )
            .expect("garbage alone is not held");
    }

    #[test]
    fn ordinary_configuration_fits_the_default_budget() {
        let mut engine = Engine::new();
        engine
            .run(
                "local n = 0 for i = 1, 100000 do n = n + i end balthasar.sum = n",
                "sum.lua",
            )
            .expect("an ordinary loop finishes");
    }
}
