#!/bin/sh
# Commitment 1: balthasar depends on no harness.
#
# A harness is a Lua source adapter, not a Rust module. The moment balthasar's Rust knows magi's
# Entry enum it is a component of magi wearing a socket, and "independent layer" becomes
# marketing. config/ mentions magi constantly; that is the point, and Lua is not Rust.
set -eu
# A LEADING word boundary, and no trailing one.
#
# A bare substring matches the name inside longer words and reports prose as a dependency. A
# full `-w` match stops catching `magi_proto`, which is the thing the gate is actually for.
# Leading-only gets both.
fail=0
if grep -rlnE '\bmagi' crates --include='*.rs' >/dev/null 2>&1; then
    echo "gate-independent: a Rust file names a harness:" >&2
    grep -rlnE '\bmagi' crates --include='*.rs' >&2
    fail=1
fi
if grep -rn '^magi' crates/*/Cargo.toml >/dev/null 2>&1; then
    echo "gate-independent: a crate depends on a harness" >&2
    fail=1
fi
[ "$fail" -eq 0 ] || exit 1
echo "gate-independent: ok"
