#!/bin/sh
# Commitment: untrusted content cannot become durable instruction.
#
# Four checks, each one precise about a specific way the defence could be taken out. A gate that
# merely asserted "security is good" would pass forever; these fail if the mechanism is removed.
set -eu

fail=0

# 1. The channel, not the wording, decides what a witness may be. `witness_for` is the only
#    thing that downgrades an imperative claimed by content that cannot make one, so if nothing
#    calls it the defence is decoration.
if ! grep -rqn 'witness_for' crates/balthasar-model/src/guard.rs; then
    echo "gate-untrusted: the imperative downgrade is gone" >&2
    fail=1
fi
if ! grep -rqn 'may_be_imperative' crates/balthasar-model/src/channel.rs; then
    echo "gate-untrusted: channels no longer say what may be an instruction" >&2
    fail=1
fi

# 2. Diversity has to count sources, not only sessions. Ten runs quoting one page are one
#    source, and this is the line that knows it.
if ! grep -qn 'domains.len()' crates/balthasar-model/src/confidence.rs; then
    echo "gate-untrusted: confidence stopped counting trust domains" >&2
    fail=1
fi
if ! grep -qn 'WITHIN_DOMAIN' crates/balthasar-model/src/confidence.rs; then
    echo "gate-untrusted: repetition within one source is no longer damped" >&2
    fail=1
fi

# 3. Quarantine is a gate rather than advice.
if ! grep -qn 'fn may_inject' crates/balthasar-model/src/utility.rs; then
    echo "gate-untrusted: quarantine no longer gates injection" >&2
    fail=1
fi

# 4. Purge closes every path back. Each of these tables can reconstruct or point at what was
#    removed, so each has to be named in the purge — this is the list that grew every time a
#    new derived table arrived, and forgetting one is how a purged secret stays reachable.
for table in link relation_view entity recall_candidate injection_memory action_memory witness memory_fts; do
    if ! grep -qn "DELETE FROM $table" crates/balthasar-store/src/purge.rs; then
        echo "gate-untrusted: purge does not cover '$table'" >&2
        fail=1
    fi
done

[ "$fail" -eq 0 ] || exit 1
echo "gate-untrusted: ok"
