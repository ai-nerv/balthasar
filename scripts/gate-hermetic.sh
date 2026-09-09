#!/bin/sh
# The suite leaves nothing behind, anywhere it is entitled to write.
#
# It used to leave a great deal: every test tidied up on its last line, and `assert!` unwinds
# straight past a trailing `remove_dir_all`. So a *failing* test always leaked, and the
# delete-then-create helpers only ever revisited their own name under their own pid, which never
# repeats. Thousands of directories accumulated across two renames of this project and nothing
# said so, because nothing looked.
#
# **`$TMPDIR` was never the whole of it.** This gate ran with a temporary directory of its own and
# checked that one directory, so it saw exactly the leaks that landed there — and balthasar's
# sockets do not. They land in `$XDG_RUNTIME_DIR/balthasar`, one directory shared by every
# instance on the machine, which the gate inherited from the developer running it and could not
# have complained about without complaining about everybody else's daemons too. `api@stub-*.sock`
# and `api@crowd-*.sock` collected there for as long as this gate has existed, and a stale socket
# is the input to the bug where a forked child's probe unlinks a live daemon's name. So every
# directory a test could reasonably write to is given a fresh one under this root, and the whole
# root is what gets checked.
#
# What it still cannot see is a test that writes to a path with no variable in it at all. Nothing
# in a run tells you that happened, so it is read out of the source instead -- see the scan at the
# bottom, which is a different kind of check and deliberately a narrow one.
set -eu

# **Short, and rooted at `/tmp` rather than under whatever `$TMPDIR` already is.** A unix socket
# path may not exceed `SUN_LEN` — 108 bytes — and several tests here bind one inside a scratch
# directory inside this root. On a developer's machine `$TMPDIR` is `/tmp` and nesting is free; on
# the runner it is `/home/runner/work/_temp`, and the same test failed with "path must be shorter
# than SUN_LEN" in the one place the gate was supposed to be proving something.
#
# Isolation comes from the directory being ours, not from where it hangs.
base=/tmp
[ -d "$base" ] && [ -w "$base" ] || base="${TMPDIR:-.}"
root=$(mktemp -d "$base/gh-XXXXXX")
trap 'rm -rf "$root"' EXIT HUP INT TERM

# Kept rather than discarded. When this fails it is a test failing, not a leak, and the name of
# the test is the whole answer — a gate that printed only "exit 101" sent the reader back to
# `cargo test` to find out what it already knew.
out=$(mktemp "$base/gh-log-XXXXXX")
trap 'rm -rf "$root" "$out"' EXIT HUP INT TERM

# One directory per thing a test might write to, all of them ours. `HOME` is deliberately left
# alone: cargo reads its registry through it, and a gate that broke the build to prove a point
# about tidiness would be turned off within a week.
mkdir -p "$root/tmp" "$root/run" "$root/data" "$root/config" "$root/state" "$root/cache"

if ! TMPDIR="$root/tmp" \
     XDG_RUNTIME_DIR="$root/run" \
     XDG_DATA_HOME="$root/data" \
     XDG_CONFIG_HOME="$root/config" \
     XDG_STATE_HOME="$root/state" \
     XDG_CACHE_HOME="$root/cache" \
     cargo test --all-targets --quiet >"$out" 2>&1; then
  cat "$out" >&2
  echo "gate-hermetic: the suite failed; nothing was checked" >&2
  exit 1
fi

# What a *product* is entitled to leave, as paths relative to the root. The six directories are
# the ones made above; `run/balthasar` is where a listener binds and `balthasar.tool` is the
# descriptor a caller with no socket spawns from, both of which balthasar writes on purpose and
# neither of which is a test's mess. `tmp/balthasar-<uid>` is the same runtime directory under its
# fallback name, for a test that unsets `$XDG_RUNTIME_DIR` to see what happens.
#
# Anything else — a socket, a store, a scratch directory, a stray file — is a test that did not
# clean up after itself, and on the failing run it never will.
left=$(
  cd "$root" && find . -mindepth 1 | sed 's|^\./||' | grep -vxE \
    'tmp|run|data|config|state|cache|run/balthasar|run/balthasar/balthasar\.tool|tmp/balthasar-[0-9]+' \
    || true
)

if [ -n "$left" ]; then
  echo "gate-hermetic: the suite left these behind:" >&2
  printf '  %s\n' $left >&2
  echo "gate-hermetic: failed" >&2
  exit 1
fi

# A path with no variable in it goes nowhere this gate can move, so no run of the suite can show
# it. Read out of the source instead, and kept to the four roots that are shared and writable:
# `/home` is absent on purpose, because scope identifiers in this suite are made-up paths like
# `/home/you/work/thing` that are compared as strings and never touched.
#
# Comments are stripped first, for the reason `gate-cycles.sh` strips them: a file that explains
# itself by naming `/tmp` is describing the problem, not causing it.
literal=$(
  find crates -name '*.rs' -type f -not -path '*/target/*' | sort | while IFS= read -r file; do
    sed 's|//.*$||' "$file" | grep -nE '"(/tmp|/var/tmp|/dev/shm|/run)[/"]' | sed "s|^|$file:|"
  done
)

if [ -n "$literal" ]; then
  echo "gate-hermetic: these name a shared directory outright, where nothing can isolate them:" >&2
  printf '%s\n' "$literal" | sed 's/^/  /' >&2
  echo "gate-hermetic: use balthasar_model::scratch::Scratch" >&2
  exit 1
fi
echo "gate-hermetic: ok"
