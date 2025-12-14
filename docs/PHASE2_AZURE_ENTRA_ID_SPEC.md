# Phase 2: Azure Entra ID Authentication - Detailed Specification

## Overview

This document provides a detailed implementation specification for adding Azure Entra ID (formerly Azure Active Directory) authentication support to Azure Codex. This enables enterprise customers to use Azure's identity platform for secure, managed authentication.

## Goals

1. Support multiple Azure authentication methods:
   - DefaultAzureCredential (automatic credential chain)
   - Device Code flow (interactive login)
   - Managed Identity (for Azure VMs, App Service, AKS, etc.)
   - Client Secret (service principal)
   - Client Certificate (service principal with cert)

2. Automatic token refresh before expiration
3. Seamless integration with existing auth flow
4. Configuration via TOML and environment variables

## Architecture

### New Module Structure

```
codex-rs/core/src/
├── auth/
│   ├── mod.rs           # Re-exports, AuthManager
│   ├── storage.rs       # Existing storage backend
│   ├── azure.rs         # NEW: Azure Entra ID implementation
│   └── azure_config.rs  # NEW: Azure auth configuration
```

### Dependencies

Add to `codex-rs/Cargo.toml`:

```toml
[workspace.dependencies]
# Azure Identity SDK
azure_identity = "0.20"
azure_core = "0.20"

# For token caching
tokio = { version = "1", features = ["sync", "time"] }
```

Add to `codex-rs/core/Cargo.toml`:

```toml
[dependencies]
azure_identity = { workspace = true, optional = true }
azure_core = { workspace = true, optional = true }

[features]
default = ["azure-auth"]
azure-auth = ["dep:azure_identity", "dep:azure_core"]
```

## Detailed Implementation

### 1. Azure Authentication Configuration (`auth/azure_config.rs`)

```rust
//! Azure Entra ID authentication configuration.

use serde::{Deserialize, Serialize};

/// Azure authentication modes supported by Azure Codex.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum AzureAuthMode {
    /// Use DefaultAzureCredential which tries multiple auth methods in order:
    /// 1. Environment variables (AZURE_CLIENT_ID, AZURE_CLIENT_SECRET, AZURE_TENANT_ID)
    /// 2. Managed Identity
    /// 3. Azure CLI
    /// 4. Azure PowerShell
    /// 5. Visual Studio Code
    Default,

    /// Device code flow for interactive login.
    /// User visits a URL and enters a code to authenticate.
    DeviceCode {
        tenant_id: String,
        client_id: String,
    },

    /// Managed Identity authentication for Azure-hosted resources.
    /// Works on Azure VMs, App Service, AKS, Azure Functions, etc.
    ManagedIdentity {
        /// Optional client ID for user-assigned managed identity.
        /// If not specified, uses system-assigned managed identity.
        #[serde(skip_serializing_if = "Option::is_none")]
        client_id: Option<String>,
    },

    /// Service Principal with client secret.
    ClientSecret {
        tenant_id: String,
        client_id: String,
        /// The client secret. Can also be set via AZURE_CLIENT_SECRET env var.
        #[serde(skip_serializing_if = "Option::is_none")]
        client_secret: Option<String>,
    },

    /// Service Principal with certificate.
    ClientCertificate {
        tenant_id: String,
        client_id: String,
        /// Path to the certificate file (PEM or PFX format).
        certificate_path: String,
        /// Password for the certificate if encrypted.
        #[serde(skip_serializing_if = "Option::is_none")]
        certificate_password: Option<String>,
    },

    /// Azure CLI authentication - uses `az login` credentials.
    AzureCli,

    /// Environment variables only (AZURE_CLIENT_ID, etc.)
    EnvironmentCredential,
}

impl Default for AzureAuthMode {
    fn default() -> Self {
        Self::Default
    }
}

/// Full Azure authentication configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AzureAuthConfig {
    /// The authentication mode to use.
    #[serde(flatten)]
    pub mode: AzureAuthMode,

    /// The scope to request. Defaults to Azure Cognitive Services scope.
    #[serde(default = "default_azure_scope")]
    pub scope: String,

    /// Optional: Override the Azure AD authority URL.
    /// Useful for sovereign clouds (US Gov, China, Germany).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority: Option<String>,
}

fn default_azure_scope() -> String {
    "https://cognitiveservices.azure.com/.default".to_string()
}

impl Default for AzureAuthConfig {
    fn default() -> Self {
        Self {
            mode: AzureAuthMode::Default,
            scope: default_azure_scope(),
            authority: None,
        }
    }
}

/// Sovereign cloud authorities.
pub mod authorities {
    pub const AZURE_PUBLIC: &str = "https://login.microsoftonline.com";
    pub const AZURE_US_GOVERNMENT: &str = "https://login.microsoftonline.us";
    pub const AZURE_CHINA: &str = "https://login.chinacloudapi.cn";
    pub const AZURE_GERMANY: &str = "https://login.microsoftonline.de";
}

/// Cognitive Services scopes for different clouds.
pub mod scopes {
    pub const AZURE_PUBLIC: &str = "https://cognitiveservices.azure.com/.default";
    pub const AZURE_US_GOVERNMENT: &str = "https://cognitiveservices.azure.us/.default";
    pub const AZURE_CHINA: &str = "https://cognitiveservices.azure.cn/.default";
}
```

