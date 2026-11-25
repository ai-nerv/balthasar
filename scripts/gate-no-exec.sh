#!/bin/sh
# Commitment 10: procedural memory describes a tool action; the harness retains authority to
# execute it.
#
# balthasar stores procedures precise enough to reuse, which is exactly what makes this gate
# necessary: a descriptor is a list of operations with typed parameters, and the distance
# between that and a command line is one `Command::new` somebody adds in a hurry.
#
# The distiller's shell-out backend is the one place balthasar runs anything, and it runs a
# CONFIGURED command against text — never a stored procedure. It and the runner it uses are named
# here rather than exempted by silence.
set -eu

ALLOWED='crates/balthasar-distil/src/distil.rs'
RUNNER='crates/balthasar-distil/src/child.rs'

fail=0
found=$(grep -rn 'Command::new\|process::Command' crates --include='*.rs' \
        | grep -v "^$ALLOWED:" \
        | grep -v "^$RUNNER:" \
        | grep -v '/tests/' \
        | grep -v 'build.rs' || true)

if [ -n "$found" ]; then
    echo "gate-no-exec: something outside the distiller runs a program:" >&2
    echo "$found" >&2
    fail=1
fi

[ "$fail" -eq 0 ] || exit 1
echo "gate-no-exec: ok"
