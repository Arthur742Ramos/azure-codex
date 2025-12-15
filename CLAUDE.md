# CLAUDE.md - AI Assistant Guide for Azure Codex

This document provides context and instructions for AI assistants (Claude, etc.) working on the Azure Codex codebase.

## Project Overview

**Azure Codex** is a fork of [OpenAI's Codex CLI](https://github.com/openai/codex) optimized for Azure OpenAI Service. It provides a terminal-based AI coding assistant that integrates with Azure's enterprise AI infrastructure.

### Key Differentiators from OpenAI Codex

| Aspect | OpenAI Codex | Azure Codex |
|--------|--------------|-------------|
| Authentication | ChatGPT/API Key | Azure Entra ID (CLI, Managed Identity, Service Principal) |
| Endpoint | api.openai.com | Azure OpenAI endpoints |
| Model Discovery | OpenAI models API | Azure deployments API |
| Wire API | Responses API | Chat Completions API |

## Project Structure

```
azure-codex/
├── codex-rs/                    # Main Rust implementation
│   ├── core/                    # Core library (most development happens here)
│   │   ├── src/
│   │   │   ├── azure/           # Azure-specific code
│   │   │   │   ├── mod.rs       # Module exports
│   │   │   │   └── deployments.rs  # Azure deployment discovery
│   │   │   ├── auth/            # Authentication
│   │   │   │   ├── azure.rs     # Azure Entra ID auth implementation
│   │   │   │   └── azure_config.rs # Azure auth configuration
│   │   │   ├── config/          # Configuration loading
│   │   │   ├── openai_models/   # Model management
│   │   │   │   └── models_manager.rs  # Handles both OpenAI and Azure models
│   │   │   ├── api_bridge.rs    # API abstraction layer
│   │   │   └── conversation_manager.rs
│   │   └── Cargo.toml
│   ├── cli/                     # CLI entry point
│   ├── tui/                     # Terminal UI (legacy)
│   ├── tui2/                    # Terminal UI (current, preferred)
│   ├── exec/                    # Non-interactive execution mode
│   └── protocol/                # Protocol definitions
└── README.md
```

## Key Components

### 1. Azure Authentication (`core/src/auth/`)

- **`azure.rs`**: Implements Azure Entra ID authentication using the `azure_identity` crate
- **`azure_config.rs`**: Configuration structures for Azure auth modes
- Supports: Azure CLI, Managed Identity, Service Principal, Device Code Flow, Environment Credentials

### 2. Azure Deployment Discovery (`core/src/azure/`)

- **`deployments.rs`**: Discovers Azure OpenAI deployments using Azure CLI
- Extracts account name from endpoint URL
- Discovers resource group automatically
- Filters to GPT models only
- Converts deployments to `ModelPreset` for the UI

### 3. Models Manager (`core/src/openai_models/models_manager.rs`)

- Handles model listing for both OpenAI and Azure
- `ModelsManager::with_azure_endpoint()` creates Azure-aware instance
- `is_azure()` method to check if using Azure backend
- Dynamic model switching via `Op::OverrideTurnContext`

### 4. Configuration (`core/src/config/`)

Minimal Azure config requires only 2 lines:
```toml
azure_endpoint = "https://your-resource.openai.azure.com"
model = "gpt-4o"
```

### 5. API Bridge (`core/src/api_bridge.rs`)

- Abstracts differences between OpenAI and Azure APIs
- Azure uses Chat Completions API (not Responses API)
- Handles authentication header injection

## Development Guidelines

### Building

```bash
# Debug build
cargo build -p codex-cli

# Release build
cargo build -p codex-cli --release

# Run tests
cargo test
```

### Code Style

- Follow existing Rust idioms and patterns in the codebase
- Use `tracing` for logging (not `println!`)
- Handle errors with `Result` types, avoid `unwrap()` in production code
- Document public APIs with doc comments

### Rust Linting Rules (Clippy & Cargo Fmt)

This project enforces strict Clippy lints and formatting. CI will fail if these rules are violated.

#### Cargo Fmt Rules

The project uses `imports_granularity = Item` (requires nightly, but CI enforces it). Key formatting rules:

1. **Imports must be individual** - No grouped imports:
   ```rust
   // ❌ BAD
   use std::sync::{Arc, RwLock};

   // ✅ GOOD
   use std::sync::Arc;
   use std::sync::RwLock;
   ```

2. **Method chains** - Long chains should be multi-line:
   ```rust
   // ❌ BAD (if line is too long)
   let result = self.client.post(&url).form(&params).send().await?;

   // ✅ GOOD
   let result = self
       .client
       .post(&url)
       .form(&params)
       .send()
       .await?;
   ```

3. **Function calls with multiple args** - Break into multiple lines when long:
   ```rust
   // ❌ BAD
   ConversationManager::with_azure_endpoint(auth_manager.clone(), SessionSource::Exec, azure_endpoint)

   // ✅ GOOD
   ConversationManager::with_azure_endpoint(
       auth_manager.clone(),
       SessionSource::Exec,
       azure_endpoint,
   )
   ```

4. **Macro calls like `tokio::join!`** - Multi-line for multiple args:
   ```rust
   // ✅ GOOD
   let (a, b, c, d) = tokio::join!(
       future_a,
       future_b,
       future_c,
       future_d
   );
   ```

#### Clippy Lints (Denied in This Project)

The workspace denies these lints (see `Cargo.toml`):

1. **`unwrap_used` / `expect_used`** - No `.unwrap()` or `.expect()` in production code:
   ```rust
   // ❌ BAD
   let value = result.unwrap();

   // ✅ GOOD - Use ? operator or handle the error
   let value = result?;
   let value = result.unwrap_or_default();

   // ✅ OK in tests/UI code - Add allow attribute
   #![allow(clippy::unwrap_used)]  // At module level for UI widgets
   ```

2. **`uninlined_format_args`** - Inline variables in format strings:
   ```rust
   // ❌ BAD
   format!("Error: {}", e)
   format!("Value: {} and {}", x, y)

   // ✅ GOOD
   format!("Error: {e}")
   format!("Value: {x} and {y}")
   ```

3. **`collapsible_if`** - Combine nested if statements:
   ```rust
   // ❌ BAD
   if let Ok(guard) = lock.read() {
       if let Some(value) = guard.as_ref() {
           // ...
       }
   }

   // ✅ GOOD
   if let Ok(guard) = lock.read()
       && let Some(value) = guard.as_ref()
   {
       // ...
   }
   ```

4. **`redundant_clone`** - Don't clone when moving:
   ```rust
   // ❌ BAD
   if let Some(x) = option.clone() { ... }  // option is not used after

   // ✅ GOOD
   if let Some(x) = option { ... }
   ```

5. **`redundant_closure`** - Use method references:
   ```rust
   // ❌ BAD
   .map(|a| a.as_ref())

   // ✅ GOOD
   .map(AsRef::as_ref)
   ```

6. **`manual_map`** - Use `.map()` instead of if-else:
   ```rust
   // ❌ BAD
   let result = if let Some(x) = option { Some(f(x)) } else { None };

   // ✅ GOOD
   let result = option.map(f);
   ```

#### TUI-Specific Rules (tui2 crate)

1. **No `.yellow()` color** - Yellow is disallowed; use ANSI colors:
   ```rust
   // ❌ BAD - disallowed_methods
   "Warning".yellow()

   // ✅ GOOD - Use ANSI colors
   "Warning".red()
   "Warning".cyan()
   ```

2. **No `Color::Rgb()`** - Use ANSI colors for better terminal compatibility:
   ```rust
   // ❌ BAD - disallowed_methods
   "Text".fg(Color::Rgb(255, 165, 0))

   // ✅ GOOD - Use ANSI colors
   "Text".red()
   "Text".cyan()
   "Text".green()
   ```

#### Cargo.toml Rules (cargo-shear)

1. **Dependencies vs dev-dependencies** - Test-only deps go in `[dev-dependencies]`:
   ```toml
   # ❌ BAD - test crates in [dependencies]
   [dependencies]
   tempfile = "3"
   wiremock = "0.6"

   # ✅ GOOD
   [dev-dependencies]
   tempfile = { workspace = true }
   wiremock = { workspace = true }
   ```

2. **No duplicate dependencies** - Don't list the same dep in both sections

3. **Keep cargo-shear ignores minimal** - Only add to `[package.metadata.cargo-shear].ignored` when truly needed

### Important Patterns

#### Azure Endpoint Detection
When `config.azure_endpoint` is `Some(...)`, use Azure-specific code paths:

```rust
if let Some(azure_endpoint) = config.azure_endpoint.clone() {
    // Azure path
    ConversationManager::with_azure_endpoint(auth_manager, session_source, azure_endpoint)
} else {
    // OpenAI path
    ConversationManager::new(auth_manager, session_source)
}
```

#### Dynamic Model Switching
Models can be changed without restart using:
```rust
Op::OverrideTurnContext(TurnContextOverride {
    model: Some(new_model),
    ..Default::default()
})
```

### Windows Considerations

- Azure CLI on Windows requires `cmd /C az` (not just `az`)
- The codebase handles this automatically in `azure.rs` and `deployments.rs`

## Common Tasks

### Adding a New Azure Feature

1. Add types/structs in `core/src/azure/`
2. Update `ModelsManager` if model-related
3. Update entry points (`tui/`, `tui2/`, `exec/`) if needed
4. Add tests in the relevant module

### Modifying Authentication

1. Update `core/src/auth/azure_config.rs` for config changes
2. Update `core/src/auth/azure.rs` for implementation
3. Update `core/src/config/` if new config fields needed

### Adding New Slash Commands

1. Add handler in `tui2/src/slash_commands/`
2. Register in the command dispatch logic
3. Update help text

## Testing

### Local Testing with Azure

1. Login: `az login`
2. Set config:
   ```toml
   azure_endpoint = "https://your-resource.openai.azure.com"
   model = "your-deployment-name"
   ```
3. Run: `./target/release/codex`

### Using Test Config

```bash
export AZURE_CODEX_HOME=/path/to/test-config
./target/release/codex
```

## Troubleshooting

### Common Issues

1. **"Failed to run az CLI"**: Azure CLI not installed or not in PATH
2. **404 errors**: Model/deployment name doesn't match Azure deployment
3. **Auth failures**: Not logged in (`az login`) or insufficient permissions

### Debug Logging

Set `RUST_LOG=debug` for verbose output:
```bash
RUST_LOG=debug ./target/release/codex
```

## Dependencies

Key crates used:
- `azure_identity` - Azure authentication
- `reqwest` - HTTP client
- `tokio` - Async runtime
- `serde` - Serialization
- `ratatui` - Terminal UI
- `tracing` - Logging

## Contact

- Repository: https://github.com/Arthur742Ramos/azure-codex
- Original Codex: https://github.com/openai/codex
