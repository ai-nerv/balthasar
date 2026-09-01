#!/bin/sh
# Commitment 4: nothing a memory holds is ever deleted.
#
# Superseded, invalidated, archived, decayed below the floor - every one of those is a column.
# There is exactly one file allowed to remove a memory, and it exists so that "delete the key I
# pasted" can be answered with yes.
#
# DERIVED INDEXES ARE DIFFERENT IN KIND, and the gate says so rather than exempting a file.
# `entity` and `memory_fts` hold nothing that is not recomputable from `memory`; re-indexing has
# to clear the old rows or a better extractor leaves its predecessor's names behind and the
# rarity counts go quietly wrong. So a delete outside purge.rs is allowed only against one of
# those, and any other target still fails.
set -eu

PURGE='crates/aeon-store/src/purge.rs'
DERIVED='entity memory_fts'

fail=0
grep -rn 'DELETE FROM' crates --include='*.rs' | grep -v "^$PURGE:" > /tmp/aeon-deletes.txt || true

while IFS= read -r line; do
    [ -n "$line" ] || continue
    target=$(printf '%s' "$line" | sed -n 's/.*DELETE FROM \([A-Za-z_][A-Za-z0-9_]*\).*/\1/p')
    ok=0
    for d in $DERIVED; do
        [ "$target" = "$d" ] && ok=1
    done
    if [ "$ok" -eq 0 ]; then
        echo "gate-no-delete: '$target' is not a derived index" >&2
        echo "  $line" >&2
        fail=1
    fi
done < /tmp/aeon-deletes.txt
rm -f /tmp/aeon-deletes.txt

[ "$fail" -eq 0 ] || exit 1
echo "gate-no-delete: ok"
