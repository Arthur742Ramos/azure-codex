use chrono::DateTime;
use chrono::Utc;
use codex_api::AuthHeaderType;
use codex_api::AuthProvider as ApiAuthProvider;
use codex_api::TransportError;
use codex_api::error::ApiError;
use codex_api::rate_limits::parse_rate_limit;
use http::HeaderMap;
use serde::Deserialize;

use crate::auth::azure::AzureAuth;
use crate::error::CodexErr;
use crate::error::RetryLimitReachedError;
use crate::error::UnexpectedResponseError;
use crate::error::UsageLimitReachedError;
use crate::model_provider_info::ModelProviderInfo;
use crate::token_data::PlanType;

pub(crate) fn map_api_error(err: ApiError) -> CodexErr {
    match err {
        ApiError::ContextWindowExceeded => CodexErr::ContextWindowExceeded,
        ApiError::QuotaExceeded => CodexErr::QuotaExceeded,
        ApiError::UsageNotIncluded => CodexErr::UsageNotIncluded,
        ApiError::Retryable { message, delay } => CodexErr::Stream(message, delay),
        ApiError::Stream(msg) => CodexErr::Stream(msg, None),
        ApiError::Api { status, message } => CodexErr::UnexpectedStatus(UnexpectedResponseError {
            status,
            body: message,
            request_id: None,
        }),
        ApiError::Transport(transport) => match transport {
            TransportError::Http {
                status,
                headers,
                body,
            } => {
                let body_text = body.unwrap_or_default();

                if status == http::StatusCode::BAD_REQUEST {
                    if body_text
                        .contains("The image data you provided does not represent a valid image")
                    {
                        CodexErr::InvalidImageRequest()
                    } else {
                        CodexErr::InvalidRequest(body_text)
                    }
                } else if status == http::StatusCode::INTERNAL_SERVER_ERROR {
                    CodexErr::InternalServerError
                } else if status == http::StatusCode::TOO_MANY_REQUESTS {
                    if let Ok(err) = serde_json::from_str::<UsageErrorResponse>(&body_text) {
                        if err.error.error_type.as_deref() == Some("usage_limit_reached") {
                            let rate_limits = headers.as_ref().and_then(parse_rate_limit);
                            let resets_at = err
                                .error
                                .resets_at
                                .and_then(|seconds| DateTime::<Utc>::from_timestamp(seconds, 0));
                            return CodexErr::UsageLimitReached(UsageLimitReachedError {
                                plan_type: err.error.plan_type,
                                resets_at,
                                rate_limits,
                            });
                        } else if err.error.error_type.as_deref() == Some("usage_not_included") {
                            return CodexErr::UsageNotIncluded;
                        }
                    }

                    CodexErr::RetryLimit(RetryLimitReachedError {
                        status,
                        request_id: extract_request_id(headers.as_ref()),
                    })
                } else {
                    CodexErr::UnexpectedStatus(UnexpectedResponseError {
                        status,
                        body: body_text,
                        request_id: extract_request_id(headers.as_ref()),
                    })
                }
            }
            TransportError::RetryLimit => CodexErr::RetryLimit(RetryLimitReachedError {
                status: http::StatusCode::INTERNAL_SERVER_ERROR,
                request_id: None,
            }),
            TransportError::Timeout => CodexErr::Timeout,
            TransportError::Network(msg) | TransportError::Build(msg) => {
                CodexErr::Stream(msg, None)
            }
        },
        ApiError::RateLimit(msg) => CodexErr::Stream(msg, None),
    }
}

fn extract_request_id(headers: Option<&HeaderMap>) -> Option<String> {
    headers.and_then(|map| {
        ["cf-ray", "x-request-id", "x-oai-request-id"]
            .iter()
            .find_map(|name| {
                map.get(*name)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
            })
    })
}

