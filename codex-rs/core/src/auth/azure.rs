//! Azure Entra ID authentication provider.
//!
//! This module implements Azure authentication for Azure OpenAI Service,
//! supporting multiple authentication methods:
//! - DefaultAzureCredential (automatic credential chain)
//! - Device Code flow (interactive login)
//! - Managed Identity
//! - Service Principal (client secret or certificate)
//! - Azure CLI credentials

use crate::auth::azure_config::{AzureAuthConfig, AzureAuthMode};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use thiserror::Error;
use tracing::{debug, info};

/// Errors that can occur during Azure authentication.
#[derive(Debug, Error)]
pub enum AzureAuthError {
    #[error("Azure authentication not configured")]
    NotConfigured,

    #[error("Token acquisition failed: {0}")]
    TokenAcquisitionFailed(String),

    #[error("Device code authentication timed out")]
    DeviceCodeTimeout,

    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),

    #[error("Token refresh failed: {0}")]
    RefreshFailed(String),

    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("JSON parsing failed: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Environment variable not set: {0}")]
    EnvVarNotSet(String),

    #[error("Certificate error: {0}")]
    CertificateError(String),

    #[error("Azure CLI authentication failed: {0}")]
    AzureCliError(String),
}

/// A cached token with its expiration time.
#[derive(Debug, Clone)]
struct CachedToken {
    /// The access token.
    token: String,
    /// When the token expires.
    expires_at: Instant,
    /// Scope the token was acquired for.
    #[allow(dead_code)]
    scope: String,
}

impl CachedToken {
    /// Check if the token is expired or will expire within the buffer time.
    fn is_expired(&self) -> bool {
        // Consider token expired 5 minutes before actual expiration
        let buffer = Duration::from_secs(300);
        Instant::now() + buffer >= self.expires_at
    }
}

/// Azure authentication provider.
///
/// Handles token acquisition, caching, and refresh for Azure Entra ID.
#[derive(Debug)]
pub struct AzureAuth {
    /// The authentication configuration.
    config: AzureAuthConfig,
    /// Cached access token.
    cached_token: Arc<RwLock<Option<CachedToken>>>,
    /// HTTP client for token requests.
    client: reqwest::Client,
}

impl Clone for AzureAuth {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            cached_token: Arc::clone(&self.cached_token),
            client: self.client.clone(),
        }
    }
}

/// Response from Azure AD token endpoint.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
    #[serde(default)]
    #[allow(dead_code)]
    token_type: String,
}

/// Device code response from Azure AD.
#[derive(Debug, Deserialize, Serialize)]
pub struct DeviceCodeResponse {
    /// The device code to poll with.
    pub device_code: String,
    /// The code the user needs to enter.
    pub user_code: String,
    /// The URL the user should visit.
    pub verification_uri: String,
    /// How often to poll (in seconds).
    pub interval: u64,
    /// When the code expires (in seconds).
    pub expires_in: u64,
    /// User-friendly message.
    pub message: String,
}

impl AzureAuth {
    /// Creates a new Azure authentication provider.
    pub fn new(config: AzureAuthConfig) -> Self {
        Self {
            config,
            cached_token: Arc::new(RwLock::new(None)),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to create HTTP client"),
        }
    }

