#!/bin/bash
# Quick test helper - run from any directory
# Usage: ./scripts/quick-test.sh "prompt" [model]

REPO="Q:/src/azure-codex/azure-codex"
export AZURE_CODEX_HOME="Q:/src/azure-codex/test-config"

PROMPT="${1:-Hello}"
MODEL="${2:-}"

ARGS="--skip-git-repo-check"
if [ -n "$MODEL" ]; then
    ARGS="$ARGS -m $MODEL"
fi

echo "=== Quick Test ==="
echo "Prompt: $PROMPT"
echo "Model: ${MODEL:-default}"
echo "=================="

"$REPO/codex-rs/target/debug/codex-exec.exe" $ARGS "$PROMPT"
