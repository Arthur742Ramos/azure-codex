# Azure Codex - Implementation Plan

## Executive Summary

This document outlines the implementation plan for forking OpenAI's Codex CLI to create **Azure Codex**, a version optimized for Azure AI Foundry / Azure OpenAI Service with enterprise-grade authentication support.

## Current State Analysis

### Architecture Overview

The Codex codebase is organized as a Rust workspace with the following key components:

```
codex-rs/
├── codex-api/          # Low-level API client library
│   └── src/
│       ├── auth.rs     # AuthProvider trait, add_auth_headers()
│       ├── provider.rs # Provider struct, Azure detection
│       └── endpoint/   # API endpoint implementations
├── core/               # Core business logic
│   └── src/
│       ├── auth.rs     # CodexAuth, AuthManager, token refresh
│       ├── client.rs   # ModelClient for API calls
│       ├── api_bridge.rs           # CoreAuthProvider bridge
│       └── model_provider_info.rs  # Provider configuration
└── cli/                # CLI entry point
```

### Key Files to Modify

| File | Current Function | Required Changes |
|------|-----------------|------------------|
| `codex-api/src/auth.rs` | `AuthProvider` trait with `bearer_token()` only | Add `api_key()`, `auth_header_type()` |
| `codex-api/src/provider.rs` | Hardcoded Azure hostname detection | Make configurable, add explicit flag |
| `core/src/auth.rs` | OpenAI-only OAuth flow | Add Azure Entra ID, managed identity |
| `core/src/model_provider_info.rs` | Provider config struct | Add `auth_type`, `is_azure` fields |
| `core/src/api_bridge.rs` | `CoreAuthProvider` implementation | Support multiple auth header types |
| `core/src/client.rs` | 401 handling for ChatGPT only | Add Azure token refresh on 401 |

### Current Limitations (from GitHub Issues)

1. **#1056** - No Azure Entra ID (AAD) authentication
2. **#3048** - Hardcoded hostname detection breaks custom APIM endpoints
3. **#2522** - No token refresh mechanism for Azure
4. **#4278** - No OAuth2/Bearer token support for secure production
5. **#6849** - OAuth fails behind corporate proxies

---

## Implementation Phases

### Phase 1: Auth Header Flexibility (Priority: Critical)

**Goal**: Support both `Authorization: Bearer` and `api-key:` header formats.

#### 1.1 Extend AuthProvider Trait

```rust
// codex-api/src/auth.rs

/// Authentication header type for the request
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthHeaderType {
    #[default]
    Bearer,      // Authorization: Bearer <token>
    ApiKey,      // api-key: <key>
    ApiKeyHeader(String), // Custom header name
}

pub trait AuthProvider: Send + Sync {
    fn bearer_token(&self) -> Option<String>;
    fn api_key(&self) -> Option<String> { None }
    fn auth_header_type(&self) -> AuthHeaderType { AuthHeaderType::Bearer }
    fn account_id(&self) -> Option<String> { None }
}

pub(crate) fn add_auth_headers<A: AuthProvider>(auth: &A, mut req: Request) -> Request {
    match auth.auth_header_type() {
        AuthHeaderType::Bearer => {
            if let Some(token) = auth.bearer_token() {
                if let Ok(header) = format!("Bearer {token}").parse() {
                    let _ = req.headers.insert(http::header::AUTHORIZATION, header);
                }
            }
        }
        AuthHeaderType::ApiKey => {
            if let Some(key) = auth.api_key().or(auth.bearer_token()) {
                if let Ok(header) = key.parse() {
                    let _ = req.headers.insert("api-key", header);
                }
            }
        }
        AuthHeaderType::ApiKeyHeader(name) => {
            if let Some(key) = auth.api_key().or(auth.bearer_token()) {
                if let Ok(header_name) = name.parse::<http::HeaderName>()
                    && let Ok(header) = key.parse() {
                    let _ = req.headers.insert(header_name, header);
                }
            }
        }
    }
    // ... rest unchanged
}
```

#### 1.2 Update ModelProviderInfo

