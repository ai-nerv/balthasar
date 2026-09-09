#!/bin/sh
# A Lua VM in balthasar is a sandboxed one, and there is only one place that builds it.
#
# This VM runs a project's own `.balthasar.lua`, found by walking up from the working directory,
# so it runs files balthasar did not write. `Lua::full()` hands a VM the whole standard library —
# `os.execute`, `io.popen`, `dofile` — and this was the repo that shipped one.
#
# The defence is one line in one constructor. `sandbox.rs` has tests proving the removals work;
# what they cannot prove is that every VM went through the constructor that applies them, so the
# count is held at one and its file is named.
#
# POSIX: /bin/sh on the runner is dash.
set -eu
ROOT="${GATE_ROOT:-crates}"

ENGINE="$ROOT/balthasar-lua/src/engine.rs"
SANDBOX="$ROOT/balthasar-lua/src/sandbox.rs"
STREAM="$ROOT/balthasar-lua/src/stream.rs"

fail=0

# Comments stripped before every match below. `sandbox.rs` opens by naming `Lua::full()` — it is
# explaining what it takes away — and a gate that counted the explanation would fire on the file
# doing the defending. `symbol(` rather than `symbol`, so `sandbox::apply_REMOVED` is not a match.
bare() {
    awk '{ sub(/\/\/.*$/, ""); print }' "$1"
}

# ---- one VM, in one place ---------------------------------------------------------------------
built=$(
  find "$ROOT" -name '*.rs' -not -path '*/target/*' | sort | while IFS= read -r file; do
    bare "$file" | grep -n 'Lua::full()\|Lua::new()' | sed "s|^|$file:|" || true
  done
)
count=$(printf '%s' "$built" | grep -c . || true)
if [ "$count" -ne 1 ]; then
  echo "gate-sandbox: a Lua VM is built in $count places; there is one sandbox and it is applied once:" >&2
  printf '%s\n' "$built" | sed 's/^/  /' >&2
  fail=1
elif ! printf '%s' "$built" | grep -q "^$ENGINE:"; then
  echo "gate-sandbox: the VM is no longer built in $ENGINE:" >&2
  printf '%s\n' "$built" | sed 's/^/  /' >&2
  fail=1
fi

# ---- and that place trims it, before it installs anything or runs a line ------------------------
if ! bare "$ENGINE" | grep -q 'sandbox::apply('; then
  echo "gate-sandbox: $ENGINE builds a VM and does not apply the sandbox to it" >&2
  fail=1
else
  applied=$(bare "$ENGINE" | grep -n 'sandbox::apply(' | head -1 | cut -d: -f1)
  installed=$(bare "$ENGINE" | grep -n 'engine.install()' | head -1 | cut -d: -f1)
  if [ -n "$installed" ] && [ "$applied" -gt "$installed" ]; then
    echo "gate-sandbox: the sandbox is applied after the module is installed, not before" >&2
    fail=1
  fi
fi

# ---- the removals are still the removals --------------------------------------------------------
# Field by field: `execute` and `popen` are the spawn, `remove`/`rename`/`tmpname` are writes
# outside any seam, `exit` ends a serving daemon from inside a config file, and `io` goes
# wholesale because every remaining member of it opens a file.
for gone in execute exit remove rename tmpname setlocale; do
  if ! bare "$SANDBOX" | grep -q "\"$gone\""; then
    echo "gate-sandbox: os.$gone is no longer removed from the VM" >&2
    fail=1
  fi
done
for gone in io package dofile loadfile require; do
  if ! bare "$SANDBOX" | grep -q "\"$gone\""; then
    echo "gate-sandbox: the global \`$gone\` is no longer removed from the VM" >&2
    fail=1
  fi
done

# ---- and the socket primitive is still narrowed --------------------------------------------------
# `__stream` is a global in this VM on purpose: the family's client stubs are plain Lua and cannot
# open a socket. It used to dial anything this user could, which put a sibling's control socket
# inside reach of an untrusted project file. Both ends are checked — the rule and the call — since
# a `dialable` nobody consults is a function, not a gate.
if ! bare "$STREAM" | grep -q 'fn dialable('; then
  echo "gate-sandbox: nothing decides which sockets a config may dial" >&2
  fail=1
fi
if ! bare "$STREAM" | grep -q 'if !dialable('; then
  echo "gate-sandbox: the connect path no longer asks whether a socket is dialable" >&2
  echo "gate-sandbox: a project file could then open any socket this user can" >&2
  fail=1
fi

# ---- an untrusted file may choose, but not declare ------------------------------------------------
# `self.` is part of the match: `refuse_declarations(` alone also matches the `fn` that defines
# it, so deleting the only call site left the gate green.
if ! bare "$ENGINE" | grep -q 'self.refuse_declarations('; then
  echo "gate-sandbox: a project's own file is no longer held to what it may declare" >&2
  fail=1
fi

[ "$fail" -eq 0 ] || { echo "gate-sandbox: failed" >&2; exit 1; }
echo "gate-sandbox: ok"
