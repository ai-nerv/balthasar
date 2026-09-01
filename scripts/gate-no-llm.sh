#!/bin/sh
# Commitment 2: aeon never requires a model, and commitment 3: never requires embeddings.
#
# The suite runs with no key and no network. A model makes aeon better; its absence must never
# make aeon fail, and the only way that stays true is to prove it on every run rather than
# remember it.
set -eu
env -u ANTHROPIC_API_KEY -u OPENAI_API_KEY -u AEON_DISTILLER \
    AEON_NO_MODEL=1 AEON_NO_EMBED=1 \
    cargo test --workspace --quiet
echo "gate-no-llm: ok"
