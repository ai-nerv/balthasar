#!/bin/sh
# Commitment 1: memo depends on no harness.
#
# A harness is a Lua source adapter, not a Rust module. The moment memo's Rust knows axon's
# Entry enum it is a component of axon wearing a socket, and "independent layer" becomes
# marketing. config/ mentions axon constantly; that is the point, and Lua is not Rust.
set -eu
# A LEADING word boundary, and no trailing one.
#
# A bare substring matched `axon` inside `taxonomy` and reported a doc comment about benchmark
# categories as a dependency. A full `-w` match then stopped catching `axon_proto`, which is
# the thing the gate is actually for. Leading-only gets both: `taxonomy` has a letter before
# the `a`, and `axon_proto` does not.
fail=0
if grep -rlnE '\baxon' crates --include='*.rs' >/dev/null 2>&1; then
    echo "gate-independent: a Rust file names a harness:" >&2
    grep -rlnE '\baxon' crates --include='*.rs' >&2
    fail=1
fi
if grep -rn '^axon' crates/*/Cargo.toml >/dev/null 2>&1; then
    echo "gate-independent: a crate depends on a harness" >&2
    fail=1
fi
[ "$fail" -eq 0 ] || exit 1
echo "gate-independent: ok"
