//! The instruction budget every chunk runs under.
//!
//! luna hands control back between slices of an executor, so the host decides how many slices a
//! chunk gets. A chunk that spends its budget is stopped and reported, never left to stall the
//! program that ran it.

use luna::{Fuel, Lua, StashedExecutor};

/// Instructions in one slice, between which the budget and the memory ceiling are checked.
const SLICE: i32 = 4096;

/// The most memory one VM may hold, in bytes.
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

/// Run `executor` until it finishes, spends `fuel`, or outgrows [`MEMORY`].
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
        lua.run_finalizers();
        if lua.total_memory() > MEMORY {
            lua.gc_collect();
            if lua.total_memory() > MEMORY {
                lua.enter(|ctx| ctx.fetch(executor).stop(&ctx));
                return Err(format!("stopped after holding more than {MEMORY} bytes"));
            }
        }
    }
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
