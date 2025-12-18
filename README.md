<p align="center">
  <img src="./.github/codex-cli-splash.png" alt="Azure Codex CLI" width="80%" />
</p>

<h1 align="center">Azure Codex</h1>

<p align="center">
  <strong>A fork of OpenAI's Codex CLI optimized for Azure OpenAI Service</strong>
</p>

<p align="center">
  <a href="#features">Features</a> •
  <a href="#quickstart">Quickstart</a> •
  <a href="#configuration">Configuration</a> •
  <a href="#authentication">Authentication</a> •
  <a href="#commands">Commands</a> •
  <a href="#contributing">Contributing</a>
</p>

---

## Overview

**Azure Codex** is a dedicated fork of [OpenAI's Codex CLI](https://github.com/openai/codex) designed specifically for Azure OpenAI Service. It provides seamless integration with Azure's enterprise-grade AI infrastructure, supporting Azure Entra ID (formerly Azure AD) authentication and automatic discovery of your Azure OpenAI deployments.

### Why Azure Codex?

- **Enterprise Ready**: Built for Azure's enterprise security and compliance requirements
- **Zero-Config Setup**: Interactive wizard guides you through first-run configuration
- **Azure Entra ID**: Native support for Azure authentication (CLI, Managed Identity, Service Principal)
- **Dynamic Model Switching**: Change models and endpoints on-the-fly without restarting
- **Deployment Discovery**: Automatically discovers your Azure OpenAI deployments
- **Reasoning Effort Control**: Configure reasoning intensity for GPT-5/o-series models

---

## Features

### Interactive Setup Wizard

On first run, Azure Codex guides you through configuration with an interactive wizard:

1. **Enter your Azure endpoint** - Just type your resource name (e.g., `my-openai-resource`)
2. **Select a model** - Automatically discovers and lists your GPT deployments
3. **Choose reasoning effort** - For reasoning models, select Low/Medium/High intensity

No manual config file editing required!

### Azure-Native Authentication

Azure Codex supports multiple authentication methods through Azure Entra ID:

| Method | Use Case |
|--------|----------|
| **Azure CLI** | Development machines (`az login`) |
| **Managed Identity** | Azure VMs, App Service, Functions |
| **Service Principal (Secret)** | CI/CD pipelines, automation |
| **Service Principal (Certificate)** | High-security environments |
| **Device Code Flow** | Headless/SSH environments |
| **Environment Credentials** | Container deployments |

Supports Azure Public, US Government, and China clouds.

### Dynamic Model & Endpoint Switching

Change your model or endpoint without restarting:

- **`/model`** - Switch between GPT deployments instantly
- **`/endpoint`** - Connect to a different Azure OpenAI resource

### Reasoning Effort Control

For GPT-5 and o-series reasoning models, configure how much "thinking" the model does:

| Effort | Description | Use Case |
|--------|-------------|----------|
| **Low** | Quick responses, minimal reasoning | Simple tasks, fast iteration |
| **Medium** | Balanced reasoning | General development |
| **High** | Deep reasoning, thorough analysis | Complex problems, architecture |

Configure via `/model` command or in config:
```toml
model_reasoning_effort = "medium"  # low, medium, high
```

### All Original Codex Features

- Interactive TUI with syntax highlighting
- Sandboxed command execution (Windows & Linux)
- MCP (Model Context Protocol) support
- Git integration
- File mentions with `@`
- Skills system with `$`
- Session persistence and resume

---

## Quickstart

### Prerequisites

- **Azure CLI**: Install from [aka.ms/installazurecli](https://aka.ms/installazurecli)
- **Azure OpenAI Resource**: With at least one GPT model deployment
- **Rust toolchain** (for building from source): Install from [rustup.rs](https://rustup.rs)

### Installation

#### Build from Source

```bash
# Clone the repository
git clone https://github.com/Arthur742Ramos/azure-codex.git
cd azure-codex/codex-rs

# Build release binary
cargo build -p codex-cli --release

# Binary is at target/release/codex (or codex.exe on Windows)
```

### First Run

1. **Login to Azure**:
   ```bash
   az login
   ```

2. **Run Azure Codex**:
   ```bash
   ./target/release/codex    # Linux/macOS
   .\target\release\codex.exe  # Windows
   ```

3. **Follow the setup wizard**:
   - Enter your Azure OpenAI resource name (e.g., `my-openai-resource`)
   - Select a model from your discovered deployments
   - Choose reasoning effort (for reasoning models)

That's it! The wizard saves your configuration automatically.

### Manual Configuration (Alternative)

If you prefer manual setup, create `~/.azure-codex/config.toml`:

```toml
azure_endpoint = "https://your-resource.openai.azure.com"
model = "gpt-4o"  # Your deployment name
```

---

## Configuration

### Config File Location

| Platform | Path |
|----------|------|
| Linux/macOS | `~/.azure-codex/config.toml` |
| Windows | `%USERPROFILE%\.azure-codex\config.toml` |

Override with `AZURE_CODEX_HOME` environment variable.

### Minimal Configuration

```toml
# ~/.azure-codex/config.toml
azure_endpoint = "https://your-resource.openai.azure.com"
model = "gpt-4o"
```

### Full Configuration Options

```toml
# Azure OpenAI endpoint (required for Azure)
azure_endpoint = "https://your-resource.openai.azure.com"

# Model/deployment name (required)
model = "gpt-4o"

# Reasoning effort for GPT-5/o-series models (optional)
# Values: "low", "medium", "high"
model_reasoning_effort = "medium"

# API version (optional, defaults to latest preview)
azure_api_version = "2025-04-01-preview"

# Azure authentication configuration (optional)
[azure_auth]
# Auth mode: "default", "azure_cli", "managed_identity", "client_secret",
#            "client_certificate", "device_code", "environment"
mode = "default"  # "default" tries all methods in order

# Azure cloud: "public" (default), "us_government", "china"
# cloud = "public"

# For service principal authentication
# tenant_id = "your-tenant-id"
# client_id = "your-client-id"
# client_secret = "your-client-secret"
# certificate_path = "/path/to/cert.pem"  # For client_certificate mode

# Approval policy: "on-failure", "unless-allow-listed", "never"
approval_policy = "on-failure"

# Sandbox policy
[sandbox]
# "read-only", "workspace-write", "full-access"
permissions = "read-only"
```

### Environment Variables

| Variable | Description |
|----------|-------------|
| `AZURE_CODEX_HOME` | Override config directory (default: `~/.azure-codex`) |
| `AZURE_OPENAI_API_KEY` | Use API key instead of Entra ID auth |
| `AZURE_TENANT_ID` | Tenant ID for service principal auth |
| `AZURE_CLIENT_ID` | Client ID for service principal auth |
| `AZURE_CLIENT_SECRET` | Client secret for service principal auth |

---

## Authentication

### Default Mode (Recommended)

The default authentication mode tries multiple methods in order:

1. **Azure CLI** - Uses `az login` credentials
2. **Managed Identity** - For Azure-hosted workloads
3. **Environment Credentials** - From environment variables
4. **Device Code Flow** - Interactive browser login

```toml
[azure_auth]
mode = "default"
```

### Azure CLI Authentication

Best for local development:

```bash
# Login to Azure
az login

# Verify you're logged in
az account show
```

No additional config needed - Azure Codex will use your CLI credentials.

### Managed Identity

For Azure VMs, App Service, Functions, or AKS:

```toml
[azure_auth]
mode = "managed_identity"
```

### Service Principal

For CI/CD and automation:

```toml
[azure_auth]
mode = "client_secret"
tenant_id = "your-tenant-id"
client_id = "your-client-id"
client_secret = "your-client-secret"
```

Or use environment variables:
```bash
export AZURE_TENANT_ID="your-tenant-id"
export AZURE_CLIENT_ID="your-client-id"
export AZURE_CLIENT_SECRET="your-client-secret"
```

### Service Principal with Certificate

For certificate-based authentication:

```toml
[azure_auth]
mode = "client_certificate"
tenant_id = "your-tenant-id"
client_id = "your-client-id"
certificate_path = "/path/to/certificate.pem"
# certificate_password = "optional-password"  # If certificate is encrypted
```

### API Key Authentication

If you prefer API key authentication:

```bash
export AZURE_OPENAI_API_KEY="your-api-key"
```

---

## Commands

### Slash Commands

| Command | Description |
|---------|-------------|
| `/model` | Choose what model and reasoning effort to use |
| `/endpoint` | Show or change the Azure OpenAI endpoint |
| `/approvals` | Choose what Codex can do without approval |
| `/skills` | Use skills to improve how Codex performs specific tasks |
| `/review` | Review current changes and find issues |
| `/new` | Start a new chat during a conversation |
| `/resume` | Resume a saved chat |
| `/init` | Create an AGENTS.md file with instructions for Codex |
| `/compact` | Summarize conversation to prevent hitting the context limit |
| `/undo` | Ask Codex to undo a turn |
| `/diff` | Show git diff (including untracked files) |
| `/mention` | Mention a file |
| `/status` | Show current session configuration and token usage |
| `/mcp` | List configured MCP tools |
| `/togglemouse` | Toggle mouse capture for native text selection |
| `/logout` | Log out of Codex |
| `/feedback` | Send logs to maintainers |
| `/quit` | Exit Azure Codex |

### Non-Interactive Mode

Run commands without the TUI:

```bash
# Simple prompt
codex exec "Create a hello world script"

# With specific model
codex exec --model gpt-5.1-codex "Refactor this function"

# Skip git repo check
codex exec --skip-git-repo-check "Explain this code"
```

---

## Model Switching

### Dynamic Model Selection

Change models on-the-fly using `/model`:

1. Type `/model` in the TUI
2. Select a different GPT deployment
3. Optionally adjust reasoning effort
4. Your next message uses the new model immediately

No restart required!

### Dynamic Endpoint Switching

Connect to a different Azure OpenAI resource using `/endpoint`:

1. Type `/endpoint` in the TUI
2. Enter the new resource name
3. Select a model from the new resource
4. Continue your session with the new endpoint

### Supported Models

Azure Codex filters to GPT models only. Your available models depend on your Azure OpenAI deployments:

- GPT-4 series (gpt-4, gpt-4o, gpt-4-turbo)
- GPT-5 series (gpt-5, gpt-5.1, gpt-5.2)
- GPT Codex models (gpt-5-codex, gpt-5.1-codex-max)
- o-series reasoning models (o1, o3, o4-mini)

---

## Architecture

```
azure-codex/
├── codex-rs/                 # Rust implementation
│   ├── core/                 # Core library
│   │   ├── src/
│   │   │   ├── azure/        # Azure-specific code
│   │   │   │   ├── deployments.rs  # Deployment discovery
│   │   │   │   └── mod.rs
│   │   │   ├── auth/
│   │   │   │   ├── azure.rs        # Azure Entra ID auth
│   │   │   │   └── azure_config.rs # Auth configuration
│   │   │   ├── config/       # Configuration loading
│   │   │   └── ...
│   ├── cli/                  # CLI entry point
│   ├── tui2/                 # Terminal UI (current)
│   │   └── src/onboarding/   # First-run setup wizard
│   └── exec/                 # Non-interactive mode
└── docs/                     # Documentation
```

---

## Development

### Building

```bash
# Quick compilation check (fastest)
cargo check -p codex-cli

# Debug build (fast, for testing)
cargo build -p codex-cli

# Release build (optimized, for production)
cargo build -p codex-cli --release

# Run tests
cargo test
```

### Build Times

| Command | Time | Use Case |
|---------|------|----------|
| `cargo check -p codex-cli` | ~1 min | Verify compilation |
| `cargo build -p codex-cli` | ~3 min | Debug build for testing |
| `cargo build -p codex-cli --release` | ~10 min | Optimized production build |

### Testing with Custom Config

```bash
# Set custom config directory
export AZURE_CODEX_HOME=/path/to/test-config

# Run with test config
./target/release/codex
```

---

## Troubleshooting

### "Failed to run az CLI"

**Windows**: Ensure Azure CLI is installed and `az.cmd` is in PATH.

**Linux/macOS**: Ensure `az` command is available.

```bash
# Verify Azure CLI
az --version

# Login if needed
az login
```

### "Resource not found" (404)

Check your `model` config matches an actual deployment name in your Azure OpenAI resource.

```bash
# List your deployments
az cognitiveservices account deployment list \
  --name your-resource-name \
  --resource-group your-rg \
  -o table
```

### "Authentication failed"

Ensure you're logged in and have access to the Azure OpenAI resource:

```bash
# Check current account
az account show

# Get access token (for debugging)
az account get-access-token --scope https://cognitiveservices.azure.com/.default
```

### Loading animation freezes

This can happen during Azure deployment discovery. The CLI makes Azure CLI calls to discover your deployments, which may take a few seconds depending on network conditions.

---

## Differences from OpenAI Codex

| Feature | OpenAI Codex | Azure Codex |
|---------|--------------|-------------|
| Authentication | ChatGPT/API Key | Azure Entra ID |
| Endpoint | api.openai.com | Your Azure endpoint |
| Model Discovery | OpenAI models API | Azure deployments API |
| Wire API | Responses API | Chat Completions API |
| First-Run Setup | Manual config | Interactive wizard |
| Endpoint Switching | Restart required | `/endpoint` command |

---

## Contributing

Contributions are welcome! Please see [CONTRIBUTING.md](./docs/contributing.md) for guidelines.

### Key Areas for Contribution

- Additional Azure authentication methods
- Azure OpenAI feature parity
- Documentation improvements
- Bug fixes and performance improvements

---

## License

This project is licensed under the [Apache-2.0 License](LICENSE).

---

## Acknowledgments

- [OpenAI Codex CLI](https://github.com/openai/codex) - The original project this fork is based on
- [Azure OpenAI Service](https://azure.microsoft.com/products/ai-services/openai-service) - Microsoft's enterprise AI platform

---

<p align="center">
  <strong>Azure Codex</strong> - Enterprise-grade AI coding assistant powered by Azure OpenAI
</p>
