#!/bin/sh
# Nothing closed, archived or forgotten is offered as memory.
#
# The files that build what a model is told may not read the `memory` table, and may not call the
# search that can return history. They read open memories through `Store::recall_current`, which
# only `crates/balthasar-store/src/current.rs` can satisfy, from the `live_memory` view.
set -eu

SERVING='crates/balthasar-host/src/supply.rs
crates/balthasar-host/src/supply/standing.rs
crates/balthasar-host/src/catalog.rs
crates/balthasar-recall/src/assemble.rs
crates/balthasar-store/src/current.rs'

# `FROM memory` or `JOIN memory` as SQL: followed by a clause word, a separator, a quote or the
# end of the line. Prose such as "From memory:" is not SQL.
SQL='\b(from|join)[[:space:]]+memory([[:space:]]+(as|where|join|on|order|group|limit)\b|[[:space:]]*[,;)"\\]|[[:space:]]*$)'

fail=0
for file in $SERVING; do
    [ -f "$file" ] || continue
    if grep -niE "$SQL" "$file" >/dev/null 2>&1; then
        echo "gate-served: $file reads the memory table rather than live_memory:" >&2
        grep -niE "$SQL" "$file" >&2
        fail=1
    fi
    if [ "$file" != 'crates/balthasar-store/src/current.rs' ] \
        && grep -nE '\.recall\(&' "$file" >/dev/null 2>&1; then
        echo "gate-served: $file searches with history; use recall_current:" >&2
        grep -nE '\.recall\(&' "$file" >&2
        fail=1
    fi
done

# The one constructor stays private: a `Current` from anywhere else could hold anything.
if grep -nE 'pub[[:space:]]+(\(crate\)[[:space:]]+)?fn[[:space:]]+(new|from|of|assume)\b' \
    crates/balthasar-store/src/current.rs >/dev/null 2>&1 \
    || grep -nE 'pub struct Current\(pub' crates/balthasar-store/src/current.rs >/dev/null 2>&1; then
    echo "gate-served: Current gained a public constructor" >&2
    fail=1
fi

[ "$fail" -eq 0 ] || exit 1
echo "gate-served: ok"