    /// Creates a new Azure authentication provider with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(AzureAuthConfig::default())
    }

    /// Returns the authentication configuration.
    pub fn config(&self) -> &AzureAuthConfig {
        &self.config
    }

    /// Gets an access token, using cached token if valid.
    pub async fn get_token(&self) -> Result<String, AzureAuthError> {
        // Check if we have a valid cached token
        {
            let cached = self.cached_token.read().unwrap();
            if let Some(ref token) = *cached {
                if !token.is_expired() {
                    debug!("Using cached Azure token");
                    return Ok(token.token.clone());
                }
            }
        }

        // Acquire a new token
        debug!("Acquiring new Azure token");
        let token = self.acquire_token().await?;

        Ok(token)
    }

    /// Forces a token refresh, ignoring any cached token.
    pub async fn refresh_token(&self) -> Result<String, AzureAuthError> {
        debug!("Forcing Azure token refresh");
        self.clear_cached_token().await;
        self.acquire_token().await
    }

    /// Clears the cached token without acquiring a new one.
    /// Call this after receiving a 401 to force re-authentication on next request.
    pub async fn clear_cached_token(&self) {
        debug!("Clearing cached Azure token");
        let mut cached = self.cached_token.write().unwrap();
        *cached = None;
    }

    /// Acquires a new token based on the configured authentication mode.
    async fn acquire_token(&self) -> Result<String, AzureAuthError> {
        let token = match &self.config.mode {
            AzureAuthMode::Default => self.acquire_token_default().await?,
            AzureAuthMode::DeviceCode { .. } => {
                return Err(AzureAuthError::InvalidConfiguration(
                    "Device code flow requires interactive authentication. Use start_device_code_flow() instead.".into(),
                ));
            }
            AzureAuthMode::ManagedIdentity { client_id } => {
                self.acquire_token_managed_identity(client_id.as_deref())
                    .await?
            }
            AzureAuthMode::ClientSecret {
                tenant_id,
                client_id,
                client_secret,
            } => {
                self.acquire_token_client_secret(tenant_id, client_id, client_secret.as_deref())
                    .await?
            }
            AzureAuthMode::ClientCertificate { .. } => {
                // Certificate auth requires more complex implementation
                return Err(AzureAuthError::InvalidConfiguration(
                    "Certificate authentication not yet implemented".into(),
                ));
            }
            AzureAuthMode::AzureCli => self.acquire_token_azure_cli().await?,
            AzureAuthMode::EnvironmentCredential => {
                self.acquire_token_environment_credential().await?
            }
        };

        Ok(token)
    }

    /// Acquire token using DefaultAzureCredential logic.
    /// Tries multiple methods in order: Environment, Managed Identity, Azure CLI.
    async fn acquire_token_default(&self) -> Result<String, AzureAuthError> {
        // Try environment credentials first
        if let Ok(token) = self.acquire_token_environment_credential().await {
            info!("Acquired Azure token via environment credentials");
            return Ok(token);
        }

        // Try managed identity
        if let Ok(token) = self.acquire_token_managed_identity(None).await {
            info!("Acquired Azure token via managed identity");
            return Ok(token);
        }

        // Try Azure CLI
        if let Ok(token) = self.acquire_token_azure_cli().await {
            info!("Acquired Azure token via Azure CLI");
            return Ok(token);
        }

        Err(AzureAuthError::TokenAcquisitionFailed(
            "All credential sources failed. Ensure you are logged in via Azure CLI, \
             have environment variables set, or are running in Azure with managed identity."
                .into(),
        ))
    }

    /// Acquire token using environment variables.
    async fn acquire_token_environment_credential(&self) -> Result<String, AzureAuthError> {
        let client_id = std::env::var("AZURE_CLIENT_ID")
            .map_err(|_| AzureAuthError::EnvVarNotSet("AZURE_CLIENT_ID".into()))?;
        let client_secret = std::env::var("AZURE_CLIENT_SECRET")
            .map_err(|_| AzureAuthError::EnvVarNotSet("AZURE_CLIENT_SECRET".into()))?;
        let tenant_id = std::env::var("AZURE_TENANT_ID")
            .map_err(|_| AzureAuthError::EnvVarNotSet("AZURE_TENANT_ID".into()))?;

        self.acquire_token_client_secret(&tenant_id, &client_id, Some(&client_secret))
            .await
    }

    /// Acquire token using client secret credentials.
    async fn acquire_token_client_secret(
        &self,
        tenant_id: &str,
        client_id: &str,
        client_secret: Option<&str>,
    ) -> Result<String, AzureAuthError> {
        let secret = match client_secret {
            Some(s) => s.to_string(),
            None => std::env::var("AZURE_CLIENT_SECRET").map_err(|_| {
                AzureAuthError::InvalidConfiguration(
                    "Client secret not provided and AZURE_CLIENT_SECRET not set".into(),
                )
            })?,
        };

        let authority = self.config.effective_authority();
        let token_url = format!("{}/{}/oauth2/v2.0/token", authority, tenant_id);

        let params = [
            ("client_id", client_id.to_string()),
            ("client_secret", secret),
            ("scope", self.config.scope.clone()),
            ("grant_type", "client_credentials".to_string()),
        ];

        let response = self
            .client
            .post(&token_url)
            .form(&params)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AzureAuthError::TokenAcquisitionFailed(format!(
                "HTTP {}: {}",
                status, body
            )));
        }

        let token_response: TokenResponse = response.json().await?;
        self.cache_token(&token_response);

        Ok(token_response.access_token)
    }

    /// Acquire token using managed identity (IMDS endpoint).
    async fn acquire_token_managed_identity(
        &self,
        client_id: Option<&str>,
    ) -> Result<String, AzureAuthError> {
        // Azure Instance Metadata Service endpoint
        let imds_url = "http://169.254.169.254/metadata/identity/oauth2/token";

        // Extract the resource from the scope (remove /.default suffix)
        let resource = self.config.scope.replace("/.default", "");
        let encoded_resource = url_encode(&resource);

        let mut url = format!(
            "{}?api-version=2019-08-01&resource={}",
            imds_url, encoded_resource
        );

        // Add client_id for user-assigned managed identity
        if let Some(id) = client_id {
            url.push_str(&format!("&client_id={}", url_encode(id)));
        }

        let response = self
            .client
            .get(&url)
            .header("Metadata", "true")
            .timeout(Duration::from_secs(5))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AzureAuthError::TokenAcquisitionFailed(format!(
                "Managed identity request failed - HTTP {}: {}",
                status, body
            )));
        }

        let token_response: TokenResponse = response.json().await?;
        self.cache_token(&token_response);

        Ok(token_response.access_token)
    }

    /// Acquire token using Azure CLI.
    async fn acquire_token_azure_cli(&self) -> Result<String, AzureAuthError> {
        let scope = &self.config.scope;

        // Use az account get-access-token
        // On Windows, we need to use cmd.exe to run az.cmd since it's a batch script
        #[cfg(windows)]
        let output = tokio::process::Command::new("cmd")
            .args([
                "/C",
                "az",
                "account",
                "get-access-token",
                "--scope",
                scope,
                "--output",
                "json",
            ])
            .output()
            .await
            .map_err(|e| AzureAuthError::AzureCliError(format!("Failed to run az CLI: {}", e)))?;

        #[cfg(not(windows))]
        let output = tokio::process::Command::new("az")
            .args([
                "account",
                "get-access-token",
                "--scope",
                scope,
                "--output",
                "json",
            ])
            .output()
            .await
            .map_err(|e| AzureAuthError::AzureCliError(format!("Failed to run az CLI: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AzureAuthError::AzureCliError(format!(
                "Azure CLI returned error: {}",
                stderr
            )));
        }

        #[derive(Deserialize)]
        struct AzCliToken {
            #[serde(rename = "accessToken")]
            access_token: String,
            #[serde(rename = "expiresOn")]
            expires_on: String,
        }

        let cli_token: AzCliToken = serde_json::from_slice(&output.stdout)?;

        // Parse expiration time and cache
        // Azure CLI returns expiration in format "2024-01-15 10:30:00.000000"
        let expires_at = if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(
            &cli_token.expires_on,
            "%Y-%m-%d %H:%M:%S%.f",
        ) {
            let duration_until_expiry = (dt - chrono::Utc::now().naive_utc())
                .to_std()
                .unwrap_or(Duration::from_secs(3600));
            Instant::now() + duration_until_expiry
        } else {
            // Default to 1 hour if we can't parse
            Instant::now() + Duration::from_secs(3600)
        };

        let cached = CachedToken {
            token: cli_token.access_token.clone(),
            expires_at,
            scope: self.config.scope.clone(),
        };

        {
            let mut cache = self.cached_token.write().unwrap();
            *cache = Some(cached);
        }

        Ok(cli_token.access_token)
    }

    /// Start the device code authentication flow.
    ///
    /// Returns a `DeviceCodeResponse` containing the user code and verification URL.
    /// The caller should display these to the user and then call `poll_device_code`
    /// to complete the authentication.
    pub async fn start_device_code_flow(
        &self,
        tenant_id: &str,
        client_id: &str,
    ) -> Result<DeviceCodeResponse, AzureAuthError> {
        let authority = self.config.effective_authority();
        let device_code_url = format!("{}/{}/oauth2/v2.0/devicecode", authority, tenant_id);

        let params = [("client_id", client_id), ("scope", &self.config.scope)];

        let response = self.client.post(&device_code_url).form(&params).send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AzureAuthError::TokenAcquisitionFailed(format!(
                "Device code request failed - HTTP {}: {}",
                status, body
            )));
        }

        let device_code: DeviceCodeResponse = response.json().await?;
        Ok(device_code)
    }

    /// Poll for device code authentication completion.
    ///
    /// This should be called after `start_device_code_flow`. It will poll the
    /// token endpoint until the user completes authentication or the code expires.
    pub async fn poll_device_code(
        &self,
        tenant_id: &str,
        client_id: &str,
        device_code: &DeviceCodeResponse,
    ) -> Result<String, AzureAuthError> {
        let authority = self.config.effective_authority();
        let token_url = format!("{}/{}/oauth2/v2.0/token", authority, tenant_id);

        let poll_interval = Duration::from_secs(device_code.interval.max(5));
        let deadline = Instant::now() + Duration::from_secs(device_code.expires_in);

        while Instant::now() < deadline {
            let params = [
                ("client_id", client_id),
                ("device_code", &device_code.device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ];

            let response = self.client.post(&token_url).form(&params).send().await?;

            if response.status().is_success() {
                let token_response: TokenResponse = response.json().await?;
                self.cache_token(&token_response);
                info!("Device code authentication completed successfully");
                return Ok(token_response.access_token);
            }

            // Check for specific error responses
            #[derive(Deserialize)]
            struct ErrorResponse {
                error: String,
                error_description: Option<String>,
            }

            let body = response.text().await.unwrap_or_default();
            if let Ok(error) = serde_json::from_str::<ErrorResponse>(&body) {
                match error.error.as_str() {
                    "authorization_pending" => {
                        // User hasn't completed auth yet, keep polling
                        debug!("Device code: authorization pending, polling again...");
                    }
                    "slow_down" => {
                        // We're polling too fast, increase interval
                        debug!("Device code: slow down requested");
                        tokio::time::sleep(poll_interval * 2).await;
                        continue;
                    }
                    "expired_token" => {
                        return Err(AzureAuthError::DeviceCodeTimeout);
                    }
                    "access_denied" => {
                        return Err(AzureAuthError::TokenAcquisitionFailed(
                            error
                                .error_description
                                .unwrap_or_else(|| "Access denied".into()),
                        ));
                    }
                    _ => {
                        return Err(AzureAuthError::TokenAcquisitionFailed(format!(
                            "{}: {}",
                            error.error,
                            error.error_description.unwrap_or_default()
                        )));
                    }
                }
            }

            tokio::time::sleep(poll_interval).await;
        }

        Err(AzureAuthError::DeviceCodeTimeout)
    }

    /// Cache a token from a token response.
    fn cache_token(&self, response: &TokenResponse) {
        let expires_at = Instant::now() + Duration::from_secs(response.expires_in.max(300));

        let cached = CachedToken {
            token: response.access_token.clone(),
            expires_at,
            scope: self.config.scope.clone(),
        };

        let mut cache = self.cached_token.write().unwrap();
        *cache = Some(cached);
    }

    /// Check if the provider is configured for Azure authentication.
    pub fn is_configured(&self) -> bool {
        true // AzureAuthConfig always has some mode
    }

    /// Returns true if this is an Azure authentication provider.
    pub fn is_azure(&self) -> bool {
        true
    }

    /// Clear any cached credentials.
    pub fn clear_cache(&self) {
        let mut cache = self.cached_token.write().unwrap();
        *cache = None;
    }
}

