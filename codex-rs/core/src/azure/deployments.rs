//! Azure OpenAI deployments discovery.
//!
//! This module provides functionality to discover and list Azure OpenAI
//! deployments using the Azure CLI.

use codex_protocol::openai_models::ModelPreset;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::openai_models::ReasoningEffortPreset;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;
use tracing::error;
use tracing::warn;

/// Information about an Azure OpenAI deployment.
#[derive(Debug, Clone, Deserialize)]
pub struct AzureDeployment {
    /// The deployment name (used as the model identifier in API calls).
    pub name: String,

    /// The resource group containing this deployment.
    #[serde(rename = "resourceGroup")]
    pub resource_group: Option<String>,

    /// Additional properties from the Azure API.
    #[serde(default)]
    pub properties: AzureDeploymentProperties,
}

/// Additional properties of an Azure OpenAI deployment.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AzureDeploymentProperties {
    /// The underlying model name.
    #[serde(default)]
    pub model: Option<AzureModelInfo>,

    /// Provisioning state of the deployment.
    #[serde(rename = "provisioningState")]
    pub provisioning_state: Option<String>,

    /// Capabilities of the deployment (chatCompletion, responses, etc.)
    #[serde(default)]
    pub capabilities: Option<AzureDeploymentCapabilities>,
}

/// Capabilities of an Azure deployment that determine which APIs it supports.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AzureDeploymentCapabilities {
    /// Whether the deployment supports Chat Completions API.
    #[serde(rename = "chatCompletion")]
    pub chat_completion: Option<String>,

    /// Whether the deployment supports the Responses API.
    pub responses: Option<String>,

    /// Whether the deployment supports embeddings.
    pub embeddings: Option<String>,

    /// Whether the deployment supports assistants.
    pub assistants: Option<String>,

    /// Whether the deployment supports agents.
    #[serde(rename = "agentsV2")]
    pub agents_v2: Option<String>,
}

/// Information about the model backing an Azure deployment.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AzureModelInfo {
    /// The model name (e.g., "gpt-4", "gpt-35-turbo").
    pub name: Option<String>,

    /// The model version.
    pub version: Option<String>,

    /// The model format.
    pub format: Option<String>,
}

/// Manages discovery of Azure OpenAI deployments.
#[derive(Debug)]
pub struct AzureDeploymentsManager {
    /// Cached list of deployments.
    deployments: RwLock<Vec<AzureDeployment>>,

    /// The Azure endpoint (e.g., "https://myresource.services.ai.azure.com").
    /// Supports both Azure AI Services and Azure OpenAI endpoint formats.
    endpoint: Option<String>,

    /// Cached account name extracted from endpoint.
    account_name: RwLock<Option<String>>,

    /// Cached resource group.
    resource_group: RwLock<Option<String>>,
}

impl AzureDeploymentsManager {
    /// Create a new deployments manager.
    pub fn new(endpoint: Option<String>) -> Self {
        Self {
            deployments: RwLock::new(Vec::new()),
            endpoint,
            account_name: RwLock::new(None),
            resource_group: RwLock::new(None),
        }
    }

    /// Extract the account name from an Azure endpoint URL.
    fn extract_account_name(endpoint: &str) -> Option<String> {
        // Supported endpoint formats:
        //   - https://{account-name}.services.ai.azure.com (Azure AI Services - preferred)
        //   - https://{account-name}.openai.azure.com (Azure OpenAI - legacy)
        //   - https://{account-name}-{region}.openai.azure.com
        let url = endpoint.trim_end_matches('/');
        let host = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"))?;

        // Get the subdomain (everything before the first dot)
        let account = host.split('.').next()?;
        Some(account.to_string())
    }

