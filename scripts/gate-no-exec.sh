#!/bin/sh
# Commitment 10: procedural memory describes a tool action; the harness retains authority to
# execute it.
#
# memo stores procedures precise enough to reuse, which is exactly what makes this gate
# necessary: a descriptor is a list of operations with typed parameters, and the distance
# between that and a command line is one `Command::new` somebody adds in a hurry.
#
# The distiller's shell-out backend is the one place memo runs anything, and it runs a
# CONFIGURED command against text — never a stored procedure. It is named here rather than
# exempted by silence.
set -eu

ALLOWED='crates/memo-distil/src/distil.rs'

fail=0
found=$(grep -rn 'Command::new\|process::Command' crates --include='*.rs' \
        | grep -v "^$ALLOWED:" \
        | grep -v '/tests/' \
        | grep -v 'build.rs' || true)

if [ -n "$found" ]; then
    echo "gate-no-exec: something outside the distiller runs a program:" >&2
    echo "$found" >&2
    fail=1
fi

# A skill descriptor's parameters are data. If anything ever formats one into a string that is
# then run, the type stops being a boundary.
if grep -rn 'format!' crates/memo-model/src/skill.rs | grep -qi 'command\|exec\|sh -c'; then
    echo "gate-no-exec: a skill descriptor is being formatted into a command" >&2
    fail=1
fi

[ "$fail" -eq 0 ] || exit 1
echo "gate-no-exec: ok"
