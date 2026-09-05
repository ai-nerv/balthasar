#!/bin/sh
# THE RULE: no .rs over 800 lines.
#
# A file that wants to be longer is telling you the state inside it wants a second module.
# Split it while it is 800 lines and the seam is obvious, not later when there is no seam left.
#
# `GATE_ROOT` because the single-crate siblings keep their sources in `src`, not `crates`.
set -eu
LIMIT=800
ROOT="${GATE_ROOT:-crates}"

# The offenders are collected as output rather than counted into a flag: a `while` loop on the
# right of a pipe runs in a subshell, so a flag set inside it is lost on the way out. Reading the
# names this way also survives a path with a space in it, which `for f in $(find ...)` does not.
offenders=$(
  # Deliberately unquoted: ROOT may name more than one tree ("src tests").
  # shellcheck disable=SC2086
  find $ROOT -name '*.rs' -type f -not -path '*/target/*' | sort | while IFS= read -r file; do
    lines=$(wc -l < "$file")
    if [ "$lines" -gt "$LIMIT" ]; then
      printf '%s: %s lines (limit %s)\n' "$file" "$lines" "$LIMIT"
    fi
  done
)

if [ -n "$offenders" ]; then
  printf '%s\n' "$offenders" >&2
  echo "gate-file-size: failed" >&2
  exit 1
fi
echo "gate-file-size: ok"
