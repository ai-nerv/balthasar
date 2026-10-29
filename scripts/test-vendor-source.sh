#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
source=$(nix eval --offline --impure --raw --expr '
  let
    locked = builtins.getFlake ("git+file://" + toString ./.);
    project = (import ./flake.nix).outputs locked.inputs;
  in toString project.packages.${builtins.currentSystem}.default.src
')
case "$source" in /nix/store/*) ;; *) echo 'unexpected source path' >&2; exit 1 ;; esac
test -f "$source/vendor/candle-core/Cargo.toml" || {
    echo "packaged source omits vendor/candle-core: $source" >&2
    exit 1
}
diff -r vendor "$source/vendor"
cmp Cargo.toml "$source/Cargo.toml"
cmp Cargo.lock "$source/Cargo.lock"
report=$(mktemp)
trap 'rm -f "$report"' EXIT HUP INT TERM
cargo metadata --offline --locked --no-deps --format-version 1 \
    --manifest-path "$source/Cargo.toml" > "$report"
jq -e '.workspace_members | length == 12' "$report" >/dev/null
printf 'vendor source: exact patched bytes and locked workspace metadata PASS (%s)\n' "$source"