```rust
// core/src/model_provider_info.rs

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthType {
    #[default]
    Bearer,
    ApiKey,
    AzureEntra,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ModelProviderInfo {
    // ... existing fields ...

    /// Authentication type for this provider
    #[serde(default)]
    pub auth_type: AuthType,

    /// Explicitly mark this as an Azure endpoint (bypasses hostname detection)
    #[serde(default)]
    pub is_azure: bool,

    /// Custom auth header name (if auth_type is not Bearer)
    pub auth_header_name: Option<String>,
}
```

#### 1.3 Update CoreAuthProvider

```rust
// core/src/api_bridge.rs

use codex_api::AuthHeaderType;

pub(crate) struct CoreAuthProvider {
    token: Option<String>,
    account_id: Option<String>,
    auth_header_type: AuthHeaderType,
}

impl ApiAuthProvider for CoreAuthProvider {
    fn bearer_token(&self) -> Option<String> {
        self.token.clone()
    }

    fn api_key(&self) -> Option<String> {
        self.token.clone()
    }

    fn auth_header_type(&self) -> AuthHeaderType {
        self.auth_header_type
    }

    fn account_id(&self) -> Option<String> {
        self.account_id.clone()
    }
}
```

**Files to modify**:
- `codex-api/src/auth.rs`
- `core/src/model_provider_info.rs`
- `core/src/api_bridge.rs`

---

### Phase 2: Azure Entra ID Authentication (Priority: High)

**Goal**: Support Azure AD / Entra ID authentication including device code flow and managed identity.

#### 2.1 Add Azure Identity Dependency

```toml
# codex-rs/Cargo.toml

[workspace.dependencies]
azure_identity = "0.20"
azure_core = "0.20"
```

#### 2.2 Create Azure Auth Module

```rust
// core/src/auth/azure.rs (new file)

use azure_identity::{
    DefaultAzureCredential,
    DeviceCodeCredential,
    ManagedIdentityCredential,
    ClientSecretCredential,
    TokenCredentialOptions,
};
use azure_core::auth::TokenCredential;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Azure authentication modes
#[derive(Debug, Clone)]
pub enum AzureAuthMode {
    /// Use DefaultAzureCredential (tries multiple methods)
    Default,
    /// Device code flow for interactive login
    DeviceCode { tenant_id: String, client_id: String },
    /// Managed Identity (for Azure VMs, App Service, etc.)
    ManagedIdentity { client_id: Option<String> },
    /// Service Principal with client secret
    ClientSecret { tenant_id: String, client_id: String, client_secret: String },
}

pub struct AzureAuth {
    credential: Arc<dyn TokenCredential>,
    scope: String,
    cached_token: RwLock<Option<CachedToken>>,
}

struct CachedToken {
    token: String,
    expires_at: std::time::Instant,
}

impl AzureAuth {
    pub fn new(mode: AzureAuthMode, scope: &str) -> Result<Self, AzureAuthError> {
        let credential: Arc<dyn TokenCredential> = match mode {
            AzureAuthMode::Default => {
                Arc::new(DefaultAzureCredential::default())
            }
            AzureAuthMode::DeviceCode { tenant_id, client_id } => {
                Arc::new(DeviceCodeCredential::new(
                    tenant_id,
                    client_id,
                    Default::default(),
                )?)
            }
            AzureAuthMode::ManagedIdentity { client_id } => {
                let mut opts = TokenCredentialOptions::default();
                if let Some(id) = client_id {
                    opts.managed_identity_client_id = Some(id);
                }
                Arc::new(ManagedIdentityCredential::new(opts)?)
            }
            AzureAuthMode::ClientSecret { tenant_id, client_id, client_secret } => {
                Arc::new(ClientSecretCredential::new(
                    tenant_id,
                    client_id,
                    client_secret,
                    Default::default(),
                )?)
            }
        };

        Ok(Self {
            credential,
            scope: scope.to_string(),
            cached_token: RwLock::new(None),
        })
    }

    pub async fn get_token(&self) -> Result<String, AzureAuthError> {
        // Check cache first
        {
            let cache = self.cached_token.read().await;
            if let Some(cached) = &*cache {
                if cached.expires_at > std::time::Instant::now() + std::time::Duration::from_secs(60) {
                    return Ok(cached.token.clone());
                }
            }
        }

        // Fetch new token
        let token_response = self.credential
            .get_token(&[&self.scope])
            .await?;

        let token = token_response.token.secret().to_string();

        // Cache it
        {
            let mut cache = self.cached_token.write().await;
            *cache = Some(CachedToken {
                token: token.clone(),
                expires_at: std::time::Instant::now() + std::time::Duration::from_secs(3000),
            });
        }

        Ok(token)
    }

    pub async fn refresh_token(&self) -> Result<String, AzureAuthError> {
        // Clear cache and get fresh token
        {
            let mut cache = self.cached_token.write().await;
            *cache = None;
        }
        self.get_token().await
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AzureAuthError {
    #[error("Azure credential error: {0}")]
    Credential(#[from] azure_identity::Error),
    #[error("Token acquisition failed: {0}")]
    TokenAcquisition(String),
}
```

