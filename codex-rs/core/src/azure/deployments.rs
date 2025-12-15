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

    /// The Azure OpenAI endpoint (e.g., "https://myresource.openai.azure.com").
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

    /// Extract the account name from an Azure OpenAI endpoint URL.
    fn extract_account_name(endpoint: &str) -> Option<String> {
        // Endpoint format: https://{account-name}.openai.azure.com
        // or https://{account-name}-{region}.openai.azure.com
        let url = endpoint.trim_end_matches('/');
        let host = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"))?;

        // Get the subdomain (everything before .openai.azure.com)
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
            .filter(|d| {
                let name_lower = d.name.to_lowercase();
                name_lower.starts_with("gpt")
            })
            .collect()
    }

    /// Check if we have a valid Azure endpoint configured.
    pub fn has_endpoint(&self) -> bool {
        self.endpoint.is_some()
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

        ModelPreset {
            id: self.name.clone(),
            model: self.name.clone(),
            display_name,
            description,
            default_reasoning_effort: ReasoningEffort::Medium,
            supported_reasoning_efforts,
            is_default: false,
            upgrade: None,
            show_in_picker: true,
        }
    }
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
) -> Vec<ReasoningEffortPreset> {
    let has_xhigh =
        supports_xhigh(deployment_name) || underlying_model.is_some_and(supports_xhigh);

    let mut efforts = vec![
        ReasoningEffortPreset {
            effort: ReasoningEffort::None,
            description: "No additional reasoning".to_string(),
        },
        ReasoningEffortPreset {
            effort: ReasoningEffort::Low,
            description: "Quick responses".to_string(),
        },
        ReasoningEffortPreset {
            effort: ReasoningEffort::Medium,
            description: "Balanced reasoning".to_string(),
        },
        ReasoningEffortPreset {
            effort: ReasoningEffort::High,
            description: "Thorough reasoning".to_string(),
        },
    ];

    if has_xhigh {
        efforts.push(ReasoningEffortPreset {
            effort: ReasoningEffort::XHigh,
            description: "Extra high reasoning for complex problems".to_string(),
        });
    }

    efforts
}

/// Format a deployment name into a display name.
fn format_display_name(name: &str) -> String {
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

/// Create a shared deployments manager.
pub fn create_deployments_manager(endpoint: Option<String>) -> Arc<AzureDeploymentsManager> {
    Arc::new(AzureDeploymentsManager::new(endpoint))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_account_name() {
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
            efforts.iter().any(|e| e.effort == ReasoningEffort::XHigh),
            "gpt-5.2 should have xHigh"
        );

        let efforts = get_reasoning_efforts_for_model("gpt-5.1-codex-max", None);
        assert!(
            efforts.iter().any(|e| e.effort == ReasoningEffort::XHigh),
            "gpt-5.1-codex-max should have xHigh"
        );
    }

    #[test]
    fn test_reasoning_efforts_without_xhigh() {
        let efforts = get_reasoning_efforts_for_model("gpt-5.1", None);
        assert!(
            !efforts.iter().any(|e| e.effort == ReasoningEffort::XHigh),
            "gpt-5.1 should NOT have xHigh"
        );

        let efforts = get_reasoning_efforts_for_model("gpt-4o", None);
        assert!(
            !efforts.iter().any(|e| e.effort == ReasoningEffort::XHigh),
            "gpt-4o should NOT have xHigh"
        );
    }

    #[test]
    fn test_reasoning_efforts_from_underlying_model() {
        // Deployment name doesn't indicate xHigh, but underlying model does
        let efforts = get_reasoning_efforts_for_model("my-custom-deployment", Some("gpt-5.2"));
        assert!(
            efforts.iter().any(|e| e.effort == ReasoningEffort::XHigh),
            "Should have xHigh based on underlying model gpt-5.2"
        );

        // Neither deployment nor underlying model supports xHigh
        let efforts = get_reasoning_efforts_for_model("my-custom-deployment", Some("gpt-4o"));
        assert!(
            !efforts.iter().any(|e| e.effort == ReasoningEffort::XHigh),
            "Should NOT have xHigh for gpt-4o"
        );
    }
}
