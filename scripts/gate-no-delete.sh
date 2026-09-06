#!/bin/sh
# Commitment 4: nothing a memory holds is ever deleted.
#
# Superseded, invalidated, archived, decayed below the floor - every one of those is a column.
# There is exactly one file allowed to remove a memory, and it exists so that "delete the key I
# pasted" can be answered with yes.
#
# DERIVED INDEXES ARE DIFFERENT IN KIND, and the gate says so rather than exempting a file.
# `entity`, `memory_fts` and `turn_fts` hold nothing not recomputable from `memory` or `turn`;
# re-indexing has
# to clear the old rows or a better extractor leaves its predecessor's names behind and the
# rarity counts go quietly wrong.
#
# THE LEDGER IS A THIRD KIND. `recall_run` and the tables under it are bounded telemetry with a
# retention policy the user sets: they record that a search happened and how the acting went,
# never what a memory is. A store whose entire ledger has aged out still believes exactly what
# it believed, with the same witnesses and the same confidence - `forget_ledger_before` has a
# test that holds it to that. Unlike a derived index these rows are NOT recomputable, so the
# carve-out is narrower: only the retention function may remove them, and only that file.
set -eu

PURGE='crates/balthasar-store/src/purge.rs'
LEDGER='crates/balthasar-store/src/usage.rs'
DERIVED='entity memory_fts turn_fts'
TELEMETRY='recall_run recall_candidate injection injection_memory action_use action_memory outcome'

fail=0
# `mktemp`, not a fixed path. This is the only gate that writes anything, and a predictable name
# under a world-writable directory is both a collision between two jobs on one runner and a
# symlink somebody else can plant.
deletes=$(mktemp)
trap 'rm -f "$deletes"' EXIT HUP INT TERM
grep -rn 'DELETE FROM' crates --include='*.rs' | grep -v "^$PURGE:" > "$deletes" || true

while IFS= read -r line; do
    [ -n "$line" ] || continue
    target=$(printf '%s' "$line" | sed -n 's/.*DELETE FROM \([A-Za-z_][A-Za-z0-9_]*\).*/\1/p')
    file=$(printf '%s' "$line" | cut -d: -f1)
    ok=0
    for d in $DERIVED; do
        [ "$target" = "$d" ] && ok=1
    done
    if [ "$file" = "$LEDGER" ]; then
        for t in $TELEMETRY; do
            [ "$target" = "$t" ] && ok=1
        done
    fi
    if [ "$ok" -eq 0 ]; then
        echo "gate-no-delete: '$target' is neither a derived index nor ledger telemetry" >&2
        echo "  $line" >&2
        fail=1
    fi
done < "$deletes"

# The ledger's carve-out is for retention only. A delete against a memory table from inside the
# ledger file would pass the check above, so it is named separately here.
if grep -n 'DELETE FROM memory\|DELETE FROM witness\|DELETE FROM link' "$LEDGER" >/dev/null 2>&1; then
    echo "gate-no-delete: the ledger removed a memory" >&2
    fail=1
fi

[ "$fail" -eq 0 ] || exit 1
echo "gate-no-delete: ok"
