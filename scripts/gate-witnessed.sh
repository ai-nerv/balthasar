#!/bin/sh
# Commitment 5: every injectable memory names its witnesses.
#
# A fact the model is shown as true must be able to answer "how do you know". Checked against
# the schema rather than trusted: `Memory::witness` is the only path that attaches evidence,
# and a second path that forgot to would be invisible until somebody ran `aeon why`.
set -eu
grep -q 'must_be_witnessed' crates/aeon-model/src/tier.rs || {
    echo "gate-witnessed: the tier rule is gone" >&2; exit 1; }
grep -q 'if witnesses.is_empty() {' crates/aeon-model/src/confidence.rs || {
    echo "gate-witnessed: nothing witnessed no longer means nothing believed" >&2; exit 1; }
echo "gate-witnessed: ok"
