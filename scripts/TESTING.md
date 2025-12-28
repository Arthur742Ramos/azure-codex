# Azure Codex Testing Infrastructure

This document describes how to test Azure Codex changes using the non-interactive `codex-exec` binary.

## Quick Start

```bash
# Set test config directory
export AZURE_CODEX_HOME="Q:/src/azure-codex/test-config"

# Run a simple prompt
./codex-rs/target/debug/codex-exec.exe --skip-git-repo-check "Say hello"

# Run with JSON output (for parsing)
./codex-rs/target/debug/codex-exec.exe --skip-git-repo-check --json "Say hello"

# Run with specific model
./codex-rs/target/debug/codex-exec.exe --skip-git-repo-check -m "gpt-5.2" "Say hello"

# Save output to file
./codex-rs/target/debug/codex-exec.exe --skip-git-repo-check -o output.txt "Say hello"
```

## Test Scripts

### PowerShell (Windows)

```powershell
# Simple test
.\scripts\test-codex.ps1 -Prompt "Say hello"

# With model override
.\scripts\test-codex.ps1 -Prompt "Say hello" -Model "gpt-5.2"

# JSON output
.\scripts\test-codex.ps1 -Prompt "Say hello" -Json

# Review mode
.\scripts\test-codex.ps1 -Review -Uncommitted
```

### Bash

```bash
# Simple test
./scripts/test-codex.sh "Say hello"

# With model override
./scripts/test-codex.sh "Say hello" -m "gpt-5.2"

# JSON output
./scripts/test-codex.sh "Say hello" -j

# Review mode
./scripts/test-codex.sh --review --uncommitted
```

## Configuration

### Test Config Directory

The test scripts use `Q:\src\azure-codex\test-config` by default. This directory should contain:

```
test-config/
├── config.toml      # Main configuration
├── sessions/        # Session data
└── log/            # Logs
```

### Config File Example

```toml
# For Azure OpenAI (GPT models)
azure_endpoint = "https://your-resource.openai.azure.com/"
model = "gpt-5.2"
model_reasoning_effort = "high"

# For Azure AI Services (Claude models)
# Note: Requires a different endpoint with Claude deployments
# azure_endpoint = "https://your-aiservices-endpoint.azure.com/"
# model = "claude-3-5-sonnet-20241022"
```

### Testing Different Providers

#### Azure OpenAI (GPT models)
```toml
azure_endpoint = "https://your-openai-resource.openai.azure.com/"
model = "gpt-5.2"
```

#### Azure AI Services (Claude models)
```toml
azure_endpoint = "https://your-aiservices-resource.cognitiveservices.azure.com/"
model = "claude-3-5-sonnet-20241022"
```

## Test Scenarios

### 1. Basic Prompt Test
```bash
export AZURE_CODEX_HOME="Q:/src/azure-codex/test-config"
./codex-rs/target/debug/codex-exec.exe --skip-git-repo-check "What is 2+2?"
```

Expected: Should return "4" or similar response with token usage displayed.

### 2. Token Usage Test (Context Bar)
```bash
./codex-rs/target/debug/codex-exec.exe --skip-git-repo-check --json "Say hello" 2>&1 | grep -i token
```

Expected: Should show token_usage in the JSON events.

### 3. Review Mode Test
```bash
./codex-rs/target/debug/codex-exec.exe --skip-git-repo-check review --uncommitted
```

Expected: Should show "Working..." progress, then review findings.

### 4. Model Override Test
```bash
./codex-rs/target/debug/codex-exec.exe --skip-git-repo-check -m "gpt-4o" "Hello"
```

Expected: Should use the specified model.

## JSON Output Format

When using `--json`, events are output as JSONL (one JSON object per line):

```json
{"type":"session_configured","session_id":"...","model":"gpt-5.2"}
{"type":"task_started","model_context_window":128000}
{"type":"agent_message","content":"Hello!"}
{"type":"token_usage_info","total_token_usage":{"input_tokens":100,"output_tokens":50}}
{"type":"task_complete"}
```

## Debugging

### Enable Debug Logging
```bash
RUST_LOG=debug ./codex-rs/target/debug/codex-exec.exe --skip-git-repo-check "Hello"
```

### Common Issues

1. **404 Not Found**: Model deployment doesn't exist at the configured endpoint
2. **Authentication failed**: Run `az login` to refresh Azure credentials
3. **Context window exceeded**: Reduce prompt size or use a model with larger context

## Building for Testing

```bash
# Debug build (faster, for development)
cd codex-rs
cargo build -p codex-exec

# Release build (for final testing)
cargo build -p codex-exec --release
```

## Integration with CI

For automated testing in CI pipelines:

```yaml
- name: Test Azure Codex
  env:
    AZURE_CODEX_HOME: ./test-config
  run: |
    ./codex-rs/target/release/codex-exec --skip-git-repo-check --json "Hello" > output.jsonl
    # Parse and verify output
    grep -q '"type":"task_complete"' output.jsonl
```