### 2. Azure Authentication Provider (`auth/azure.rs`)

```rust
//! Azure Entra ID authentication implementation.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::auth::azure_config::{AzureAuthConfig, AzureAuthMode};

#[cfg(feature = "azure-auth")]
use azure_core::auth::TokenCredential;
#[cfg(feature = "azure-auth")]
use azure_identity::{
    AzureCliCredential,
    ClientCertificateCredential,
    ClientSecretCredential,
    DefaultAzureCredential,
    DeviceCodeCredential,
    EnvironmentCredential,
    ManagedIdentityCredential,
    TokenCredentialOptions,
};

/// Cached token with expiration tracking.
#[derive(Debug, Clone)]
struct CachedToken {
    token: String,
    /// When this token was acquired
    acquired_at: Instant,
    /// Token lifetime (typically 1 hour for Azure AD)
    expires_in: Duration,
}

impl CachedToken {
    /// Returns true if the token will expire within the given buffer time.
    fn is_expiring_within(&self, buffer: Duration) -> bool {
        let elapsed = self.acquired_at.elapsed();
        elapsed + buffer >= self.expires_in
    }

    /// Returns true if the token is still valid with a 5-minute buffer.
    fn is_valid(&self) -> bool {
        !self.is_expiring_within(Duration::from_secs(300))
    }
}

/// Error types for Azure authentication.
#[derive(Debug, thiserror::Error)]
pub enum AzureAuthError {
    #[error("Azure credential error: {0}")]
    Credential(String),

    #[error("Token acquisition failed: {0}")]
    TokenAcquisition(String),

    #[error("Configuration error: {0}")]
    Configuration(String),

    #[error("Azure auth feature not enabled")]
    FeatureNotEnabled,
}

#[cfg(feature = "azure-auth")]
impl From<azure_core::error::Error> for AzureAuthError {
    fn from(err: azure_core::error::Error) -> Self {
        AzureAuthError::Credential(err.to_string())
    }
}

/// Azure Entra ID authentication provider.
///
/// Handles token acquisition, caching, and automatic refresh.
pub struct AzureAuth {
    config: AzureAuthConfig,
    #[cfg(feature = "azure-auth")]
    credential: Arc<dyn TokenCredential>,
    cached_token: RwLock<Option<CachedToken>>,
    /// Callback for device code flow - displays code to user
    device_code_callback: Option<Box<dyn Fn(&str, &str) + Send + Sync>>,
}

impl AzureAuth {
    /// Creates a new Azure authentication provider.
    #[cfg(feature = "azure-auth")]
    pub fn new(config: AzureAuthConfig) -> Result<Self, AzureAuthError> {
        let credential = Self::create_credential(&config)?;
        Ok(Self {
            config,
            credential,
            cached_token: RwLock::new(None),
            device_code_callback: None,
        })
    }

    /// Creates a new Azure auth provider with a custom device code callback.
    #[cfg(feature = "azure-auth")]
    pub fn with_device_code_callback<F>(
        config: AzureAuthConfig,
        callback: F,
    ) -> Result<Self, AzureAuthError>
    where
        F: Fn(&str, &str) + Send + Sync + 'static,
    {
        let mut auth = Self::new(config)?;
        auth.device_code_callback = Some(Box::new(callback));
        Ok(auth)
    }

    #[cfg(feature = "azure-auth")]
    fn create_credential(
        config: &AzureAuthConfig,
    ) -> Result<Arc<dyn TokenCredential>, AzureAuthError> {
        let credential: Arc<dyn TokenCredential> = match &config.mode {
            AzureAuthMode::Default => {
                Arc::new(DefaultAzureCredential::default())
            }

            AzureAuthMode::DeviceCode { tenant_id, client_id } => {
                let cred = DeviceCodeCredential::new(
                    tenant_id.clone(),
                    client_id.clone(),
                    |code| {
                        // Default device code handler - print to stderr
                        eprintln!("\n╭────────────────────────────────────────────────────────────╮");
                        eprintln!("│  Azure Login Required                                       │");
                        eprintln!("├────────────────────────────────────────────────────────────┤");
                        eprintln!("│  To sign in, open a browser to:                            │");
                        eprintln!("│  https://microsoft.com/devicelogin                         │");
                        eprintln!("│                                                            │");
                        eprintln!("│  Enter code: {:<45} │", code.user_code);
                        eprintln!("╰────────────────────────────────────────────────────────────╯\n");
                    },
                )?;
                Arc::new(cred)
            }

            AzureAuthMode::ManagedIdentity { client_id } => {
                let mut opts = TokenCredentialOptions::default();
                if let Some(id) = client_id {
                    // User-assigned managed identity
                    opts = opts.with_client_id(id.clone());
                }
                Arc::new(ManagedIdentityCredential::new(opts)?)
            }

            AzureAuthMode::ClientSecret {
                tenant_id,
                client_id,
                client_secret,
            } => {
                let secret = client_secret
                    .clone()
                    .or_else(|| std::env::var("AZURE_CLIENT_SECRET").ok())
                    .ok_or_else(|| {
                        AzureAuthError::Configuration(
                            "client_secret not provided and AZURE_CLIENT_SECRET not set".into(),
                        )
                    })?;

                Arc::new(ClientSecretCredential::new(
                    tenant_id.clone(),
                    client_id.clone(),
                    secret,
                    TokenCredentialOptions::default(),
                )?)
            }

            AzureAuthMode::ClientCertificate {
                tenant_id,
                client_id,
                certificate_path,
                certificate_password,
            } => {
                let cert_data = std::fs::read(certificate_path).map_err(|e| {
                    AzureAuthError::Configuration(format!(
                        "Failed to read certificate at {}: {}",
                        certificate_path, e
                    ))
                })?;

                Arc::new(ClientCertificateCredential::new(
                    tenant_id.clone(),
                    client_id.clone(),
                    cert_data,
                    certificate_password.clone(),
                    TokenCredentialOptions::default(),
                )?)
            }

            AzureAuthMode::AzureCli => {
                Arc::new(AzureCliCredential::new()?)
            }

            AzureAuthMode::EnvironmentCredential => {
                Arc::new(EnvironmentCredential::new(TokenCredentialOptions::default())?)
            }
        };

        Ok(credential)
    }

    /// Gets a valid token, using cache if available or fetching a new one.
    #[cfg(feature = "azure-auth")]
    pub async fn get_token(&self) -> Result<String, AzureAuthError> {
        // Check cache first
        {
            let cache = self.cached_token.read().await;
            if let Some(cached) = &*cache {
                if cached.is_valid() {
                    debug!("Using cached Azure token");
                    return Ok(cached.token.clone());
                }
                debug!("Cached token expired or expiring soon, refreshing");
            }
        }

        // Fetch new token
        self.refresh_token().await
    }

    /// Forces a token refresh, bypassing the cache.
    #[cfg(feature = "azure-auth")]
    pub async fn refresh_token(&self) -> Result<String, AzureAuthError> {
        info!("Acquiring new Azure access token");

        let token_response = self
            .credential
            .get_token(&[&self.config.scope])
            .await
            .map_err(|e| AzureAuthError::TokenAcquisition(e.to_string()))?;

        let token = token_response.token.secret().to_string();

        // Azure tokens typically expire in 1 hour
        let expires_in = token_response
            .expires_on
            .map(|exp| {
                let now = time::OffsetDateTime::now_utc();
                let duration = exp - now;
                Duration::from_secs(duration.whole_seconds().max(0) as u64)
            })
            .unwrap_or(Duration::from_secs(3600));

        // Update cache
        {
            let mut cache = self.cached_token.write().await;
            *cache = Some(CachedToken {
                token: token.clone(),
                acquired_at: Instant::now(),
                expires_in,
            });
        }

        debug!("Azure token acquired, expires in {:?}", expires_in);
        Ok(token)
    }

    /// Returns the configured scope.
    pub fn scope(&self) -> &str {
        &self.config.scope
    }

    /// Returns the authentication mode.
    pub fn mode(&self) -> &AzureAuthMode {
        &self.config.mode
    }

    // Non-azure-auth feature implementations
    #[cfg(not(feature = "azure-auth"))]
    pub fn new(_config: AzureAuthConfig) -> Result<Self, AzureAuthError> {
        Err(AzureAuthError::FeatureNotEnabled)
    }

    #[cfg(not(feature = "azure-auth"))]
    pub async fn get_token(&self) -> Result<String, AzureAuthError> {
        Err(AzureAuthError::FeatureNotEnabled)
    }

    #[cfg(not(feature = "azure-auth"))]
    pub async fn refresh_token(&self) -> Result<String, AzureAuthError> {
        Err(AzureAuthError::FeatureNotEnabled)
    }
}

impl std::fmt::Debug for AzureAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AzureAuth")
            .field("config", &self.config)
            .field("has_callback", &self.device_code_callback.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cached_token_expiry() {
        let token = CachedToken {
            token: "test".to_string(),
            acquired_at: Instant::now(),
            expires_in: Duration::from_secs(3600),
        };
        assert!(token.is_valid());
        assert!(!token.is_expiring_within(Duration::from_secs(3500)));
        assert!(token.is_expiring_within(Duration::from_secs(3601)));
    }

    #[test]
    fn test_azure_auth_config_default() {
        let config = AzureAuthConfig::default();
        assert_eq!(config.mode, AzureAuthMode::Default);
        assert_eq!(config.scope, "https://cognitiveservices.azure.com/.default");
    }

    #[test]
    fn test_azure_auth_config_serialization() {
        let config = AzureAuthConfig {
            mode: AzureAuthMode::DeviceCode {
                tenant_id: "tenant-123".to_string(),
                client_id: "client-456".to_string(),
            },
            scope: "https://cognitiveservices.azure.com/.default".to_string(),
            authority: None,
        };

        let toml = toml::to_string(&config).unwrap();
        assert!(toml.contains("mode = \"device_code\""));
        assert!(toml.contains("tenant_id = \"tenant-123\""));
    }
}
```

