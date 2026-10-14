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
# open a socket. It reaches the family's own socket directories and nothing else, by name, and the
# connect path dials only what `dialable` handed back.
if ! bare "$STREAM" | grep -q 'fn dialable('; then
  echo "gate-sandbox: nothing decides which sockets a config may dial" >&2
  fail=1
fi
if ! bare "$STREAM" | grep -q '= dialable('; then
  echo "gate-sandbox: the connect path no longer asks whether a socket is dialable" >&2
  echo "gate-sandbox: a project file could then open any socket this user can" >&2
  fail=1
fi
if ! bare "$STREAM" | grep -q 'runtime::family_dirs()'; then
  echo "gate-sandbox: dialable no longer names the family's own socket directories" >&2
  fail=1
fi
if bare "$STREAM" | grep -q 'temp_dir()\|XDG_RUNTIME_DIR'; then
  echo "gate-sandbox: dialable reaches a whole directory again, not the family's sockets in it" >&2
  fail=1
fi

# ---- a project's own file is data, in a VM of its own --------------------------------------------
PROJECT="$ROOT/balthasar-lua/src/project.rs"
HELPERS="$ROOT/balthasar-lua/src/helpers.rs"
if ! bare "$ENGINE" | grep -q 'crate::project::install('; then
  echo "gate-sandbox: a project's own file no longer runs in the data-only VM" >&2
  fail=1
fi
if ! bare "$ENGINE" | grep -q 'refuse_declarations(path)'; then
  echo "gate-sandbox: a project's own file is no longer held to what it may declare" >&2
  fail=1
fi
for lent in '"fs"' '"git"' '"stream"' 'stream::table' 'helpers::install('; do
  if bare "$PROJECT" | grep -q "$lent"; then
    echo "gate-sandbox: the project VM is handed $lent" >&2
    fail=1
  fi
done
for gone in '"load"' '"getenv"'; do
  if ! bare "$PROJECT" | grep -q "$gone"; then
    echo "gate-sandbox: the project VM no longer removes $gone" >&2
    fail=1
  fi
done

# ---- fs.read is lent to the person's own files only ----------------------------------------------
if ! bare "$HELPERS" | grep -q 'lent.get()'; then
  echo "gate-sandbox: balthasar.fs.read no longer asks whether it is lent" >&2
  fail=1
fi

# ---- and every chunk runs under a budget ---------------------------------------------------------
if ! bare "$ENGINE" | grep -q 'crate::fuel::drive('; then
  echo "gate-sandbox: chunks no longer run under an instruction budget" >&2
  fail=1
fi
unbudgeted=$(find "$ROOT/balthasar-lua/src" -name '*.rs' | sort | while IFS= read -r file; do
  bare "$file" | grep -n '\.execute::<\|\.finish(' | sed "s|^|$file:|" || true
done)
if [ -n "$unbudgeted" ]; then
  echo "gate-sandbox: a chunk runs without a budget:" >&2
  printf '%s\n' "$unbudgeted" | sed 's/^/  /' >&2
  fail=1
fi

[ "$fail" -eq 0 ] || { echo "gate-sandbox: failed" >&2; exit 1; }
echo "gate-sandbox: ok"