    /// Discover the resource group for the Azure OpenAI account.
    async fn discover_resource_group(&self, account_name: &str) -> Option<String> {
        // Check cache first
        if let Some(rg) = self.resource_group.read().await.clone() {
            return Some(rg);
        }

        debug!("Discovering resource group for account: {}", account_name);

        // Use Azure CLI to find the resource group
        #[cfg(windows)]
        let output = tokio::process::Command::new("cmd")
            .args([
                "/C",
                "az",
                "cognitiveservices",
                "account",
                "list",
                "--query",
                &format!("[?contains(name,'{account_name}')].resourceGroup | [0]"),
                "-o",
                "tsv",
            ])
            .output()
            .await;

        #[cfg(not(windows))]
        let output = tokio::process::Command::new("az")
            .args([
                "cognitiveservices",
                "account",
                "list",
                "--query",
                &format!("[?contains(name,'{account_name}')].resourceGroup | [0]"),
                "-o",
                "tsv",
            ])
            .output()
            .await;

        match output {
            Ok(output) if output.status.success() => {
                let rg = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !rg.is_empty() {
                    debug!("Found resource group: {}", rg);
                    *self.resource_group.write().await = Some(rg.clone());
                    return Some(rg);
                }
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                warn!("Failed to discover resource group: {}", stderr);
            }
            Err(e) => {
                error!("Failed to run az CLI: {}", e);
            }
        }

        None
    }

    /// List all deployments from the Azure OpenAI account.
    async fn list_deployments_from_azure(
        &self,
        account_name: &str,
        resource_group: &str,
    ) -> Vec<AzureDeployment> {
        debug!(
            "Listing deployments for account: {}, resource group: {}",
            account_name, resource_group
        );

        #[cfg(windows)]
        let output = tokio::process::Command::new("cmd")
            .args([
                "/C",
                "az",
                "cognitiveservices",
                "account",
                "deployment",
                "list",
                "--name",
                account_name,
                "--resource-group",
                resource_group,
                "-o",
                "json",
            ])
            .output()
            .await;

        #[cfg(not(windows))]
        let output = tokio::process::Command::new("az")
            .args([
                "cognitiveservices",
                "account",
                "deployment",
                "list",
                "--name",
                account_name,
                "--resource-group",
                resource_group,
                "-o",
                "json",
            ])
            .output()
            .await;

        match output {
            Ok(output) if output.status.success() => {
                let json_str = String::from_utf8_lossy(&output.stdout);
                match serde_json::from_str::<Vec<AzureDeployment>>(&json_str) {
                    Ok(deployments) => {
                        debug!("Found {} deployments", deployments.len());
                        return deployments;
                    }
                    Err(e) => {
                        error!("Failed to parse deployments JSON: {}", e);
                    }
                }
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                error!("Failed to list deployments: {}", stderr);
            }
            Err(e) => {
                error!("Failed to run az CLI: {}", e);
            }
        }

        Vec::new()
    }

    /// Refresh the list of deployments from Azure.
    pub async fn refresh_deployments(&self) -> Vec<AzureDeployment> {
        let endpoint = match &self.endpoint {
            Some(e) => e.clone(),
            None => {
                debug!("No Azure endpoint configured, skipping deployment discovery");
                return Vec::new();
            }
        };

        let account_name = match Self::extract_account_name(&endpoint) {
            Some(name) => {
                *self.account_name.write().await = Some(name.clone());
                name
            }
            None => {
                error!("Failed to extract account name from endpoint: {}", endpoint);
                return Vec::new();
            }
        };

        let resource_group = match self.discover_resource_group(&account_name).await {
            Some(rg) => rg,
            None => {
                error!(
                    "Failed to discover resource group for account: {}",
                    account_name
                );
                return Vec::new();
            }
        };

        let deployments = self
            .list_deployments_from_azure(&account_name, &resource_group)
            .await;

        *self.deployments.write().await = deployments.clone();
        deployments
    }

    /// Get cached deployments or refresh if empty.
    pub async fn get_deployments(&self) -> Vec<AzureDeployment> {
        let cached = self.deployments.read().await.clone();
        if !cached.is_empty() {
            return cached;
        }
        self.refresh_deployments().await
    }

    /// Get only GPT model deployments (filtered by name starting with "gpt").
    pub async fn get_gpt_deployments(&self) -> Vec<AzureDeployment> {
        self.get_deployments()
            .await
            .into_iter()
            .filter(|d| is_supported_model(&d.name, d.underlying_model_name()))
            .collect()
    }

    /// Check if we have a valid Azure endpoint configured.
    pub fn has_endpoint(&self) -> bool {
        self.endpoint.is_some()
    }