#### 2.3 Extend AuthMode

```rust
// core/src/auth.rs

#[derive(Debug, Clone, PartialEq)]
pub enum AuthMode {
    ApiKey,
    ChatGPT,
    AzureEntra(AzureAuthConfig),
}

#[derive(Debug, Clone, PartialEq)]
pub struct AzureAuthConfig {
    pub mode: AzureAuthMode,
    pub scope: String,
}

impl Default for AzureAuthConfig {
    fn default() -> Self {
        Self {
            mode: AzureAuthMode::Default,
            scope: "https://cognitiveservices.azure.com/.default".to_string(),
        }
    }
}
```

#### 2.4 Update CodexAuth

```rust
// core/src/auth.rs

pub struct CodexAuth {
    pub mode: AuthMode,
    pub(crate) api_key: Option<String>,
    pub(crate) auth_dot_json: Arc<Mutex<Option<AuthDotJson>>>,
    storage: Arc<dyn AuthStorageBackend>,
    pub(crate) client: CodexHttpClient,
    // NEW: Azure auth handler
    pub(crate) azure_auth: Option<Arc<AzureAuth>>,
}

impl CodexAuth {
    pub async fn get_token(&self) -> Result<String, std::io::Error> {
        match &self.mode {
            AuthMode::ApiKey => Ok(self.api_key.clone().unwrap_or_default()),
            AuthMode::ChatGPT => {
                let id_token = self.get_token_data().await?.access_token;
                Ok(id_token)
            }
            AuthMode::AzureEntra(_) => {
                let azure = self.azure_auth.as_ref()
                    .ok_or_else(|| std::io::Error::other("Azure auth not configured"))?;
                azure.get_token().await
                    .map_err(|e| std::io::Error::other(e.to_string()))
            }
        }
    }

    pub async fn refresh_token(&self) -> Result<String, RefreshTokenError> {
        match &self.mode {
            AuthMode::AzureEntra(_) => {
                let azure = self.azure_auth.as_ref()
                    .ok_or_else(|| RefreshTokenError::other_with_message("Azure auth not configured"))?;
                azure.refresh_token().await
                    .map_err(|e| RefreshTokenError::other_with_message(e.to_string()))
            }
            AuthMode::ChatGPT => {
                // ... existing ChatGPT refresh logic ...
            }
            AuthMode::ApiKey => {
                Err(RefreshTokenError::other_with_message("API key auth does not support refresh"))
            }
        }
    }
}
```

**Files to create/modify**:
- `core/src/auth/azure.rs` (new)
- `core/src/auth.rs`
- `codex-rs/Cargo.toml`

---

### Phase 3: Token Refresh on 401 (Priority: High)

**Goal**: Automatically refresh Azure tokens when receiving 401 responses.

#### 3.1 Update handle_unauthorized in client.rs

```rust
// core/src/client.rs

async fn handle_unauthorized(
    status: StatusCode,
    refreshed: &mut bool,
    auth_manager: &Option<Arc<AuthManager>>,
    auth: &Option<crate::auth::CodexAuth>,
) -> Result<()> {
    if *refreshed {
        return Err(map_unauthorized_status(status));
    }

    if let Some(manager) = auth_manager.as_ref()
        && let Some(auth) = auth.as_ref()
    {
        // Handle both ChatGPT and Azure Entra
        match &auth.mode {
            AuthMode::ChatGPT | AuthMode::AzureEntra(_) => {
                match manager.refresh_token().await {
                    Ok(_) => {
                        *refreshed = true;
                        Ok(())
                    }
                    Err(RefreshTokenError::Permanent(failed)) => {
                        Err(CodexErr::RefreshTokenFailed(failed))
                    }
                    Err(RefreshTokenError::Transient(other)) => {
                        Err(CodexErr::Io(other))
                    }
                }
            }
            AuthMode::ApiKey => {
                // API keys can't be refreshed
                Err(map_unauthorized_status(status))
            }
        }
    } else {
        Err(map_unauthorized_status(status))
    }
}
```