### 3. Configuration Examples

#### TOML Configuration

```toml
# ~/.codex/config.toml

# Example 1: Azure with API Key (simplest)
model = "gpt-4o"
model_provider = "azure"

[model_providers.azure]
name = "Azure OpenAI"
base_url = "https://my-resource.openai.azure.com/openai/v1"
env_key = "AZURE_OPENAI_API_KEY"
wire_api = "responses"
auth_header_type = "api_key"
is_azure = true

# Example 2: Azure with DefaultAzureCredential
[model_providers.azure-entra]
name = "Azure OpenAI (Entra)"
base_url = "https://my-resource.openai.azure.com/openai/v1"
wire_api = "responses"
is_azure = true
auth_header_type = "bearer"

[model_providers.azure-entra.azure_auth]
mode = "default"
scope = "https://cognitiveservices.azure.com/.default"

# Example 3: Device Code Flow
[model_providers.azure-device]
name = "Azure OpenAI (Device Code)"
base_url = "https://my-resource.openai.azure.com/openai/v1"
wire_api = "responses"
is_azure = true
auth_header_type = "bearer"

[model_providers.azure-device.azure_auth]
mode = "device_code"
tenant_id = "your-tenant-id"
client_id = "your-client-id"
scope = "https://cognitiveservices.azure.com/.default"

# Example 4: Managed Identity (for Azure VMs/App Service)
[model_providers.azure-mi]
name = "Azure OpenAI (Managed Identity)"
base_url = "https://my-resource.openai.azure.com/openai/v1"
wire_api = "responses"
is_azure = true
auth_header_type = "bearer"

[model_providers.azure-mi.azure_auth]
mode = "managed_identity"
# client_id = "user-assigned-mi-client-id"  # Optional

# Example 5: Service Principal (for automation)
[model_providers.azure-sp]
name = "Azure OpenAI (Service Principal)"
base_url = "https://my-resource.openai.azure.com/openai/v1"
wire_api = "responses"
is_azure = true
auth_header_type = "bearer"

[model_providers.azure-sp.azure_auth]
mode = "client_secret"
tenant_id = "your-tenant-id"
client_id = "your-client-id"
# client_secret via AZURE_CLIENT_SECRET env var

# Example 6: Azure US Government Cloud
[model_providers.azure-gov]
name = "Azure OpenAI (US Gov)"
base_url = "https://my-resource.openai.azure.us/openai/v1"
wire_api = "responses"
is_azure = true
auth_header_type = "api_key"
env_key = "AZURE_OPENAI_API_KEY"

[model_providers.azure-gov.azure_auth]
scope = "https://cognitiveservices.azure.us/.default"
authority = "https://login.microsoftonline.us"
```

