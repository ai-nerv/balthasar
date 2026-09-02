#!/bin/sh
# Commitment 2: memo never requires a model, and commitment 3: never requires embeddings.
#
# The suite runs with no key and no network. A model makes memo better; its absence must never
# make memo fail, and the only way that stays true is to prove it on every run rather than
# remember it.
set -eu
env -u ANTHROPIC_API_KEY -u OPENAI_API_KEY -u MEMO_DISTILLER \
    MEMO_NO_MODEL=1 MEMO_NO_EMBED=1 \
    cargo test --workspace --quiet
echo "gate-no-llm: ok"