**Files to modify**:
- `core/src/client.rs`

---

### Phase 4: Custom Endpoint/APIM Support (Priority: Medium)

**Goal**: Support custom Azure API Management endpoints that don't match hardcoded hostname patterns.

#### 4.1 Make Azure Detection Configurable

```rust
// core/src/model_provider_info.rs

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ModelProviderInfo {
    // ... existing fields ...

    /// Explicitly mark this as an Azure endpoint
    #[serde(default)]
    pub is_azure: bool,

    /// Skip hostname-based Azure detection
    #[serde(default)]
    pub skip_azure_detection: bool,
}

impl ModelProviderInfo {
    pub fn is_azure_endpoint(&self) -> bool {
        // Explicit flag takes precedence
        if self.is_azure {
            return true;
        }

        // Skip automatic detection if requested
        if self.skip_azure_detection {
            return false;
        }

        // Fall back to hostname detection
        self.name.eq_ignore_ascii_case("azure") ||
        self.base_url.as_ref().map_or(false, |url| is_azure_url(url))
    }
}
```

#### 4.2 Update Provider Detection

```rust
// codex-api/src/provider.rs

impl Provider {
    pub fn is_azure_responses_endpoint(&self) -> bool {
        if self.wire != WireApi::Responses {
            return false;
        }

        // Check explicit flag in headers (passed from ModelProviderInfo)
        if self.headers.get("x-azure-codex-is-azure")
            .map_or(false, |v| v == "true") {
            return true;
        }

        // Existing detection logic...
        if self.name.eq_ignore_ascii_case("azure") {
            return true;
        }

        self.base_url.to_ascii_lowercase().contains("openai.azure.")
            || matches_azure_responses_base_url(&self.base_url)
    }
}
```

**Files to modify**:
- `core/src/model_provider_info.rs`
- `codex-api/src/provider.rs`

---

### Phase 5: Configuration Examples

#### 5.1 Azure API Key Configuration

```toml
# ~/.codex/config.toml

model = "gpt-4o"  # Deployment name
model_provider = "azure"

[model_providers.azure]
name = "Azure OpenAI"
base_url = "https://YOUR_RESOURCE.openai.azure.com/openai/v1"
env_key = "AZURE_OPENAI_API_KEY"
wire_api = "responses"
auth_type = "api_key"
is_azure = true
```

#### 5.2 Azure Entra ID (Default Credential)

```toml
model = "gpt-4o"
model_provider = "azure-entra"

[model_providers.azure-entra]
name = "Azure OpenAI (Entra)"
base_url = "https://YOUR_RESOURCE.openai.azure.com/openai/v1"
wire_api = "responses"
auth_type = "azure_entra"
is_azure = true

[model_providers.azure-entra.azure_auth]
mode = "default"
scope = "https://cognitiveservices.azure.com/.default"
```

#### 5.3 Azure Entra ID (Device Code)

```toml
model = "gpt-4o"
model_provider = "azure-device"

[model_providers.azure-device]
name = "Azure OpenAI (Device Code)"
base_url = "https://YOUR_RESOURCE.openai.azure.com/openai/v1"
wire_api = "responses"
auth_type = "azure_entra"
is_azure = true

[model_providers.azure-device.azure_auth]
mode = "device_code"
tenant_id = "YOUR_TENANT_ID"
client_id = "YOUR_CLIENT_ID"
scope = "https://cognitiveservices.azure.com/.default"
```

#### 5.4 Azure Entra ID (Managed Identity)

```toml
model = "gpt-4o"
model_provider = "azure-mi"

[model_providers.azure-mi]
name = "Azure OpenAI (Managed Identity)"
base_url = "https://YOUR_RESOURCE.openai.azure.com/openai/v1"
wire_api = "responses"
auth_type = "azure_entra"
is_azure = true

[model_providers.azure-mi.azure_auth]
mode = "managed_identity"
# client_id = "USER_ASSIGNED_MI_CLIENT_ID"  # Optional for user-assigned MI
scope = "https://cognitiveservices.azure.com/.default"
```