### 4. Integration with AuthManager

Update `core/src/auth.rs` to support Azure authentication:

```rust
// In AuthMode enum
#[derive(Debug, Clone, PartialEq)]
pub enum AuthMode {
    ApiKey,
    ChatGPT,
    AzureEntra(AzureAuthConfig),
}

// In CodexAuth struct
pub struct CodexAuth {
    pub mode: AuthMode,
    pub(crate) api_key: Option<String>,
    pub(crate) auth_dot_json: Arc<Mutex<Option<AuthDotJson>>>,
    storage: Arc<dyn AuthStorageBackend>,
    pub(crate) client: CodexHttpClient,
    // NEW
    #[cfg(feature = "azure-auth")]
    pub(crate) azure_auth: Option<Arc<AzureAuth>>,
}

impl CodexAuth {
    pub async fn get_token(&self) -> Result<String, std::io::Error> {
        match &self.mode {
            AuthMode::ApiKey => Ok(self.api_key.clone().unwrap_or_default()),
            AuthMode::ChatGPT => {
                let token = self.get_token_data().await?.access_token;
                Ok(token)
            }
            #[cfg(feature = "azure-auth")]
            AuthMode::AzureEntra(_) => {
                let azure = self.azure_auth.as_ref()
                    .ok_or_else(|| std::io::Error::other("Azure auth not initialized"))?;
                azure.get_token().await
                    .map_err(|e| std::io::Error::other(e.to_string()))
            }
            #[cfg(not(feature = "azure-auth"))]
            AuthMode::AzureEntra(_) => {
                Err(std::io::Error::other("Azure auth feature not enabled"))
            }
        }
    }
}
```

