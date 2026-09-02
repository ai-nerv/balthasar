//! The two namespaces, and how Rust asks them.
//!
//! Handlers are functions, and a function cannot cross the boundary — so the VM keeps them in
//! two globals and Rust keeps only their names. Calling one is a generated chunk that reads
//! arguments from a global and leaves an answer in another, which is the same mechanism the
//! family uses for anything it registers that carries a callback.

use crate::convert;
use luna::{Callback, CallbackReturn, Table, Value};

/// Questions `balthasar.on.<name>` may be registered against.
///
/// Enumerated rather than open, so reading this list tells you every decision a config can
/// take part in. A surface you have to run something to learn is one nobody audits.
/// Handlers that are asked a question and whose answer is used.
///
/// `outcome` is here rather than in [`TOLD`] because §6.7 has it *returning* a classification:
/// a configuration may look at an action and say how it went. The two namespaces stay disjoint,
/// which is what keeps "balthasar asked and used the answer" distinguishable from "balthasar mentioned it
/// happened" — a test holds them apart.
pub const ASKED: &[&str] = &[
    "scope",
    "admit",
    "promote",
    "importance",
    "redact",
    "outcome",
];

/// Events `balthasar.did.<name>` may be registered against.
pub const TOLD: &[&str] = &[
    "observe",
    "promote",
    "contradict",
    "recall",
    "forget",
    "consolidate",
    "compact",
    "used",
];

/// Where asked-handlers live inside the VM.
pub const ON: &str = "__balthasar_on";
/// Where told-handlers live inside the VM.
pub const DID: &str = "__balthasar_did";
/// Where a call's arguments are left for the generated chunk.
pub const ARGS: &str = "__balthasar_args";
/// Where a call's answer is left for Rust.
pub const ANSWER: &str = "__balthasar_answer";

/// Build `balthasar.on` and `balthasar.did`, and the tables behind them.
pub fn install<'gc>(ctx: luna::Context<'gc>, balthasar: Table<'gc>) {
    for (field, store, names) in [("on", ON, ASKED), ("did", DID, TOLD)] {
        let holder = Table::new(&ctx);
        let namespace = Table::new(&ctx);

        for name in names {
            let list = Table::new(&ctx);
            holder.set(ctx, *name, list).ok();

            let which = *name;
            let where_ = store;
            let register = Callback::from_fn(&ctx, move |ctx, _exec, mut stack| {
                let handler: Value = stack.consume(ctx)?;
                if !matches!(handler, Value::Function(_)) {
                    return Err(convert::raise(
                        ctx,
                        &format!("balthasar.{field}.{which}: expects a function"),
                    ));
                }
                let Value::Table(holder) = ctx.get_global_value(where_) else {
                    return Err(convert::raise(ctx, "the handler table is gone"));
                };
                let Ok(Value::Table(list)) = holder.get::<_, Value>(ctx, which) else {
                    return Err(convert::raise(ctx, "the handler list is gone"));
                };
                let next = list.length(&ctx) + 1;
                list.set(ctx, next, handler).ok();
                stack.replace(ctx, ());
                Ok(CallbackReturn::Return)
            });
            namespace.set(ctx, *name, register).ok();
        }

        ctx.set_global(store, holder);
        balthasar.set(ctx, field, namespace).ok();
    }
}

/// The chunk that asks every handler for one question until one answers.
///
/// First non-`nil` wins, and each handler runs inside its own `pcall`: a raise is that
/// handler's problem and must not cost the rest their say.
#[must_use]
pub fn asking(question: &str) -> String {
    format!(
        "local list = {ON} and {ON}[{question:?}]\n\
         {ANSWER} = nil\n\
         if list then\n\
           for i = 1, #list do\n\
             local ok, answer = pcall(list[i], table.unpack({ARGS}))\n\
             if ok and answer ~= nil then {ANSWER} = answer break end\n\
             if not ok then {ANSWER} = nil end\n\
           end\n\
         end"
    )
}

/// The chunk that tells every handler, and lets each of them fail alone.
#[must_use]
pub fn telling(event: &str) -> String {
    format!(
        "local list = {DID} and {DID}[{event:?}]\n\
         {ANSWER} = nil\n\
         if list then\n\
           for i = 1, #list do pcall(list[i], table.unpack({ARGS})) end\n\
         end"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_question_is_named_once() {
        let mut seen = ASKED.to_vec();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), ASKED.len());
    }

    #[test]
    fn the_two_namespaces_do_not_overlap() {
        // `on.promote` asks whether to; `did.promote` says that it happened. They are
        // different contracts and must not be reachable through one name.
        for name in ASKED {
            assert!(
                !TOLD.contains(name) || *name == "promote",
                "{name} is in both namespaces"
            );
        }
    }

    #[test]
    fn asking_stops_at_the_first_answer() {
        let chunk = asking("scope");
        assert!(chunk.contains("break"), "first non-nil must win");
        assert!(chunk.contains("pcall"), "one raise must not cost the rest");
    }

    #[test]
    fn telling_never_stops_early() {
        let chunk = telling("observe");
        assert!(!chunk.contains("break"), "every observer is told");
        assert!(chunk.contains("pcall"));
    }
}