    /// Find a specific deployment by name.
    pub async fn find_deployment(&self, name: &str) -> Option<AzureDeployment> {
        self.get_deployments()
            .await
            .into_iter()
            .find(|d| d.name.eq_ignore_ascii_case(name))
    }

    /// Get the preferred wire API for a specific model/deployment.
    /// Returns None if deployment not found, in which case caller should use default.
    pub async fn get_wire_api_for_model(
        &self,
        model_name: &str,
    ) -> Option<crate::model_provider_info::WireApi> {
        self.find_deployment(model_name)
            .await
            .map(|d| d.preferred_wire_api())
    }

    /// Get GPT deployments as ModelPresets for the picker.
    pub async fn get_gpt_model_presets(&self) -> Vec<ModelPreset> {
        let deployments = self.get_gpt_deployments().await;
        let mut presets: Vec<ModelPreset> = deployments
            .into_iter()
            .map(|d| d.to_model_preset())
            .collect();

        // Sort by name for consistent ordering
        presets.sort_by(|a, b| a.model.cmp(&b.model));

        // Mark the first one as default if there are any
        if let Some(first) = presets.first_mut() {
            first.is_default = true;
        }

        presets
    }
}

impl AzureDeployment {
    /// Get the underlying model name if available.
    pub fn underlying_model_name(&self) -> Option<&str> {
        self.properties
            .model
            .as_ref()
            .and_then(|m| m.name.as_deref())
    }

    /// Returns true if this deployment supports the Responses API.
    pub fn supports_responses_api(&self) -> bool {
        self.properties
            .capabilities
            .as_ref()
            .and_then(|c| c.responses.as_ref())
            .is_some_and(|v| v == "true")
    }

    /// Returns true if this deployment supports Chat Completions API.
    pub fn supports_chat_completions(&self) -> bool {
        self.properties
            .capabilities
            .as_ref()
            .and_then(|c| c.chat_completion.as_ref())
            .is_some_and(|v| v == "true")
    }

    /// Returns true if Chat Completions is explicitly disabled (false, not just missing).
    pub fn chat_completions_disabled(&self) -> bool {
        self.properties
            .capabilities
            .as_ref()
            .and_then(|c| c.chat_completion.as_ref())
            .is_some_and(|v| v == "false")
    }

    /// Determines the preferred wire API for this deployment.
    /// Priority:
    /// 1. If only responses API is available (chatCompletion explicitly false), use Responses
    /// 2. If responses API is available and chatCompletion is not explicitly false, use Responses
    /// 3. If only chatCompletion is available, use Chat
    /// 4. Default to Responses for unknown/missing capabilities (for backwards compatibility)
    pub fn preferred_wire_api(&self) -> crate::model_provider_info::WireApi {
        use crate::model_provider_info::WireApi;

        let supports_responses = self.supports_responses_api();
        let supports_chat = self.supports_chat_completions();
        let chat_disabled = self.chat_completions_disabled();

        debug!(
            deployment = %self.name,
            supports_responses = %supports_responses,
            supports_chat = %supports_chat,
            chat_disabled = %chat_disabled,
            "Determining preferred wire API for deployment"
        );

        // If chatCompletion is explicitly disabled, must use Responses
        if chat_disabled {
            return WireApi::Responses;
        }

        // If responses is supported, prefer it (enables reasoning features)
        if supports_responses {
            return WireApi::Responses;
        }

        // If only chat is supported (e.g., Claude, Grok), use Chat
        if supports_chat {
            return WireApi::Chat;
        }

        // Default to Responses for backwards compatibility with models
        // that don't report capabilities (older deployments)
        WireApi::Responses
    }

