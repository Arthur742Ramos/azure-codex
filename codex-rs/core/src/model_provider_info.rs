//! Registry of model providers supported by Codex.
//!
//! Providers can be defined in two places:
//!   1. Built-in defaults compiled into the binary so Codex works out-of-the-box.
//!   2. User-defined entries inside `~/.codex/config.toml` under the `model_providers`
//!      key. These override or extend the defaults at runtime.

use codex_api::AuthHeaderType;
use codex_api::Provider as ApiProvider;
use codex_api::WireApi as ApiWireApi;
use codex_api::provider::RetryConfig as ApiRetryConfig;
use codex_app_server_protocol::AuthMode;
use http::HeaderMap;
use http::header::HeaderName;
use http::header::HeaderValue;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::env::VarError;
use std::time::Duration;

use crate::error::EnvVarError;
const DEFAULT_STREAM_IDLE_TIMEOUT_MS: u64 = 300_000;
const DEFAULT_STREAM_MAX_RETRIES: u64 = 10;
const DEFAULT_REQUEST_MAX_RETRIES: u64 = 4;
/// Hard cap for user-configured `stream_max_retries`.
const MAX_STREAM_MAX_RETRIES: u64 = 100;
/// Hard cap for user-configured `request_max_retries`.
const MAX_REQUEST_MAX_RETRIES: u64 = 100;
pub const CHAT_WIRE_API_DEPRECATION_SUMMARY: &str = r#"Support for the "chat" wire API is deprecated and will soon be removed. Update your model provider definition in config.toml to use wire_api = "responses"."#;

const OPENAI_PROVIDER_NAME: &str = "OpenAI";

/// Wire protocol that the provider speaks. Most third-party services only
/// implement the classic OpenAI Chat Completions JSON schema, whereas OpenAI
/// itself (and a handful of others) additionally expose the more modern
/// *Responses* API. The two protocols use different request/response shapes
/// and *cannot* be auto-detected at runtime, therefore each provider entry
/// must declare which one it expects.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WireApi {
    /// The Responses API exposed by OpenAI at `/v1/responses`.
    Responses,

    /// Regular Chat Completions compatible with `/v1/chat/completions`.
    #[default]
    Chat,
}

/// Serializable representation of a provider definition.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ModelProviderInfo {
    /// Friendly display name.
    pub name: String,
    /// Base URL for the provider's OpenAI-compatible API.
    pub base_url: Option<String>,
    /// Environment variable that stores the user's API key for this provider.
    pub env_key: Option<String>,

    /// Optional instructions to help the user get a valid value for the
    /// variable and set it.
    pub env_key_instructions: Option<String>,

    /// Value to use with `Authorization: Bearer <token>` header. Use of this
    /// config is discouraged in favor of `env_key` for security reasons, but
    /// this may be necessary when using this programmatically.
    pub experimental_bearer_token: Option<String>,

    /// Which wire protocol this provider expects.
    #[serde(default)]
    pub wire_api: WireApi,

    /// Optional query parameters to append to the base URL.
    pub query_params: Option<HashMap<String, String>>,

    /// Additional HTTP headers to include in requests to this provider where
    /// the (key, value) pairs are the header name and value.
    pub http_headers: Option<HashMap<String, String>>,

    /// Optional HTTP headers to include in requests to this provider where the
    /// (key, value) pairs are the header name and _environment variable_ whose
    /// value should be used. If the environment variable is not set, or the
    /// value is empty, the header will not be included in the request.
    pub env_http_headers: Option<HashMap<String, String>>,

    /// Maximum number of times to retry a failed HTTP request to this provider.
    pub request_max_retries: Option<u64>,

    /// Number of times to retry reconnecting a dropped streaming response before failing.
    pub stream_max_retries: Option<u64>,

    /// Idle timeout (in milliseconds) to wait for activity on a streaming response before treating
    /// the connection as lost.
    pub stream_idle_timeout_ms: Option<u64>,

    /// Does this provider require an OpenAI API Key or ChatGPT login token? If true,
    /// user is presented with login screen on first run, and login preference and token/key
    /// are stored in auth.json. If false (which is the default), login screen is skipped,
    /// and API key (if needed) comes from the "env_key" environment variable.
    #[serde(default)]
    pub requires_openai_auth: bool,

    // ===== Azure-specific configuration =====
    /// Authentication header type for this provider.
    /// - `bearer`: Uses `Authorization: Bearer <token>` (default, OpenAI style)
    /// - `api_key`: Uses `api-key: <key>` (Azure OpenAI style)
    /// - `custom`: Uses a custom header name specified in `auth_header_name`
    #[serde(default)]
    pub auth_header_type: AuthHeaderType,

    /// Explicitly mark this as an Azure endpoint. When true, bypasses hostname-based
    /// Azure detection and applies Azure-specific request handling.
    #[serde(default)]
    pub is_azure: bool,

    /// Skip automatic Azure endpoint detection based on hostname patterns.
    /// Set to true if you're using an Azure-compatible endpoint that shouldn't
    /// be treated as Azure (e.g., a local mock server).
    #[serde(default)]
    pub skip_azure_detection: bool,
}

