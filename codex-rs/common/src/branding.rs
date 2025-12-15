//! Azure Codex branding and configuration constants.
//!
//! This module centralizes all branding-related constants to ensure consistency
//! across the codebase and to make it easy to rebrand the application.
//!
//! ## Configuration Separation
//!
//! Azure Codex uses a separate configuration directory (`~/.azure-codex/`) from
//! the original OpenAI Codex (`~/.codex/`) to allow both tools to coexist on
//! the same machine without conflicts.

/// The application name used in user-facing messages and documentation.
pub const APP_NAME: &str = "Azure Codex";

/// The short application name used in CLI and logs.
pub const APP_NAME_SHORT: &str = "azure-codex";

/// The configuration directory name (without leading dot for internal use).
/// The actual directory will be `~/.azure-codex/`.
pub const CONFIG_DIR_NAME: &str = ".azure-codex";

/// Environment variable to override the configuration directory location.
/// Takes precedence over the default `~/.azure-codex/` location.
pub const ENV_VAR_HOME: &str = "AZURE_CODEX_HOME";

/// The directory name used for project-local configuration.
/// This is placed in repository roots for project-specific settings.
pub const REPO_CONFIG_DIR_NAME: &str = ".azure-codex";

/// macOS managed preferences application identifier.
/// Used for enterprise configuration via MDM profiles.
pub const MACOS_PREFERENCES_ID: &str = "com.azure.codex";

/// Keyring service name for storing credentials.
pub const KEYRING_SERVICE_NAME: &str = "azure-codex";

/// User agent string for HTTP requests.
pub const USER_AGENT: &str = concat!("azure-codex/", env!("CARGO_PKG_VERSION"));

/// Environment variable prefix for Azure Codex specific variables.
/// e.g., AZURE_CODEX_HOME, AZURE_CODEX_API_KEY
pub const ENV_VAR_PREFIX: &str = "AZURE_CODEX_";

// ============================================================================
// Azure-specific constants
// ============================================================================

/// Default Azure OpenAI API scope for Entra ID authentication.
pub const AZURE_DEFAULT_SCOPE: &str = "https://cognitiveservices.azure.com/.default";

/// Azure US Government scope.
pub const AZURE_US_GOV_SCOPE: &str = "https://cognitiveservices.azure.us/.default";

/// Azure China scope.
pub const AZURE_CHINA_SCOPE: &str = "https://cognitiveservices.azure.cn/.default";

/// Default Azure AD authority (public cloud).
pub const AZURE_PUBLIC_AUTHORITY: &str = "https://login.microsoftonline.com";

/// Azure US Government authority.
pub const AZURE_US_GOV_AUTHORITY: &str = "https://login.microsoftonline.us";

/// Azure China authority.
pub const AZURE_CHINA_AUTHORITY: &str = "https://login.chinacloudapi.cn";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_starts_with_dot() {
        assert!(CONFIG_DIR_NAME.starts_with('.'));
    }

    #[test]
    fn repo_config_dir_starts_with_dot() {
        assert!(REPO_CONFIG_DIR_NAME.starts_with('.'));
    }

    #[test]
    fn env_var_is_uppercase() {
        assert_eq!(ENV_VAR_HOME, ENV_VAR_HOME.to_uppercase());
    }
}