/// Create an auth provider for Azure Codex API calls.
///
/// Azure Codex is designed exclusively for Azure OpenAI endpoints and uses
/// Azure Entra ID authentication. The priority order is:
///
/// 1. Provider-specific API key (from config or AZURE_OPENAI_API_KEY env)
/// 2. Experimental bearer token from provider config
/// 3. Azure Entra ID authentication (DefaultAzureCredential, etc.)
pub(crate) async fn auth_provider_from_auth(
    azure_auth: Option<&AzureAuth>,
    provider: &ModelProviderInfo,
) -> crate::error::Result<CoreAuthProvider> {
    // Determine the effective auth header type for this provider
    let auth_header_type = provider.effective_auth_header_type();
    let is_azure = provider.is_azure_endpoint();

    // Priority 1: Provider-specific API key (from config or env var)
    // This supports AZURE_OPENAI_API_KEY for users who prefer API key auth
    // Note: We use ok().flatten() to treat missing env vars as None, allowing
    // fallback to Azure Entra ID auth instead of failing immediately.
    if let Some(api_key) = provider.api_key().ok().flatten() {
        tracing::debug!("Using API key for Azure endpoint");
        return Ok(CoreAuthProvider {
            token: Some(api_key),
            account_id: None,
            auth_header_type,
            is_azure: true,
        });
    }

    // Priority 2: Experimental bearer token from provider config
    if let Some(token) = provider.experimental_bearer_token.clone() {
        return Ok(CoreAuthProvider {
            token: Some(token),
            account_id: None,
            auth_header_type,
            is_azure: true,
        });
    }

    // Priority 3: Use Azure Entra ID authentication
    if let Some(azure) = azure_auth {
        match azure.get_token().await {
            Ok(token) => {
                tracing::debug!("Using Azure Entra ID token");
                return Ok(CoreAuthProvider {
                    token: Some(token),
                    account_id: None,
                    // Azure OpenAI uses Bearer tokens for Entra ID auth
                    auth_header_type: AuthHeaderType::Bearer,
                    is_azure: true,
                });
            }
            Err(e) => {
                tracing::error!("Failed to get Azure Entra ID token: {}", e);
                return Err(crate::error::CodexErr::Authentication(format!(
                    "Azure authentication failed: {e}. Please ensure you are logged in via Azure CLI, \
                     have AZURE_CLIENT_ID/AZURE_CLIENT_SECRET/AZURE_TENANT_ID set, \
                     or are running in Azure with managed identity."
                )));
            }
        }
    }

    // No authentication configured - return error for Azure endpoints
    if is_azure {
        return Err(crate::error::CodexErr::Authentication(
            "Azure authentication required but not configured. \
             Set AZURE_OPENAI_API_KEY or configure azure_auth in config.toml"
                .to_string(),
        ));
    }

    // For non-Azure endpoints (shouldn't happen in Azure Codex), allow unauthenticated
    Ok(CoreAuthProvider {
        token: None,
        account_id: None,
        auth_header_type,
        is_azure: false,
    })
}

#[derive(Debug, Deserialize)]
struct UsageErrorResponse {
    error: UsageErrorBody,
}

#[derive(Debug, Deserialize)]
struct UsageErrorBody {
    #[serde(rename = "type")]
    error_type: Option<String>,
    plan_type: Option<PlanType>,
    resets_at: Option<i64>,
}

#[derive(Clone)]
pub(crate) struct CoreAuthProvider {
    token: Option<String>,
    account_id: Option<String>,
    auth_header_type: AuthHeaderType,
    is_azure: bool,
}

impl Default for CoreAuthProvider {
    fn default() -> Self {
        Self {
            token: None,
            account_id: None,
            auth_header_type: AuthHeaderType::Bearer,
            is_azure: false,
        }
    }
}

impl ApiAuthProvider for CoreAuthProvider {
    fn bearer_token(&self) -> Option<String> {
        self.token.clone()
    }

    fn api_key(&self) -> Option<String> {
        self.token.clone()
    }

    fn auth_header_type(&self) -> AuthHeaderType {
        self.auth_header_type.clone()
    }

    fn account_id(&self) -> Option<String> {
        self.account_id.clone()
    }

    fn is_azure(&self) -> bool {
        self.is_azure
    }
}