    /// Convert this Azure deployment to a ModelPreset for the model picker.
    pub fn to_model_preset(&self) -> ModelPreset {
        // Create a display name from the deployment name
        let display_name = format_display_name(&self.name);

        // Get the underlying model name if available
        let underlying_model = self
            .properties
            .model
            .as_ref()
            .and_then(|m| m.name.as_deref());

        let description = match &self.properties.model {
            Some(model) => {
                let model_name = model.name.as_deref().unwrap_or("Unknown");
                let version = model.version.as_deref().unwrap_or("");
                if version.is_empty() {
                    format!("Azure deployment ({model_name})")
                } else {
                    format!("Azure deployment ({model_name} v{version})")
                }
            }
            None => "Azure OpenAI deployment".to_string(),
        };

        // Determine supported reasoning efforts based on the model
        let supported_reasoning_efforts =
            get_reasoning_efforts_for_model(&self.name, underlying_model);
        let default_reasoning_effort = supported_reasoning_efforts
            .default_effort
            .unwrap_or(ReasoningEffort::Medium);

        ModelPreset {
            id: self.name.clone(),
            model: self.name.clone(),
            display_name,
            description,
            default_reasoning_effort,
            supported_reasoning_efforts: supported_reasoning_efforts.presets,
            is_default: false,
            upgrade: None,
            show_in_picker: true,
            supported_in_api: true, // Azure deployments are always API-accessible
        }
    }
}

/// Supported model prefixes for the model picker.
/// Includes GPT models, Claude models, and other Azure AI deployments.
const SUPPORTED_MODEL_PREFIXES: &[&str] = &[
    "gpt", "claude", "o1", // OpenAI o1 models
    "o3", // OpenAI o3 models
];

/// Check if a deployment is a supported model for the picker.
/// Checks both the deployment name and the underlying model name.
fn is_supported_model(deployment_name: &str, underlying_model: Option<&str>) -> bool {
    let name_lower = deployment_name.to_lowercase();

    // Check deployment name
    for prefix in SUPPORTED_MODEL_PREFIXES {
        if name_lower.starts_with(prefix) {
            return true;
        }
    }

    // Check underlying model name
    if let Some(model) = underlying_model {
        let model_lower = model.to_lowercase();
        for prefix in SUPPORTED_MODEL_PREFIXES {
            if model_lower.starts_with(prefix) {
                return true;
            }
        }
    }

    false
}

/// Check if a model is a Claude model.
fn is_claude_model(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.starts_with("claude") || lower.contains("claude")
}

/// Check if a model supports xHigh reasoning based on its name.
/// Models that support xHigh: gpt-5.1-codex-max, gpt-5.2
fn supports_xhigh(model_name: &str) -> bool {
    let name_lower = model_name.to_lowercase();
    name_lower.contains("gpt-5.1-codex-max")
        || name_lower.contains("gpt-5.2")
        || name_lower.starts_with("gpt-5.2")
}

/// Get the appropriate reasoning efforts for a model.
/// Checks both deployment name and underlying model name.
fn get_reasoning_efforts_for_model(
    deployment_name: &str,
    underlying_model: Option<&str>,
) -> ReasoningSupport {
    let name_lower = deployment_name.to_lowercase();
    let underlying_lower = underlying_model.map(str::to_lowercase);

    // Check for Claude models (Azure AI deployed)
    let is_claude =
        is_claude_model(deployment_name) || underlying_model.is_some_and(is_claude_model);

    if is_claude {
        // Claude models support extended thinking but with different terminology
        let is_opus = name_lower.contains("opus")
            || underlying_lower
                .as_ref()
                .is_some_and(|m| m.contains("opus"));

        if is_opus {
            // Claude Opus has the highest capabilities
            return ReasoningSupport {
                default_effort: Some(ReasoningEffort::High),
                presets: vec![
                    effort(ReasoningEffort::Medium, "Balanced thinking"),
                    effort(ReasoningEffort::High, "Extended thinking"),
                    effort(ReasoningEffort::XHigh, "Maximum thinking depth"),
                ],
            };
        }

        let is_haiku = name_lower.contains("haiku")
            || underlying_lower
                .as_ref()
                .is_some_and(|m| m.contains("haiku"));

        if is_haiku {
            // Claude Haiku is optimized for speed
            return ReasoningSupport {
                default_effort: Some(ReasoningEffort::Low),
                presets: vec![effort(ReasoningEffort::Low, "Fast responses")],
            };
        }

        // Claude Sonnet (default Claude model)
        return ReasoningSupport {
            default_effort: Some(ReasoningEffort::Medium),
            presets: vec![
                effort(ReasoningEffort::Low, "Quick responses"),
                effort(ReasoningEffort::Medium, "Balanced thinking"),
                effort(ReasoningEffort::High, "Extended thinking"),
            ],
        };
    }

    // Specialized support maps based on model capability signals.
    let is_gpt5_pro = name_lower.contains("gpt-5-pro")
        || underlying_lower
            .as_ref()
            .is_some_and(|m| m.contains("gpt-5-pro"));

    if is_gpt5_pro {
        return ReasoningSupport {
            default_effort: Some(ReasoningEffort::High),
            presets: vec![effort(
                ReasoningEffort::High,
                "High reasoning (maximum supported by GPT-5-Pro)",
            )],
        };
    }

    let has_xhigh = supports_xhigh(deployment_name) || underlying_model.is_some_and(supports_xhigh);

    let mut presets = vec![
        effort(ReasoningEffort::None, "No additional reasoning"),
        effort(ReasoningEffort::Low, "Quick responses"),
        effort(ReasoningEffort::Medium, "Balanced reasoning"),
        effort(ReasoningEffort::High, "Thorough reasoning"),
    ];

    if has_xhigh {
        presets.push(effort(
            ReasoningEffort::XHigh,
            "Extra high reasoning for complex problems",
        ));
    }

    ReasoningSupport {
        default_effort: Some(ReasoningEffort::Medium),
        presets,
    }
}

