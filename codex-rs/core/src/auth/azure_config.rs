//! Azure Entra ID authentication configuration.
//!
//! This module defines the configuration structures for Azure authentication,
//! supporting multiple authentication methods including:
//! - DefaultAzureCredential (automatic credential chain)
//! - Device Code flow (interactive login)
//! - Managed Identity (for Azure-hosted resources)
//! - Service Principal with client secret or certificate

use codex_branding::{
    AZURE_CHINA_AUTHORITY, AZURE_CHINA_SCOPE, AZURE_DEFAULT_SCOPE, AZURE_PUBLIC_AUTHORITY,
    AZURE_US_GOV_AUTHORITY, AZURE_US_GOV_SCOPE,
};
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
        /// Azure AD tenant ID
        tenant_id: String,
        /// Application (client) ID registered in Azure AD
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
        /// Azure AD tenant ID
        tenant_id: String,
        /// Application (client) ID
        client_id: String,
        /// The client secret. Can also be set via AZURE_CLIENT_SECRET env var.
        #[serde(skip_serializing_if = "Option::is_none")]
        client_secret: Option<String>,
    },

    /// Service Principal with certificate.
    ClientCertificate {
        /// Azure AD tenant ID
        tenant_id: String,
        /// Application (client) ID
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

impl std::fmt::Display for AzureAuthMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AzureAuthMode::Default => write!(f, "default"),
            AzureAuthMode::DeviceCode { .. } => write!(f, "device_code"),
            AzureAuthMode::ManagedIdentity { .. } => write!(f, "managed_identity"),
            AzureAuthMode::ClientSecret { .. } => write!(f, "client_secret"),
            AzureAuthMode::ClientCertificate { .. } => write!(f, "client_certificate"),
            AzureAuthMode::AzureCli => write!(f, "azure_cli"),
            AzureAuthMode::EnvironmentCredential => write!(f, "environment"),
        }
    }
}

/// Azure cloud environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AzureCloud {
    /// Azure Public Cloud (default)
    #[default]
    Public,
    /// Azure US Government Cloud
    UsGovernment,
    /// Azure China Cloud (21Vianet)
    China,
    /// Custom cloud with explicit authority and scope
    Custom,
}

impl AzureCloud {
    /// Returns the default authority URL for this cloud.
    pub fn authority(&self) -> &'static str {
        match self {
            AzureCloud::Public | AzureCloud::Custom => AZURE_PUBLIC_AUTHORITY,
            AzureCloud::UsGovernment => AZURE_US_GOV_AUTHORITY,
            AzureCloud::China => AZURE_CHINA_AUTHORITY,
        }
    }

    /// Returns the default Cognitive Services scope for this cloud.
    pub fn cognitive_services_scope(&self) -> &'static str {
        match self {
            AzureCloud::Public | AzureCloud::Custom => AZURE_DEFAULT_SCOPE,
            AzureCloud::UsGovernment => AZURE_US_GOV_SCOPE,
            AzureCloud::China => AZURE_CHINA_SCOPE,
        }
    }
}

/// Full Azure authentication configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AzureAuthConfig {
    /// The authentication mode to use.
    #[serde(flatten)]
    pub mode: AzureAuthMode,

    /// The Azure cloud environment.
    #[serde(default)]
    pub cloud: AzureCloud,

    /// The scope to request. Defaults to Azure Cognitive Services scope.
    #[serde(default = "default_azure_scope")]
    pub scope: String,

    /// Optional: Override the Azure AD authority URL.
    /// Useful for custom or sovereign cloud configurations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority: Option<String>,
}

fn default_azure_scope() -> String {
    AZURE_DEFAULT_SCOPE.to_string()
}

impl Default for AzureAuthConfig {
    fn default() -> Self {
        Self {
            mode: AzureAuthMode::Default,
            cloud: AzureCloud::Public,
            scope: default_azure_scope(),
            authority: None,
        }
    }
}

impl AzureAuthConfig {
    /// Creates a new Azure auth config with default credential chain.
    pub fn new_default() -> Self {
        Self::default()
    }

