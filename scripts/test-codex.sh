#!/bin/bash
# Test runner for Azure Codex
# Usage: ./test-codex.sh "Your prompt here" [options]
#
# Options:
#   -m, --model MODEL    Model override (e.g., "claude-3-5-sonnet-20241022")
#   -j, --json           Output as JSONL
#   -o, --output FILE    Write last message to file
#   -t, --timeout SECS   Timeout in seconds (default: 120)
#   -r, --release        Use release build instead of debug
#   --review             Run review mode
#   --uncommitted        Review uncommitted changes

set -e

REPO_ROOT="Q:/src/azure-codex/azure-codex"
CONFIG_DIR="Q:/src/azure-codex/test-config"
DEBUG_EXE="$REPO_ROOT/codex-rs/target/debug/codex-exec.exe"
RELEASE_EXE="$REPO_ROOT/codex-rs/target/release/codex-exec.exe"

# Default values
USE_RELEASE=false
JSON_MODE=false
MODEL=""
OUTPUT_FILE=""
TIMEOUT=120
REVIEW_MODE=false
UNCOMMITTED=false

# Parse arguments
POSITIONAL_ARGS=()
while [[ $# -gt 0 ]]; do
    case $1 in
        -m|--model)
            MODEL="$2"
            shift 2
            ;;
        -j|--json)
            JSON_MODE=true
            shift
            ;;
        -o|--output)
            OUTPUT_FILE="$2"
            shift 2
            ;;
        -t|--timeout)
            TIMEOUT="$2"
            shift 2
            ;;
        -r|--release)
            USE_RELEASE=true
            shift
            ;;
        --review)
            REVIEW_MODE=true
            shift
            ;;
        --uncommitted)
            UNCOMMITTED=true
            shift
            ;;
        -h|--help)
            echo "Usage: $0 'Your prompt here' [options]"
            echo ""
            echo "Options:"
            echo "  -m, --model MODEL    Model override"
            echo "  -j, --json           Output as JSONL"
            echo "  -o, --output FILE    Write last message to file"
            echo "  -t, --timeout SECS   Timeout (default: 120)"
            echo "  -r, --release        Use release build"
            echo "  --review             Run review mode"
            echo "  --uncommitted        Review uncommitted changes"
            echo ""
            echo "Examples:"
            echo "  $0 'Say hello'"
            echo "  $0 'What is 2+2?' -m 'claude-3-5-sonnet-20241022' -j"
            echo "  $0 --review --uncommitted"
            exit 0
            ;;
        *)
            POSITIONAL_ARGS+=("$1")
            shift
            ;;
    esac
done

# Restore positional args
set -- "${POSITIONAL_ARGS[@]}"

# Select executable
if [ "$USE_RELEASE" = true ]; then
    CODEX_EXE="$RELEASE_EXE"
else
    CODEX_EXE="$DEBUG_EXE"
fi

# Check executable exists
if [ ! -f "$CODEX_EXE" ]; then
    echo "Error: codex-exec not found at $CODEX_EXE"
    echo "Build it with: cd $REPO_ROOT/codex-rs && cargo build -p codex-exec"
    exit 1
fi

# Set environment
export AZURE_CODEX_HOME="$CONFIG_DIR"

echo "Using config from: $CONFIG_DIR"
echo "Using executable: $CODEX_EXE"

# Build command arguments
ARGS=()
ARGS+=("--skip-git-repo-check")

if [ -n "$MODEL" ]; then
    ARGS+=("-m" "$MODEL")
fi

if [ "$JSON_MODE" = true ]; then
    ARGS+=("--json")
fi

if [ -n "$OUTPUT_FILE" ]; then
    ARGS+=("-o" "$OUTPUT_FILE")
fi

# Handle review mode
if [ "$REVIEW_MODE" = true ]; then
    ARGS+=("review")
    if [ "$UNCOMMITTED" = true ]; then
        ARGS+=("--uncommitted")
    fi
else
    PROMPT="${1:-}"
    if [ -z "$PROMPT" ]; then
        echo "Error: No prompt provided"
        echo "Usage: $0 'Your prompt here' [options]"
        exit 1
    fi
    ARGS+=("$PROMPT")
fi

echo "Running: $CODEX_EXE ${ARGS[*]}"
echo "---"

# Run with timeout
timeout "$TIMEOUT" "$CODEX_EXE" "${ARGS[@]}"
EXIT_CODE=$?

echo "---"
if [ $EXIT_CODE -eq 0 ]; then
    echo "Exit code: $EXIT_CODE (success)"
else
    echo "Exit code: $EXIT_CODE (failure)"
fi

if [ -n "$OUTPUT_FILE" ] && [ -f "$OUTPUT_FILE" ]; then
    echo ""
    echo "Output saved to: $OUTPUT_FILE"
    echo "Content:"
    cat "$OUTPUT_FILE"
fi

exit $EXIT_CODE
