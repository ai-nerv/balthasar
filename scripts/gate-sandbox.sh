#!/bin/sh
# A Lua VM in balthasar is a sandboxed one, and there is only one place that builds it.
#
# This VM runs a project's own `.balthasar.lua`, found by walking up from the working directory,
# so it runs files balthasar did not write. `Lua::full()` hands a VM the whole standard library —
# `os.execute`, `io.popen`, `dofile` — and this was the repo that shipped one.
#
# The defence is one constructor that builds a VM from an allowlist. `sandbox.rs` has tests
# holding each VM's names to that list; what they cannot prove is that every VM went through the
# constructor, so the count is held at one and its file is named.
#
# POSIX: /bin/sh on the runner is dash.
set -eu
ROOT="${GATE_ROOT:-crates}"

ENGINE="$ROOT/balthasar-lua/src/engine.rs"
SANDBOX="$ROOT/balthasar-lua/src/sandbox.rs"
STREAM="$ROOT/balthasar-lua/src/stream.rs"

fail=0

# Comments stripped before every match below. `sandbox.rs` names in prose what it leaves out —
# `print`, `debug`, the loaders — and a gate that counted the explanation would fire on the file
# doing the defending. `symbol(` rather than `symbol`, so `sandbox::vm_REMOVED` is not a match.
bare() {
    awk '{ sub(/\/\/.*$/, ""); print }' "$1"
}

# ---- one VM, in one place ---------------------------------------------------------------------
built=$(
  find "$ROOT" -name '*.rs' -not -path '*/target/*' | sort | while IFS= read -r file; do
    bare "$file" | grep -n 'Lua::full()\|Lua::new()\|Lua::core()\|Lua::empty()' | sed "s|^|$file:|" || true
  done
)
count=$(printf '%s' "$built" | grep -c . || true)
if [ "$count" -ne 1 ]; then
  echo "gate-sandbox: a Lua VM is built in $count places; there is one sandbox and it is applied once:" >&2
  printf '%s\n' "$built" | sed 's/^/  /' >&2
  fail=1
elif ! printf '%s' "$built" | grep -q "^$SANDBOX:.*Lua::core()"; then
  echo "gate-sandbox: the VM is no longer built from luna's core in $SANDBOX:" >&2
  printf '%s\n' "$built" | sed 's/^/  /' >&2
  fail=1
fi
loaded=$(find "$ROOT" -name '*.rs' -not -path '*/target/*' | sort | while IFS= read -r file; do
  bare "$file" | grep -n 'load_io(\|load_package(\|load_debug(' | sed "s|^|$file:|" || true
done)
if [ -n "$loaded" ]; then
  echo "gate-sandbox: a VM is handed io, package or debug:" >&2
  printf '%s\n' "$loaded" | sed 's/^/  /' >&2
  fail=1
fi

# ---- and the engine takes it from there, before it installs anything or runs a line -------------
if ! bare "$ENGINE" | grep -q 'sandbox::vm('; then
  echo "gate-sandbox: $ENGINE no longer takes its VM from the sandbox" >&2
  fail=1
else
  built_at=$(bare "$ENGINE" | grep -n 'sandbox::vm(' | head -1 | cut -d: -f1)
  installed=$(bare "$ENGINE" | grep -n 'engine.install()' | head -1 | cut -d: -f1)
  if [ -n "$installed" ] && [ "$built_at" -gt "$installed" ]; then
    echo "gate-sandbox: the module is installed before the VM is built from the allowlist" >&2
    fail=1
  fi
fi

# ---- the allowlist names nothing that spawns, writes, loads a file or reaches past it -----------
# `execute` and `popen` are the spawn, `rename`/`tmpname` are writes outside any seam, `exit` ends
# a serving daemon from inside a config file, `io` opens files, the loaders read code from them,
# `debug` reaches past every guard, `print` and `warn` write to this process's own output, and
# `collectgarbage` drives the collector. `remove` is held to `os` alone: `table.remove` is kept.
listed=$(bare "$SANDBOX" | sed '/#\[cfg(test)\]/,$d')
for gone in io package dofile loadfile require debug print warn collectgarbage dump popen \
            execute exit rename tmpname setlocale; do
  if printf '%s\n' "$listed" | grep -q "\"$gone\""; then
    echo "gate-sandbox: the allowlist names \`$gone\`" >&2
    fail=1
  fi
done
if printf '%s\n' "$listed" | awk '/\("os",/,/\),/' | grep -q '"remove"'; then
  echo "gate-sandbox: the allowlist keeps os.remove" >&2
  fail=1
fi
core=$(printf '%s\n' "$listed" | awk '/^const CORE/,/^\];/')
for owner in '"load"' '"os"' '"getenv"'; do
  if printf '%s\n' "$core" | grep -q "$owner"; then
    echo "gate-sandbox: $owner is in the core every VM keeps, so a project file has it" >&2
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
if ! bare "$ENGINE" | grep -q 'bare_vm(crate::sandbox::Surface::Project)'; then
  echo "gate-sandbox: a project's own file no longer runs in a VM built from the project's list" >&2
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

# ---- no finalizer is ever enrolled, so none is ever run ------------------------------------------
# luna runs a `__gc` handler as Lua outside any budget. `setmetatable` refuses a metatable that
# carries one, and `getmetatable` keeps the metatable luna shares between its error objects out of
# reach; with nothing enrolled, nothing here calls `run_finalizers`.
if ! printf '%s\n' "$listed" | grep -q 'guard(ctx, globals, "setmetatable", finalized)'; then
  echo "gate-sandbox: setmetatable no longer refuses a metatable carrying __gc" >&2
  fail=1
fi
if ! printf '%s\n' "$listed" | grep -q 'hide_userdata_metatables(ctx, globals)'; then
  echo "gate-sandbox: getmetatable hands out a userdata's metatable again" >&2
  fail=1
fi
finalizing=$(find "$ROOT" -name '*.rs' -not -path '*/target/*' | sort | while IFS= read -r file; do
  bare "$file" | grep -n 'run_finalizers(' | sed "s|^|$file:|" || true
done)
if [ -n "$finalizing" ]; then
  echo "gate-sandbox: a finalizer is run, outside any budget:" >&2
  printf '%s\n' "$finalizing" | sed 's/^/  /' >&2
  fail=1
fi

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