impl ModelProviderInfo {
    fn build_header_map(&self) -> crate::error::Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        if let Some(extra) = &self.http_headers {
            for (k, v) in extra {
                if let (Ok(name), Ok(value)) = (HeaderName::try_from(k), HeaderValue::try_from(v)) {
                    headers.insert(name, value);
                }
            }
        }

        if let Some(env_headers) = &self.env_http_headers {
            for (header, env_var) in env_headers {
                if let Ok(val) = std::env::var(env_var)
                    && !val.trim().is_empty()
                    && let (Ok(name), Ok(value)) =
                        (HeaderName::try_from(header), HeaderValue::try_from(val))
                {
                    headers.insert(name, value);
                }
            }
        }

        Ok(headers)
    }

    pub(crate) fn to_api_provider(
        &self,
        auth_mode: Option<AuthMode>,
    ) -> crate::error::Result<ApiProvider> {
        self.to_api_provider_with_model(auth_mode, None)
    }

    /// Creates an API provider, optionally inserting the model name into the URL
    /// for Azure OpenAI endpoints that use the deployment-based URL format.
    pub(crate) fn to_api_provider_with_model(
        &self,
        auth_mode: Option<AuthMode>,
        model: Option<&str>,
    ) -> crate::error::Result<ApiProvider> {
        let default_base_url = if matches!(auth_mode, Some(AuthMode::ChatGPT)) {
            "https://chatgpt.com/backend-api/codex"
        } else {
            "https://api.openai.com/v1"
        };

        let mut base_url = self
            .base_url
            .clone()
            .unwrap_or_else(|| default_base_url.to_string());

        // For Azure OpenAI with auto-configured endpoint, the base_url ends with
        // "/openai/deployments" and we need to append the model/deployment name.
        // This allows the simple config format:
        //   azure_endpoint = "https://myresource.openai.azure.com"
        //   model = "gpt-4"
        if self.is_azure
            && base_url.ends_with("/openai/deployments")
            && let Some(model_name) = model
        {
            base_url = format!("{base_url}/{model_name}");
        }

        let headers = self.build_header_map()?;
        let retry = ApiRetryConfig {
            max_attempts: self.request_max_retries(),
            base_delay: Duration::from_millis(200),
            retry_429: false,
            retry_5xx: true,
            retry_transport: true,
        };

        Ok(ApiProvider {
            name: self.name.clone(),
            base_url,
            query_params: self.query_params.clone(),
            wire: match self.wire_api {
                WireApi::Responses => ApiWireApi::Responses,
                WireApi::Chat => ApiWireApi::Chat,
            },
            headers,
            retry,
            stream_idle_timeout: self.stream_idle_timeout(),
        })
    }

    /// If `env_key` is Some, returns the API key for this provider if present
    /// (and non-empty) in the environment. If `env_key` is required but
    /// cannot be found, returns an error.
    pub fn api_key(&self) -> crate::error::Result<Option<String>> {
        match &self.env_key {
            Some(env_key) => {
                let env_value = std::env::var(env_key);
                env_value
                    .and_then(|v| {
                        if v.trim().is_empty() {
                            Err(VarError::NotPresent)
                        } else {
                            Ok(Some(v))
                        }
                    })
                    .map_err(|_| {
                        crate::error::CodexErr::EnvVar(EnvVarError {
                            var: env_key.clone(),
                            instructions: self.env_key_instructions.clone(),
                        })
                    })
            }
            None => Ok(None),
        }
    }

    /// Effective maximum number of request retries for this provider.
    pub fn request_max_retries(&self) -> u64 {
        self.request_max_retries
            .unwrap_or(DEFAULT_REQUEST_MAX_RETRIES)
            .min(MAX_REQUEST_MAX_RETRIES)
    }

    /// Effective maximum number of stream reconnection attempts for this provider.
    pub fn stream_max_retries(&self) -> u64 {
        self.stream_max_retries
            .unwrap_or(DEFAULT_STREAM_MAX_RETRIES)
            .min(MAX_STREAM_MAX_RETRIES)
    }

    /// Effective idle timeout for streaming responses.
    pub fn stream_idle_timeout(&self) -> Duration {
        self.stream_idle_timeout_ms
            .map(Duration::from_millis)
            .unwrap_or(Duration::from_millis(DEFAULT_STREAM_IDLE_TIMEOUT_MS))
    }
    pub fn create_openai_provider() -> ModelProviderInfo {
        ModelProviderInfo {
            name: OPENAI_PROVIDER_NAME.into(),
            // Allow users to override the default OpenAI endpoint by
            // exporting `OPENAI_BASE_URL`. This is useful when pointing
            // Codex at a proxy, mock server, or Azure-style deployment
            // without requiring a full TOML override for the built-in
            // OpenAI provider.
            base_url: std::env::var("OPENAI_BASE_URL")
                .ok()
                .filter(|v| !v.trim().is_empty()),
            env_key: None,
            env_key_instructions: None,
            experimental_bearer_token: None,
            wire_api: WireApi::Responses,
            query_params: None,
            http_headers: Some(
                [("version".to_string(), env!("CARGO_PKG_VERSION").to_string())]
                    .into_iter()
                    .collect(),
            ),
            env_http_headers: Some(
                [
                    (
                        "OpenAI-Organization".to_string(),
                        "OPENAI_ORGANIZATION".to_string(),
                    ),
                    ("OpenAI-Project".to_string(), "OPENAI_PROJECT".to_string()),
                ]
                .into_iter()
                .collect(),
            ),
            // Use global defaults for retry/timeout unless overridden in config.toml.
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            requires_openai_auth: true,
            // OpenAI uses bearer token authentication
            auth_header_type: AuthHeaderType::Bearer,
            is_azure: false,
            skip_azure_detection: false,
        }
    }

    /// Creates a built-in Azure OpenAI provider configuration.
    ///
    /// This provider uses API key authentication by default, which is the most
    /// common auth method for Azure OpenAI. Users can configure Azure Entra ID
    /// authentication through config.toml.
    pub fn create_azure_provider() -> ModelProviderInfo {
        ModelProviderInfo {
            name: "Azure OpenAI".into(),
            // Allow users to set the Azure endpoint via environment variable
            base_url: std::env::var("AZURE_OPENAI_ENDPOINT")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .map(|mut url| {
                    // Ensure the URL ends with /openai/v1 for v1 API compatibility
                    if !url.ends_with("/openai/v1") && !url.ends_with("/openai/v1/") {
                        if url.ends_with('/') {
                            url.push_str("openai/v1");
                        } else {
                            url.push_str("/openai/v1");
                        }
                    }
                    url
                }),
            env_key: Some("AZURE_OPENAI_API_KEY".into()),
            env_key_instructions: Some(
                "Set AZURE_OPENAI_ENDPOINT to your Azure OpenAI resource endpoint \
                (e.g., https://your-resource.openai.azure.com) and AZURE_OPENAI_API_KEY \
                to your API key from the Azure Portal."
                    .into(),
            ),
            experimental_bearer_token: None,
            wire_api: WireApi::Responses,
            query_params: None,
            http_headers: None,
            env_http_headers: None,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            requires_openai_auth: false,
            // Azure uses api-key header by default
            auth_header_type: AuthHeaderType::ApiKey,
            is_azure: true,
            skip_azure_detection: false,
        }
    }

    pub fn is_openai(&self) -> bool {
        self.name == OPENAI_PROVIDER_NAME
    }

    /// Returns whether this provider should be treated as an Azure endpoint.
    ///
    /// Uses explicit `is_azure` flag if set, otherwise falls back to hostname-based
    /// detection unless `skip_azure_detection` is enabled.
    pub fn is_azure_endpoint(&self) -> bool {
        // Explicit flag takes precedence
        if self.is_azure {
            return true;
        }

        // Skip automatic detection if requested
        if self.skip_azure_detection {
            return false;
        }

        // Check provider name
        if self.name.eq_ignore_ascii_case("azure") || self.name.to_lowercase().contains("azure") {
            return true;
        }

        // Fall back to hostname detection
        self.base_url
            .as_ref()
            .is_some_and(|url| is_azure_hostname(url))
    }

    /// Returns the auth header type for this provider, taking into account
    /// Azure-specific defaults.
    pub fn effective_auth_header_type(&self) -> AuthHeaderType {
        // If explicitly set, use that
        if self.auth_header_type != AuthHeaderType::default() {
            return self.auth_header_type.clone();
        }

        // For Azure endpoints using API key auth, default to api-key header
        if self.is_azure_endpoint() && self.env_key.is_some() {
            return AuthHeaderType::ApiKey;
        }

        // Otherwise use bearer (default)
        AuthHeaderType::Bearer
    }
}