#[derive(Debug, Clone)]
struct ReasoningSupport {
    default_effort: Option<ReasoningEffort>,
    presets: Vec<ReasoningEffortPreset>,
}

fn effort(effort: ReasoningEffort, description: &str) -> ReasoningEffortPreset {
    ReasoningEffortPreset {
        effort,
        description: description.to_string(),
    }
}

/// Format a deployment name into a display name.
fn format_display_name(name: &str) -> String {
    // Handle Claude models specially
    if is_claude_model(name) {
        return format_claude_display_name(name);
    }

    // Convert deployment names like "gpt-4o" to "GPT-4o"
    // and "gpt-5.1-codex-max" to "GPT-5.1 Codex Max"
    let parts: Vec<&str> = name.split('-').collect();
    let formatted: Vec<String> = parts
        .iter()
        .enumerate()
        .map(|(i, part)| {
            if i == 0 && part.to_lowercase() == "gpt" {
                "GPT".to_string()
            } else if part.chars().all(|c| c.is_ascii_digit() || c == '.') {
                // Keep version numbers as-is
                part.to_string()
            } else {
                // Capitalize first letter of each word
                let mut chars: Vec<char> = part.chars().collect();
                if let Some(first) = chars.first_mut() {
                    *first = first.to_ascii_uppercase();
                }
                chars.into_iter().collect()
            }
        })
        .collect();

    formatted.join("-").replace("-", " ").replace("  ", " ")
}

/// Format Claude model names into display names.
/// Examples:
///   "claude-3-5-sonnet" → "Claude 3.5 Sonnet"
///   "claude-3-opus" → "Claude 3 Opus"
///   "claude-3-5-sonnet-20241022" → "Claude 3.5 Sonnet"
fn format_claude_display_name(name: &str) -> String {
    let parts: Vec<&str> = name.split('-').collect();
    let mut result_parts: Vec<String> = Vec::new();
    let mut i = 0;

    while i < parts.len() {
        let part = parts[i];
        let lower = part.to_lowercase();

        if lower == "claude" {
            result_parts.push("Claude".to_string());
        } else if part.chars().all(|c| c.is_ascii_digit()) {
            // Check if this is part of a compound version (e.g., "3-5" → "3.5")
            if i + 1 < parts.len()
                && parts[i + 1].chars().all(|c| c.is_ascii_digit())
                && parts[i + 1].len() == 1
            {
                result_parts.push(format!("{}.{}", part, parts[i + 1]));
                i += 1;
            } else if part.len() >= 8 {
                // Skip date-like suffixes (e.g., "20241022")
                break;
            } else {
                result_parts.push(part.to_string());
            }
        } else {
            // Capitalize model names like "sonnet", "opus", "haiku"
            let mut chars: Vec<char> = part.chars().collect();
            if let Some(first) = chars.first_mut() {
                *first = first.to_ascii_uppercase();
            }
            result_parts.push(chars.into_iter().collect());
        }
        i += 1;
    }

    result_parts.join(" ")
}