### 5. 401 Token Refresh Integration

Update `core/src/client.rs`:

```rust
async fn handle_unauthorized(
    status: StatusCode,
    refreshed: &mut bool,
    auth_manager: &Option<Arc<AuthManager>>,
    auth: &Option<CodexAuth>,
) -> Result<()> {
    if *refreshed {
        return Err(map_unauthorized_status(status));
    }

    if let Some(manager) = auth_manager.as_ref()
        && let Some(auth) = auth.as_ref()
    {
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

## Testing Plan

### Unit Tests
- Token caching and expiry logic
- Configuration serialization/deserialization
- Auth mode selection based on config

### Integration Tests (require Azure resources)
- DefaultAzureCredential with Azure CLI
- Device code flow (manual test)
- Managed Identity (on Azure VM)
- Service Principal auth

### Mock Tests
- Token refresh on 401
- Token caching behavior
- Error handling for various failure modes

## Security Considerations

1. **Never log tokens** - Tokens should be treated as secrets
2. **Secure storage** - Client secrets should come from env vars, not config files
3. **Token caching** - Tokens are cached in memory only, never persisted
4. **Certificate handling** - Support for certificate-based auth (more secure than secrets)
5. **Scope validation** - Ensure requested scope matches Azure OpenAI

## Migration Path

1. Existing API key users: No changes required
2. Existing ChatGPT users: No changes required
3. New Azure Entra users: Add `azure_auth` config section

## Future Enhancements

1. Token persistence across sessions (encrypted)
2. Multi-tenant support
3. Conditional access policy handling
4. Azure Key Vault integration for secrets
5. Token pre-refresh (refresh before expiry, not after)