/// Checks if a URL hostname matches known Azure patterns.
fn is_azure_hostname(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    const AZURE_PATTERNS: [&str; 7] = [
        "openai.azure.com",
        "openai.azure.us",
        "cognitiveservices.azure.",
        "aoai.azure.",
        "azure-api.net",
        "azurefd.net",
        "windows.net/openai",
    ];
    AZURE_PATTERNS.iter().any(|pattern| lower.contains(pattern))
}

pub const DEFAULT_LMSTUDIO_PORT: u16 = 1234;
pub const DEFAULT_OLLAMA_PORT: u16 = 11434;

pub const LMSTUDIO_OSS_PROVIDER_ID: &str = "lmstudio";
pub const OLLAMA_OSS_PROVIDER_ID: &str = "ollama";
pub const AZURE_PROVIDER_ID: &str = "azure";

/// Built-in default provider list.
pub fn built_in_model_providers() -> HashMap<String, ModelProviderInfo> {
    use ModelProviderInfo as P;

    // Built-in providers include OpenAI, Azure OpenAI, and open source providers.
    // Users can add additional providers via `model_providers` in config.toml.
    [
        ("openai", P::create_openai_provider()),
        (AZURE_PROVIDER_ID, P::create_azure_provider()),
        (
            OLLAMA_OSS_PROVIDER_ID,
            create_oss_provider(DEFAULT_OLLAMA_PORT, WireApi::Chat),
        ),
        (
            LMSTUDIO_OSS_PROVIDER_ID,
            create_oss_provider(DEFAULT_LMSTUDIO_PORT, WireApi::Responses),
        ),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

pub fn create_oss_provider(default_provider_port: u16, wire_api: WireApi) -> ModelProviderInfo {
    // These AZURE_CODEX_OSS_ environment variables are experimental: we may
    // switch to reading values from config.toml instead.
    let oss_base_url = match std::env::var("AZURE_CODEX_OSS_BASE_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        Some(url) => url,
        None => format!(
            "http://localhost:{port}/v1",
            port = std::env::var("AZURE_CODEX_OSS_PORT")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .and_then(|v| v.parse::<u16>().ok())
                .unwrap_or(default_provider_port)
        ),
    };
    create_oss_provider_with_base_url(&oss_base_url, wire_api)
}

pub fn create_oss_provider_with_base_url(base_url: &str, wire_api: WireApi) -> ModelProviderInfo {
    ModelProviderInfo {
        name: "gpt-oss".into(),
        base_url: Some(base_url.into()),
        env_key: None,
        env_key_instructions: None,
        experimental_bearer_token: None,
        wire_api,
        query_params: None,
        http_headers: None,
        env_http_headers: None,
        request_max_retries: None,
        stream_max_retries: None,
        stream_idle_timeout_ms: None,
        requires_openai_auth: false,
        // OSS providers typically don't require auth
        auth_header_type: AuthHeaderType::Bearer,
        is_azure: false,
        skip_azure_detection: true, // Local providers shouldn't trigger Azure detection
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_deserialize_ollama_model_provider_toml() {
        let azure_provider_toml = r#"
name = "Ollama"
base_url = "http://localhost:11434/v1"
        "#;
        let expected_provider = ModelProviderInfo {
            name: "Ollama".into(),
            base_url: Some("http://localhost:11434/v1".into()),
            env_key: None,
            env_key_instructions: None,
            experimental_bearer_token: None,
            wire_api: WireApi::Chat,
            query_params: None,
            http_headers: None,
            env_http_headers: None,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            requires_openai_auth: false,
            auth_header_type: AuthHeaderType::Bearer,
            is_azure: false,
            skip_azure_detection: false,
        };

        let provider: ModelProviderInfo = toml::from_str(azure_provider_toml).unwrap();
        assert_eq!(expected_provider, provider);
    }

    #[test]
    fn test_deserialize_azure_model_provider_toml() {
        let azure_provider_toml = r#"
name = "Azure"
base_url = "https://xxxxx.openai.azure.com/openai"
env_key = "AZURE_OPENAI_API_KEY"
query_params = { api-version = "2025-04-01-preview" }
        "#;
        let expected_provider = ModelProviderInfo {
            name: "Azure".into(),
            base_url: Some("https://xxxxx.openai.azure.com/openai".into()),
            env_key: Some("AZURE_OPENAI_API_KEY".into()),
            env_key_instructions: None,
            experimental_bearer_token: None,
            wire_api: WireApi::Chat,
            query_params: Some(maplit::hashmap! {
                "api-version".to_string() => "2025-04-01-preview".to_string(),
            }),
            http_headers: None,
            env_http_headers: None,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            requires_openai_auth: false,
            auth_header_type: AuthHeaderType::Bearer,
            is_azure: false,
            skip_azure_detection: false,
        };

        let provider: ModelProviderInfo = toml::from_str(azure_provider_toml).unwrap();
        assert_eq!(expected_provider, provider);
    }

    #[test]
    fn test_deserialize_azure_with_api_key_auth() {
        let azure_provider_toml = r#"
name = "Azure"
base_url = "https://xxxxx.openai.azure.com/openai/v1"
env_key = "AZURE_OPENAI_API_KEY"
auth_header_type = "api_key"
is_azure = true
        "#;
        let expected_provider = ModelProviderInfo {
            name: "Azure".into(),
            base_url: Some("https://xxxxx.openai.azure.com/openai/v1".into()),
            env_key: Some("AZURE_OPENAI_API_KEY".into()),
            env_key_instructions: None,
            experimental_bearer_token: None,
            wire_api: WireApi::Chat,
            query_params: None,
            http_headers: None,
            env_http_headers: None,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            requires_openai_auth: false,
            auth_header_type: AuthHeaderType::ApiKey,
            is_azure: true,
            skip_azure_detection: false,
        };

        let provider: ModelProviderInfo = toml::from_str(azure_provider_toml).unwrap();
        assert_eq!(expected_provider, provider);
    }

    #[test]
    fn test_deserialize_example_model_provider_toml() {
        let azure_provider_toml = r#"
name = "Example"
base_url = "https://example.com"
env_key = "API_KEY"
http_headers = { "X-Example-Header" = "example-value" }
env_http_headers = { "X-Example-Env-Header" = "EXAMPLE_ENV_VAR" }
        "#;
        let expected_provider = ModelProviderInfo {
            name: "Example".into(),
            base_url: Some("https://example.com".into()),
            env_key: Some("API_KEY".into()),
            env_key_instructions: None,
            experimental_bearer_token: None,
            wire_api: WireApi::Chat,
            query_params: None,
            http_headers: Some(maplit::hashmap! {
                "X-Example-Header".to_string() => "example-value".to_string(),
            }),
            env_http_headers: Some(maplit::hashmap! {
                "X-Example-Env-Header".to_string() => "EXAMPLE_ENV_VAR".to_string(),
            }),
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            requires_openai_auth: false,
            auth_header_type: AuthHeaderType::Bearer,
            is_azure: false,
            skip_azure_detection: false,
        };

        let provider: ModelProviderInfo = toml::from_str(azure_provider_toml).unwrap();
        assert_eq!(expected_provider, provider);
    }

    #[test]
    fn detects_azure_responses_base_urls() {
        let positive_cases = [
            "https://foo.openai.azure.com/openai",
            "https://foo.openai.azure.us/openai/deployments/bar",
            "https://foo.cognitiveservices.azure.cn/openai",
            "https://foo.aoai.azure.com/openai",
            "https://foo.openai.azure-api.net/openai",
            "https://foo.z01.azurefd.net/",
        ];
        for base_url in positive_cases {
            let provider = ModelProviderInfo {
                name: "test".into(),
                base_url: Some(base_url.into()),
                env_key: None,
                env_key_instructions: None,
                experimental_bearer_token: None,
                wire_api: WireApi::Responses,
                query_params: None,
                http_headers: None,
                env_http_headers: None,
                request_max_retries: None,
                stream_max_retries: None,
                stream_idle_timeout_ms: None,
                requires_openai_auth: false,
                auth_header_type: AuthHeaderType::Bearer,
                is_azure: false,
                skip_azure_detection: false,
            };
            let api = provider.to_api_provider(None).expect("api provider");
            assert!(
                api.is_azure_responses_endpoint(),
                "expected {base_url} to be detected as Azure"
            );
        }

        let named_provider = ModelProviderInfo {
            name: "Azure".into(),
            base_url: Some("https://example.com".into()),
            env_key: None,
            env_key_instructions: None,
            experimental_bearer_token: None,
            wire_api: WireApi::Responses,
            query_params: None,
            http_headers: None,
            env_http_headers: None,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            requires_openai_auth: false,
            auth_header_type: AuthHeaderType::Bearer,
            is_azure: false,
            skip_azure_detection: false,
        };
        let named_api = named_provider.to_api_provider(None).expect("api provider");
        assert!(named_api.is_azure_responses_endpoint());

        let negative_cases = [
            "https://api.openai.com/v1",
            "https://example.com/openai",
            "https://myproxy.azurewebsites.net/openai",
        ];
        for base_url in negative_cases {
            let provider = ModelProviderInfo {
                name: "test".into(),
                base_url: Some(base_url.into()),
                env_key: None,
                env_key_instructions: None,
                experimental_bearer_token: None,
                wire_api: WireApi::Responses,
                query_params: None,
                http_headers: None,
                env_http_headers: None,
                request_max_retries: None,
                stream_max_retries: None,
                stream_idle_timeout_ms: None,
                requires_openai_auth: false,
                auth_header_type: AuthHeaderType::Bearer,
                is_azure: false,
                skip_azure_detection: false,
            };
            let api = provider.to_api_provider(None).expect("api provider");
            assert!(
                !api.is_azure_responses_endpoint(),
                "expected {base_url} not to be detected as Azure"
            );
        }
    }

    #[test]
    fn test_is_azure_endpoint_explicit_flag() {
        let provider = ModelProviderInfo {
            name: "Custom".into(),
            base_url: Some("https://my-custom-gateway.com/openai".into()),
            env_key: Some("API_KEY".into()),
            env_key_instructions: None,
            experimental_bearer_token: None,
            wire_api: WireApi::Responses,
            query_params: None,
            http_headers: None,
            env_http_headers: None,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            requires_openai_auth: false,
            auth_header_type: AuthHeaderType::ApiKey,
            is_azure: true, // Explicitly marked as Azure
            skip_azure_detection: false,
        };
        assert!(provider.is_azure_endpoint());
    }

    #[test]
    fn test_is_azure_endpoint_skip_detection() {
        let provider = ModelProviderInfo {
            name: "Azure Mock".into(),
            base_url: Some("https://foo.openai.azure.com/openai".into()),
            env_key: None,
            env_key_instructions: None,
            experimental_bearer_token: None,
            wire_api: WireApi::Responses,
            query_params: None,
            http_headers: None,
            env_http_headers: None,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            requires_openai_auth: false,
            auth_header_type: AuthHeaderType::Bearer,
            is_azure: false,
            skip_azure_detection: true, // Skip detection even though URL looks like Azure
        };
        assert!(!provider.is_azure_endpoint());
    }

    #[test]
    fn test_effective_auth_header_type_azure_defaults() {
        // Azure endpoint with env_key should default to ApiKey
        let provider = ModelProviderInfo {
            name: "Azure".into(),
            base_url: Some("https://foo.openai.azure.com/openai/v1".into()),
            env_key: Some("AZURE_OPENAI_API_KEY".into()),
            env_key_instructions: None,
            experimental_bearer_token: None,
            wire_api: WireApi::Responses,
            query_params: None,
            http_headers: None,
            env_http_headers: None,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            requires_openai_auth: false,
            auth_header_type: AuthHeaderType::Bearer, // Default
            is_azure: false,
            skip_azure_detection: false,
        };
        // Should auto-detect as Azure and use ApiKey
        assert_eq!(
            provider.effective_auth_header_type(),
            AuthHeaderType::ApiKey
        );
    }

    #[test]
    fn test_effective_auth_header_type_explicit_override() {
        // Explicitly set to Bearer should override Azure default
        let provider = ModelProviderInfo {
            name: "Azure Entra".into(),
            base_url: Some("https://foo.openai.azure.com/openai/v1".into()),
            env_key: None, // Using Entra ID, not API key
            env_key_instructions: None,
            experimental_bearer_token: Some("entra-token".into()),
            wire_api: WireApi::Responses,
            query_params: None,
            http_headers: None,
            env_http_headers: None,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            requires_openai_auth: false,
            auth_header_type: AuthHeaderType::Bearer,
            is_azure: true,
            skip_azure_detection: false,
        };
        // No env_key, so should keep Bearer
        assert_eq!(
            provider.effective_auth_header_type(),
            AuthHeaderType::Bearer
        );
    }
}
