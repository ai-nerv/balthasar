#!/bin/sh
# THE RULE: no .rs over 800 lines.
#
# A file that wants to be longer is telling you the state inside it wants a second module.
# Split it while it is 800 lines and the seam is obvious, not later when there is no seam left.
set -eu
LIMIT=800
fail=0
for file in $(find crates -name '*.rs' -type f | sort); do
    lines=$(wc -l < "$file")
    if [ "$lines" -gt "$LIMIT" ]; then
        printf '%s: %s lines (limit %s)\n' "$file" "$lines" "$LIMIT" >&2
        fail=1
    fi
done
[ "$fail" -eq 0 ] || { echo "gate-file-size: failed" >&2; exit 1; }
echo "gate-file-size: ok"