#### 5.5 Custom APIM Endpoint

```toml
model = "gpt-4o"
model_provider = "azure-apim"

[model_providers.azure-apim]
name = "Azure APIM Gateway"
base_url = "https://mycompany-api.azure-api.net/openai/v1"
env_key = "APIM_SUBSCRIPTION_KEY"
wire_api = "responses"
auth_type = "api_key"
auth_header_name = "Ocp-Apim-Subscription-Key"
is_azure = true
skip_azure_detection = false
```

---

## Phase 6: Branding & Distribution

### 6.1 Package Naming

- npm: `@azure/codex` or `azure-codex`
- Binary: `azure-codex`
- GitHub: `azure-codex` (this repo)

### 6.2 Default Configuration

Create an Azure-first experience:

```rust
// core/src/model_provider_info.rs

pub fn built_in_model_providers() -> HashMap<String, ModelProviderInfo> {
    [
        ("azure", create_azure_provider()),  // Default
        ("azure-entra", create_azure_entra_provider()),
        ("openai", ModelProviderInfo::create_openai_provider()),
        (OLLAMA_OSS_PROVIDER_ID, create_oss_provider(...)),
        (LMSTUDIO_OSS_PROVIDER_ID, create_oss_provider(...)),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

fn create_azure_provider() -> ModelProviderInfo {
    ModelProviderInfo {
        name: "Azure OpenAI".into(),
        base_url: std::env::var("AZURE_OPENAI_ENDPOINT").ok(),
        env_key: Some("AZURE_OPENAI_API_KEY".into()),
        env_key_instructions: Some(
            "Set AZURE_OPENAI_ENDPOINT and AZURE_OPENAI_API_KEY from Azure Portal".into()
        ),
        wire_api: WireApi::Responses,
        auth_type: AuthType::ApiKey,
        is_azure: true,
        requires_openai_auth: false,
        // ...
    }
}
```

### 6.3 README Updates

- Update README.md with Azure-first quickstart
- Add Azure-specific troubleshooting section
- Include Azure AI Foundry setup guide

---

## Implementation Timeline

| Phase | Description | Effort | Dependencies |
|-------|-------------|--------|--------------|
| 1 | Auth Header Flexibility | 2-3 days | None |
| 2 | Azure Entra ID Support | 4-5 days | Phase 1 |
| 3 | Token Refresh on 401 | 1-2 days | Phase 2 |
| 4 | Custom Endpoint Support | 1-2 days | Phase 1 |
| 5 | Configuration Examples | 1 day | Phase 1-4 |
| 6 | Branding & Distribution | 2-3 days | All phases |

**Total Estimated Effort**: 2-3 weeks

---

## Testing Strategy

### Unit Tests
- Auth header generation for all types
- Azure token caching and refresh
- Provider detection logic

### Integration Tests
- End-to-end with Azure OpenAI (API key)
- End-to-end with Azure Entra ID
- Custom APIM endpoint testing
- Token expiry and refresh scenarios

### Manual Testing
- Device code flow interactive login
- Managed identity on Azure VM
- Corporate proxy with custom CA certs

---

## Risk Assessment

| Risk | Impact | Mitigation |
|------|--------|------------|
| Azure SDK compatibility | High | Pin azure_identity version, test against multiple Azure regions |
| Upstream sync difficulty | Medium | Minimize diff, keep changes modular |
| Breaking config changes | Medium | Maintain backward compatibility, migrate existing configs |
| Corporate proxy issues | Medium | Support custom CA certificates, proxy configuration |

---

## Next Steps

1. **Immediate**: Set up fork with proper git history
2. **Phase 1**: Implement auth header flexibility (start here)
3. **Parallel**: Document Azure setup guide for end users
4. **Later**: Consider contributing improvements upstream

---

## References

- [GitHub Issue #1056](https://github.com/openai/codex/issues/1056) - Entra authentication
- [GitHub Issue #3048](https://github.com/openai/codex/issues/3048) - Custom endpoints
- [GitHub Issue #4278](https://github.com/openai/codex/issues/4278) - OAuth support
- [Azure Identity SDK](https://docs.rs/azure_identity/latest/azure_identity/)
- [Azure OpenAI v1 API](https://learn.microsoft.com/en-us/azure/ai-foundry/openai/reference)