    /// Creates a new Azure auth config for device code flow.
    pub fn new_device_code(tenant_id: impl Into<String>, client_id: impl Into<String>) -> Self {
        Self {
            mode: AzureAuthMode::DeviceCode {
                tenant_id: tenant_id.into(),
                client_id: client_id.into(),
            },
            ..Default::default()
        }
    }

    /// Creates a new Azure auth config for managed identity.
    pub fn new_managed_identity(client_id: Option<String>) -> Self {
        Self {
            mode: AzureAuthMode::ManagedIdentity { client_id },
            ..Default::default()
        }
    }

    /// Creates a new Azure auth config for service principal with secret.
    pub fn new_client_secret(
        tenant_id: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: Option<String>,
    ) -> Self {
        Self {
            mode: AzureAuthMode::ClientSecret {
                tenant_id: tenant_id.into(),
                client_id: client_id.into(),
                client_secret,
            },
            ..Default::default()
        }
    }

    /// Sets the Azure cloud environment.
    pub fn with_cloud(mut self, cloud: AzureCloud) -> Self {
        self.cloud = cloud;
        // Update scope to match cloud unless explicitly set
        if self.scope == default_azure_scope() {
            self.scope = cloud.cognitive_services_scope().to_string();
        }
        self
    }

    /// Sets a custom scope.
    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = scope.into();
        self
    }

    /// Sets a custom authority URL.
    pub fn with_authority(mut self, authority: impl Into<String>) -> Self {
        self.authority = Some(authority.into());
        self
    }

    /// Returns the effective authority URL.
    pub fn effective_authority(&self) -> &str {
        self.authority
            .as_deref()
            .unwrap_or_else(|| self.cloud.authority())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AzureAuthConfig::default();
        assert_eq!(config.mode, AzureAuthMode::Default);
        assert_eq!(config.cloud, AzureCloud::Public);
        assert_eq!(config.scope, AZURE_DEFAULT_SCOPE);
        assert_eq!(config.effective_authority(), AZURE_PUBLIC_AUTHORITY);
    }

    #[test]
    fn test_device_code_config() {
        let config = AzureAuthConfig::new_device_code("tenant-123", "client-456");
        match &config.mode {
            AzureAuthMode::DeviceCode {
                tenant_id,
                client_id,
            } => {
                assert_eq!(tenant_id, "tenant-123");
                assert_eq!(client_id, "client-456");
            }
            _ => panic!("Expected DeviceCode mode"),
        }
    }

    #[test]
    fn test_us_gov_cloud() {
        let config = AzureAuthConfig::default().with_cloud(AzureCloud::UsGovernment);
        assert_eq!(config.scope, AZURE_US_GOV_SCOPE);
        assert_eq!(config.effective_authority(), AZURE_US_GOV_AUTHORITY);
    }

    #[test]
    fn test_custom_authority() {
        let config =
            AzureAuthConfig::default().with_authority("https://login.custom.example.com");
        assert_eq!(
            config.effective_authority(),
            "https://login.custom.example.com"
        );
    }

    #[test]
    fn test_serialization_roundtrip() {
        let config = AzureAuthConfig::new_device_code("tenant", "client")
            .with_cloud(AzureCloud::UsGovernment);

        let toml_str = toml::to_string(&config).expect("serialize");
        let parsed: AzureAuthConfig = toml::from_str(&toml_str).expect("deserialize");

        assert_eq!(config.mode, parsed.mode);
        assert_eq!(config.cloud, parsed.cloud);
    }

    #[test]
    fn test_mode_display() {
        assert_eq!(AzureAuthMode::Default.to_string(), "default");
        assert_eq!(
            AzureAuthMode::DeviceCode {
                tenant_id: "t".into(),
                client_id: "c".into()
            }
            .to_string(),
            "device_code"
        );
        assert_eq!(
            AzureAuthMode::ManagedIdentity { client_id: None }.to_string(),
            "managed_identity"
        );
    }
}