/// Create a shared deployments manager.
pub fn create_deployments_manager(endpoint: Option<String>) -> Arc<AzureDeploymentsManager> {
    Arc::new(AzureDeploymentsManager::new(endpoint))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_account_name() {
        // Azure AI Services (preferred format)
        assert_eq!(
            AzureDeploymentsManager::extract_account_name(
                "https://myresource.services.ai.azure.com"
            ),
            Some("myresource".to_string())
        );

        // Azure OpenAI (legacy format)
        assert_eq!(
            AzureDeploymentsManager::extract_account_name("https://myresource.openai.azure.com"),
            Some("myresource".to_string())
        );

        assert_eq!(
            AzureDeploymentsManager::extract_account_name(
                "https://arfre-m9hi1awe-eastus2.openai.azure.com/"
            ),
            Some("arfre-m9hi1awe-eastus2".to_string())
        );

        assert_eq!(
            AzureDeploymentsManager::extract_account_name("http://localhost:8080"),
            Some("localhost:8080".to_string())
        );
    }

    #[test]
    fn test_supports_xhigh() {
        // Models that should support xHigh
        assert!(supports_xhigh("gpt-5.1-codex-max"));
        assert!(supports_xhigh("GPT-5.1-CODEX-MAX"));
        assert!(supports_xhigh("gpt-5.2"));
        assert!(supports_xhigh("GPT-5.2"));
        assert!(supports_xhigh("gpt-5.2-preview"));

        // Models that should NOT support xHigh
        assert!(!supports_xhigh("gpt-5.1-codex"));
        assert!(!supports_xhigh("gpt-5.1-codex-mini"));
        assert!(!supports_xhigh("gpt-5.1"));
        assert!(!supports_xhigh("gpt-5"));
        assert!(!supports_xhigh("gpt-4o"));
        assert!(!supports_xhigh("gpt-4"));
    }

    #[test]
    fn test_reasoning_efforts_with_xhigh() {
        let efforts = get_reasoning_efforts_for_model("gpt-5.2", None);
        assert!(
            efforts
                .presets
                .iter()
                .any(|e| e.effort == ReasoningEffort::XHigh),
            "gpt-5.2 should have xHigh"
        );

        let efforts = get_reasoning_efforts_for_model("gpt-5.1-codex-max", None);
        assert!(
            efforts
                .presets
                .iter()
                .any(|e| e.effort == ReasoningEffort::XHigh),
            "gpt-5.1-codex-max should have xHigh"
        );
    }

    #[test]
    fn test_reasoning_efforts_without_xhigh() {
        let efforts = get_reasoning_efforts_for_model("gpt-5.1", None);
        assert!(
            !efforts
                .presets
                .iter()
                .any(|e| e.effort == ReasoningEffort::XHigh),
            "gpt-5.1 should NOT have xHigh"
        );

        let efforts = get_reasoning_efforts_for_model("gpt-4o", None);
        assert!(
            !efforts
                .presets
                .iter()
                .any(|e| e.effort == ReasoningEffort::XHigh),
            "gpt-4o should NOT have xHigh"
        );
    }

    #[test]
    fn test_reasoning_efforts_from_underlying_model() {
        // Deployment name doesn't indicate xHigh, but underlying model does
        let efforts = get_reasoning_efforts_for_model("my-custom-deployment", Some("gpt-5.2"));
        assert!(
            efforts
                .presets
                .iter()
                .any(|e| e.effort == ReasoningEffort::XHigh),
            "Should have xHigh based on underlying model gpt-5.2"
        );

        // Neither deployment nor underlying model supports xHigh
        let efforts = get_reasoning_efforts_for_model("my-custom-deployment", Some("gpt-4o"));
        assert!(
            !efforts
                .presets
                .iter()
                .any(|e| e.effort == ReasoningEffort::XHigh),
            "Should NOT have xHigh for gpt-4o"
        );
    }

    #[test]
    fn test_reasoning_efforts_gpt5_pro_high_only() {
        let efforts = get_reasoning_efforts_for_model("gpt-5-pro", None);
        assert_eq!(
            efforts.default_effort,
            Some(ReasoningEffort::High),
            "gpt-5-pro default should be High"
        );
        assert_eq!(
            efforts.presets,
            vec![ReasoningEffortPreset {
                effort: ReasoningEffort::High,
                description: "High reasoning (maximum supported by GPT-5-Pro)".to_string()
            }],
            "gpt-5-pro should only surface High"
        );
    }

    // Claude model tests

    #[test]
    fn test_is_claude_model() {
        assert!(is_claude_model("claude-3-5-sonnet"));
        assert!(is_claude_model("Claude-3-Opus"));
        assert!(is_claude_model("claude-3-haiku"));
        assert!(is_claude_model("my-claude-deployment"));

        assert!(!is_claude_model("gpt-4o"));
        assert!(!is_claude_model("gpt-5.1-codex"));
    }

    #[test]
    fn test_is_supported_model_claude() {
        // Claude models should be supported
        assert!(is_supported_model("claude-3-5-sonnet", None));
        assert!(is_supported_model("claude-3-opus", None));
        assert!(is_supported_model("claude-3-haiku", None));

        // Custom deployment with Claude underlying model
        assert!(is_supported_model(
            "my-deployment",
            Some("claude-3-5-sonnet")
        ));

        // Non-supported models
        assert!(!is_supported_model("llama-3", None));
        assert!(!is_supported_model("mistral-7b", None));
    }

    #[test]
    fn test_format_claude_display_name() {
        assert_eq!(
            format_claude_display_name("claude-3-5-sonnet"),
            "Claude 3.5 Sonnet"
        );
        assert_eq!(format_claude_display_name("claude-3-opus"), "Claude 3 Opus");
        assert_eq!(
            format_claude_display_name("claude-3-haiku"),
            "Claude 3 Haiku"
        );
        assert_eq!(
            format_claude_display_name("claude-3-5-sonnet-20241022"),
            "Claude 3.5 Sonnet"
        );
        assert_eq!(
            format_claude_display_name("Claude-3-5-Sonnet"),
            "Claude 3.5 Sonnet"
        );
    }

    #[test]
    fn test_format_display_name_routes_claude() {
        // Claude models should go through format_claude_display_name
        assert_eq!(
            format_display_name("claude-3-5-sonnet"),
            "Claude 3.5 Sonnet"
        );

        // GPT models should use the existing formatter
        assert_eq!(format_display_name("gpt-4o"), "GPT 4o");
    }

    #[test]
    fn test_reasoning_efforts_claude_opus() {
        let efforts = get_reasoning_efforts_for_model("claude-3-opus", None);
        assert_eq!(
            efforts.default_effort,
            Some(ReasoningEffort::High),
            "Claude Opus default should be High"
        );
        assert!(
            efforts
                .presets
                .iter()
                .any(|e| e.effort == ReasoningEffort::XHigh),
            "Claude Opus should have XHigh"
        );
    }

    #[test]
    fn test_reasoning_efforts_claude_sonnet() {
        let efforts = get_reasoning_efforts_for_model("claude-3-5-sonnet", None);
        assert_eq!(
            efforts.default_effort,
            Some(ReasoningEffort::Medium),
            "Claude Sonnet default should be Medium"
        );
        assert!(
            efforts
                .presets
                .iter()
                .any(|e| e.effort == ReasoningEffort::High),
            "Claude Sonnet should have High"
        );
        assert!(
            !efforts
                .presets
                .iter()
                .any(|e| e.effort == ReasoningEffort::XHigh),
            "Claude Sonnet should NOT have XHigh"
        );
    }

    #[test]
    fn test_reasoning_efforts_claude_haiku() {
        let efforts = get_reasoning_efforts_for_model("claude-3-haiku", None);
        assert_eq!(
            efforts.default_effort,
            Some(ReasoningEffort::Low),
            "Claude Haiku default should be Low"
        );
        assert_eq!(
            efforts.presets.len(),
            1,
            "Claude Haiku should only have one effort level"
        );
    }

    #[test]
    fn test_reasoning_efforts_claude_from_underlying() {
        // Custom deployment name but Claude underlying model
        let efforts = get_reasoning_efforts_for_model("my-ai-deployment", Some("claude-3-opus"));
        assert_eq!(
            efforts.default_effort,
            Some(ReasoningEffort::High),
            "Should detect Claude Opus from underlying model"
        );
    }
}