/// Simple URL encoding for query parameters.
/// Only encodes the most necessary characters for OAuth flows.
fn url_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    for c in s.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => {
                result.push(c);
            }
            _ => {
                for byte in c.to_string().as_bytes() {
                    result.push('%');
                    result.push_str(&format!("{:02X}", byte));
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_branding::AZURE_DEFAULT_SCOPE;

    #[test]
    fn test_cached_token_expiration() {
        let token = CachedToken {
            token: "test".to_string(),
            expires_at: Instant::now() + Duration::from_secs(600),
            scope: AZURE_DEFAULT_SCOPE.to_string(),
        };
        assert!(!token.is_expired());

        let expired_token = CachedToken {
            token: "test".to_string(),
            expires_at: Instant::now() + Duration::from_secs(60), // Within 5 min buffer
            scope: AZURE_DEFAULT_SCOPE.to_string(),
        };
        assert!(expired_token.is_expired());
    }

    #[test]
    fn test_url_encode() {
        assert_eq!(url_encode("hello"), "hello");
        assert_eq!(
            url_encode("https://cognitiveservices.azure.com"),
            "https%3A%2F%2Fcognitiveservices.azure.com"
        );
    }

    #[test]
    fn test_azure_auth_creation() {
        let auth = AzureAuth::with_defaults();
        assert!(auth.is_azure());
        assert!(auth.is_configured());
    }

    #[test]
    fn test_azure_auth_with_config() {
        let config = AzureAuthConfig::new_device_code("tenant-123", "client-456");
        let auth = AzureAuth::new(config);

        match &auth.config().mode {
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
}
