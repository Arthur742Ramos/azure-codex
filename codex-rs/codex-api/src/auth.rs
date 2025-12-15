use codex_client::Request;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Authentication header type for API requests.
///
/// Different providers expect credentials in different header formats:
/// - OpenAI uses `Authorization: Bearer <token>`
/// - Azure OpenAI can use either `api-key: <key>` or `Authorization: Bearer <token>`
/// - Custom APIM gateways may use custom header names
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthHeaderType {
    /// Standard OAuth2 Bearer token: `Authorization: Bearer <token>`
    #[default]
    Bearer,
    /// Azure-style API key: `api-key: <key>`
    ApiKey,
    /// Custom header name with the key/token as value
    #[serde(rename = "custom")]
    Custom(String),
}

impl fmt::Display for AuthHeaderType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthHeaderType::Bearer => write!(f, "bearer"),
            AuthHeaderType::ApiKey => write!(f, "api_key"),
            AuthHeaderType::Custom(name) => write!(f, "custom({name})"),
        }
    }
}

/// Provides authentication information for API requests.
///
/// Implementations should be cheap and non-blocking; any asynchronous
/// refresh or I/O should be handled by higher layers before requests
/// reach this interface.
pub trait AuthProvider: Send + Sync {
    /// Returns the bearer token for Authorization header.
    fn bearer_token(&self) -> Option<String>;

    /// Returns the API key (used for api-key header style auth).
    /// Defaults to returning the bearer token if not overridden.
    fn api_key(&self) -> Option<String> {
        self.bearer_token()
    }

    /// Returns the authentication header type to use.
    fn auth_header_type(&self) -> AuthHeaderType {
        AuthHeaderType::Bearer
    }

    /// Returns the account ID for ChatGPT-Account-ID header.
    fn account_id(&self) -> Option<String> {
        None
    }

    /// Returns whether this provider is for an Azure endpoint.
    /// Used to determine Azure-specific request handling.
    fn is_azure(&self) -> bool {
        false
    }
}

/// Adds authentication headers to a request based on the auth provider's configuration.
pub(crate) fn add_auth_headers<A: AuthProvider>(auth: &A, mut req: Request) -> Request {
    match auth.auth_header_type() {
        AuthHeaderType::Bearer => {
            if let Some(token) = auth.bearer_token()
                && let Ok(header) = format!("Bearer {token}").parse()
            {
                let _ = req.headers.insert(http::header::AUTHORIZATION, header);
            }
        }
        AuthHeaderType::ApiKey => {
            if let Some(key) = auth.api_key()
                && let Ok(header) = key.parse()
            {
                let _ = req.headers.insert("api-key", header);
            }
        }
        AuthHeaderType::Custom(header_name) => {
            if let Some(key) = auth.api_key()
                && let Ok(name) = header_name.parse::<http::HeaderName>()
                && let Ok(value) = key.parse()
            {
                let _ = req.headers.insert(name, value);
            }
        }
    }

    if let Some(account_id) = auth.account_id()
        && let Ok(header) = account_id.parse()
    {
        let _ = req.headers.insert("ChatGPT-Account-ID", header);
    }
    req
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderMap;

    struct TestAuthProvider {
        token: Option<String>,
        api_key: Option<String>,
        header_type: AuthHeaderType,
        is_azure: bool,
    }

    impl AuthProvider for TestAuthProvider {
        fn bearer_token(&self) -> Option<String> {
            self.token.clone()
        }

        fn api_key(&self) -> Option<String> {
            self.api_key.clone().or_else(|| self.token.clone())
        }

        fn auth_header_type(&self) -> AuthHeaderType {
            self.header_type.clone()
        }

        fn is_azure(&self) -> bool {
            self.is_azure
        }
    }

    fn create_test_request() -> Request {
        Request {
            method: http::Method::POST,
            url: "https://api.example.com/test".to_string(),
            headers: HeaderMap::new(),
            body: None,
            timeout: None,
        }
    }

    #[test]
    fn test_bearer_auth_header() {
        let auth = TestAuthProvider {
            token: Some("test-token".to_string()),
            api_key: None,
            header_type: AuthHeaderType::Bearer,
            is_azure: false,
        };

        let req = add_auth_headers(&auth, create_test_request());

        assert_eq!(
            req.headers.get(http::header::AUTHORIZATION).unwrap(),
            "Bearer test-token"
        );
        assert!(req.headers.get("api-key").is_none());
    }

    #[test]
    fn test_api_key_auth_header() {
        let auth = TestAuthProvider {
            token: None,
            api_key: Some("my-api-key".to_string()),
            header_type: AuthHeaderType::ApiKey,
            is_azure: true,
        };

        let req = add_auth_headers(&auth, create_test_request());

        assert_eq!(req.headers.get("api-key").unwrap(), "my-api-key");
        assert!(req.headers.get(http::header::AUTHORIZATION).is_none());
    }

    #[test]
    fn test_custom_auth_header() {
        let auth = TestAuthProvider {
            token: None,
            api_key: Some("subscription-key".to_string()),
            header_type: AuthHeaderType::Custom("Ocp-Apim-Subscription-Key".to_string()),
            is_azure: true,
        };

        let req = add_auth_headers(&auth, create_test_request());

        assert_eq!(
            req.headers.get("Ocp-Apim-Subscription-Key").unwrap(),
            "subscription-key"
        );
        assert!(req.headers.get(http::header::AUTHORIZATION).is_none());
        assert!(req.headers.get("api-key").is_none());
    }

    #[test]
    fn test_api_key_falls_back_to_bearer_token() {
        let auth = TestAuthProvider {
            token: Some("fallback-token".to_string()),
            api_key: None,
            header_type: AuthHeaderType::ApiKey,
            is_azure: true,
        };

        let req = add_auth_headers(&auth, create_test_request());

        assert_eq!(req.headers.get("api-key").unwrap(), "fallback-token");
    }
}
