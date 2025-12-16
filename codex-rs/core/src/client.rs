use std::sync::Arc;

use crate::api_bridge::auth_provider_from_auth;
use crate::api_bridge::map_api_error;
use crate::auth::azure::AzureAuth;
use codex_api::AggregateStreamExt;
use codex_api::ChatClient as ApiChatClient;
use codex_api::CompactClient as ApiCompactClient;
use codex_api::CompactionInput as ApiCompactionInput;
use codex_api::Prompt as ApiPrompt;
use codex_api::RequestTelemetry;
use codex_api::ReqwestTransport;
use codex_api::ResponseStream as ApiResponseStream;
use codex_api::ResponsesClient as ApiResponsesClient;
use codex_api::ResponsesOptions as ApiResponsesOptions;
use codex_api::SseTelemetry;
use codex_api::TransportError;
use codex_api::common::Reasoning;
use codex_api::create_text_param_for_request;
use codex_api::error::ApiError;
use codex_app_server_protocol::AuthMode;
use codex_otel::otel_manager::OtelManager;
use codex_protocol::ConversationId;
use codex_protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use codex_protocol::protocol::SessionSource;
use eventsource_stream::Event;
use eventsource_stream::EventStreamError;
use futures::StreamExt;
use http::HeaderMap as ApiHeaderMap;
use http::HeaderValue;
use http::StatusCode as HttpStatusCode;
use reqwest::StatusCode;
use serde_json::Value;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::warn;

use crate::AuthManager;
use crate::auth::RefreshTokenError;
use crate::client_common::Prompt;
use crate::client_common::ResponseEvent;
use crate::client_common::ResponseStream;
use crate::config::Config;
use crate::default_client::build_reqwest_client;
use crate::error::CodexErr;
use crate::error::Result;
use crate::flags::CODEX_RS_SSE_FIXTURE;
use crate::model_provider_info::ModelProviderInfo;
use crate::model_provider_info::WireApi;
use crate::openai_models::model_family::ModelFamily;
use crate::tools::spec::create_tools_json_for_chat_completions_api;
use crate::tools::spec::create_tools_json_for_responses_api;

#[derive(Debug, Clone)]
pub struct ModelClient {
    config: Arc<Config>,
    auth_manager: Option<Arc<AuthManager>>,
    azure_auth: Option<Arc<AzureAuth>>,
    model_family: ModelFamily,
    otel_manager: OtelManager,
    provider: ModelProviderInfo,
    conversation_id: ConversationId,
    effort: Option<ReasoningEffortConfig>,
    summary: ReasoningSummaryConfig,
    session_source: SessionSource,
    /// Optional override for wire API based on model capabilities.
    /// When set, this takes precedence over provider.wire_api.
    wire_api_override: Option<WireApi>,
}

#[allow(clippy::too_many_arguments)]
impl ModelClient {
    pub fn new(
        config: Arc<Config>,
        auth_manager: Option<Arc<AuthManager>>,
        azure_auth: Option<Arc<AzureAuth>>,
        model_family: ModelFamily,
        otel_manager: OtelManager,
        provider: ModelProviderInfo,
        effort: Option<ReasoningEffortConfig>,
        summary: ReasoningSummaryConfig,
        conversation_id: ConversationId,
        session_source: SessionSource,
    ) -> Self {
        // Determine wire API and effective provider based on model capabilities
        let (wire_api_override, effective_provider) = if provider.is_azure_endpoint() {
            let model_slug = model_family.get_model_slug();
            let effective_wire_api = determine_wire_api_for_azure_model(model_slug);

            // If the effective wire API differs from the provider's default,
            // we need to adjust the provider's base_url for Azure
            let adjusted_provider = if effective_wire_api != provider.wire_api {
                adjust_azure_provider_for_wire_api(&provider, effective_wire_api, model_slug)
            } else {
                provider
            };

            (Some(effective_wire_api), adjusted_provider)
        } else {
            (None, provider)
        };

        Self {
            config,
            auth_manager,
            azure_auth,
            model_family,
            otel_manager,
            provider: effective_provider,
            conversation_id,
            effort,
            summary,
            session_source,
            wire_api_override,
        }
    }

    /// Returns the effective wire API for this client.
    /// Uses the override if set, otherwise falls back to provider default.
    fn effective_wire_api(&self) -> WireApi {
        self.wire_api_override.unwrap_or(self.provider.wire_api)
    }

    pub fn get_model_context_window(&self) -> Option<i64> {
        let model_family = self.get_model_family();
        let effective_context_window_percent = model_family.effective_context_window_percent;
        model_family
            .context_window
            .map(|w| w.saturating_mul(effective_context_window_percent) / 100)
    }

    pub fn config(&self) -> Arc<Config> {
        Arc::clone(&self.config)
    }

    pub fn provider(&self) -> &ModelProviderInfo {
        &self.provider
    }

    /// Streams a single model turn using either the Responses or Chat
    /// Completions wire API, depending on the model capabilities and provider.
    ///
    /// For Azure endpoints, the wire API is determined dynamically based on
    /// the model's capabilities (some models only support Chat Completions,
    /// others only support Responses API).
    ///
    /// For Chat providers, the underlying stream is optionally aggregated
    /// based on the `show_raw_agent_reasoning` flag in the config.
    pub async fn stream(&self, prompt: &Prompt) -> Result<ResponseStream> {
        let wire_api = self.effective_wire_api();
        tracing::debug!(
            model = %self.model_family.slug,
            wire_api = ?wire_api,
            "Streaming with wire API"
        );

        match wire_api {
            WireApi::Responses => self.stream_responses_api(prompt).await,
            WireApi::Chat => {
                let api_stream = self.stream_chat_completions(prompt).await?;

                if self.config.show_raw_agent_reasoning {
                    Ok(map_response_stream(
                        api_stream.streaming_mode(),
                        self.otel_manager.clone(),
                    ))
                } else {
                    Ok(map_response_stream(
                        api_stream.aggregate(),
                        self.otel_manager.clone(),
                    ))
                }
            }
        }
    }

    /// Streams a turn via the OpenAI Chat Completions API.
    ///
    /// This path is only used when the provider is configured with
    /// `WireApi::Chat`; it does not support `output_schema` today.
    async fn stream_chat_completions(&self, prompt: &Prompt) -> Result<ApiResponseStream> {
        if prompt.output_schema.is_some() {
            return Err(CodexErr::UnsupportedOperation(
                "output_schema is not supported for Chat Completions API".to_string(),
            ));
        }

        let auth_manager = self.auth_manager.clone();
        let model_family = self.get_model_family();
        let instructions = prompt.get_full_instructions(&model_family).into_owned();
        let tools_json = create_tools_json_for_chat_completions_api(&prompt.tools)?;
        let api_prompt = build_api_prompt(prompt, instructions, tools_json);
        let conversation_id = self.conversation_id.to_string();
        let session_source = self.session_source.clone();

        let mut refreshed = false;
        let model = self.get_model();
        loop {
            let auth = auth_manager.as_ref().and_then(|m| m.auth());
            let api_provider = self
                .provider
                .to_api_provider_with_model(auth.as_ref().map(|a| a.mode), Some(&model))?;
            let api_auth = auth_provider_from_auth(
                self.azure_auth.as_ref().map(AsRef::as_ref),
                &self.provider,
                auth.as_ref(),
            )
            .await?;
            let transport = ReqwestTransport::new(build_reqwest_client());
            let (request_telemetry, sse_telemetry) = self.build_streaming_telemetry();
            let client = ApiChatClient::new(transport, api_provider, api_auth)
                .with_telemetry(Some(request_telemetry), Some(sse_telemetry));

            let stream_result = client
                .stream_prompt(
                    &model,
                    &api_prompt,
                    Some(conversation_id.clone()),
                    Some(session_source.clone()),
                )
                .await;

            match stream_result {
                Ok(stream) => return Ok(stream),
                Err(ApiError::Transport(TransportError::Http { status, .. }))
                    if status == StatusCode::UNAUTHORIZED =>
                {
                    handle_unauthorized(
                        status,
                        &mut refreshed,
                        &auth_manager,
                        &self.azure_auth,
                        &auth,
                    )
                    .await?;
                    continue;
                }
                Err(err) => return Err(map_api_error(err)),
            }
        }
    }

    /// Streams a turn via the OpenAI Responses API.
    ///
    /// Handles SSE fixtures, reasoning summaries, verbosity, and the
    /// `text` controls used for output schemas.
    async fn stream_responses_api(&self, prompt: &Prompt) -> Result<ResponseStream> {
        if let Some(path) = &*CODEX_RS_SSE_FIXTURE {
            warn!(path, "Streaming from fixture");
            let stream = codex_api::stream_from_fixture(path, self.provider.stream_idle_timeout())
                .map_err(map_api_error)?;
            return Ok(map_response_stream(stream, self.otel_manager.clone()));
        }

        let auth_manager = self.auth_manager.clone();
        let model_family = self.get_model_family();
        let instructions = prompt.get_full_instructions(&model_family).into_owned();
        let tools_json: Vec<Value> = create_tools_json_for_responses_api(&prompt.tools)?;

        let reasoning = if model_family.supports_reasoning_summaries {
            Some(Reasoning {
                effort: self.effort.or(model_family.default_reasoning_effort),
                summary: if self.summary == ReasoningSummaryConfig::None {
                    None
                } else {
                    Some(self.summary)
                },
            })
        } else {
            None
        };

        let include: Vec<String> = if reasoning.is_some() {
            vec!["reasoning.encrypted_content".to_string()]
        } else {
            vec![]
        };

        let verbosity = if model_family.support_verbosity {
            self.config
                .model_verbosity
                .or(model_family.default_verbosity)
        } else {
            if self.config.model_verbosity.is_some() {
                warn!(
                    "model_verbosity is set but ignored as the model does not support verbosity: {}",
                    model_family.family
                );
            }
            None
        };

        let text = create_text_param_for_request(verbosity, &prompt.output_schema);
        let api_prompt = build_api_prompt(prompt, instructions.clone(), tools_json);
        let conversation_id = self.conversation_id.to_string();
        let session_source = self.session_source.clone();

        let mut refreshed = false;
        let model = self.get_model();
        loop {
            let auth = auth_manager.as_ref().and_then(|m| m.auth());
            let api_provider = self
                .provider
                .to_api_provider_with_model(auth.as_ref().map(|a| a.mode), Some(&model))?;
            let api_auth = auth_provider_from_auth(
                self.azure_auth.as_ref().map(AsRef::as_ref),
                &self.provider,
                auth.as_ref(),
            )
            .await?;
            let transport = ReqwestTransport::new(build_reqwest_client());
            let (request_telemetry, sse_telemetry) = self.build_streaming_telemetry();
            let client = ApiResponsesClient::new(transport, api_provider, api_auth)
                .with_telemetry(Some(request_telemetry), Some(sse_telemetry));

            let options = ApiResponsesOptions {
                reasoning: reasoning.clone(),
                include: include.clone(),
                prompt_cache_key: Some(conversation_id.clone()),
                text: text.clone(),
                store_override: None,
                conversation_id: Some(conversation_id.clone()),
                session_source: Some(session_source.clone()),
            };

            let stream_result = client.stream_prompt(&model, &api_prompt, options).await;

            match stream_result {
                Ok(stream) => {
                    return Ok(map_response_stream(stream, self.otel_manager.clone()));
                }
                Err(ApiError::Transport(TransportError::Http { status, .. }))
                    if status == StatusCode::UNAUTHORIZED =>
                {
                    handle_unauthorized(
                        status,
                        &mut refreshed,
                        &auth_manager,
                        &self.azure_auth,
                        &auth,
                    )
                    .await?;
                    continue;
                }
                Err(err) => return Err(map_api_error(err)),
            }
        }
    }

    pub fn get_provider(&self) -> ModelProviderInfo {
        self.provider.clone()
    }

    pub fn get_otel_manager(&self) -> OtelManager {
        self.otel_manager.clone()
    }

    pub fn get_session_source(&self) -> SessionSource {
        self.session_source.clone()
    }

    /// Returns the currently configured model slug.
    pub fn get_model(&self) -> String {
        self.get_model_family().get_model_slug().to_string()
    }

    /// Returns the currently configured model family.
    pub fn get_model_family(&self) -> ModelFamily {
        self.model_family.clone()
    }

    /// Returns the current reasoning effort setting.
    pub fn get_reasoning_effort(&self) -> Option<ReasoningEffortConfig> {
        self.effort
    }

    /// Returns the current reasoning summary setting.
    pub fn get_reasoning_summary(&self) -> ReasoningSummaryConfig {
        self.summary
    }

    pub fn get_auth_manager(&self) -> Option<Arc<AuthManager>> {
        self.auth_manager.clone()
    }

    /// Compacts the current conversation history using the Compact endpoint.
    ///
    /// This is a unary call (no streaming) that returns a new list of
    /// `ResponseItem`s representing the compacted transcript.
    pub async fn compact_conversation_history(&self, prompt: &Prompt) -> Result<Vec<ResponseItem>> {
        if prompt.input.is_empty() {
            return Ok(Vec::new());
        }
        let auth_manager = self.auth_manager.clone();
        let auth = auth_manager.as_ref().and_then(|m| m.auth());
        let model = self.get_model();
        let api_provider = self
            .provider
            .to_api_provider_with_model(auth.as_ref().map(|a| a.mode), Some(&model))?;
        let api_auth = auth_provider_from_auth(
            self.azure_auth.as_ref().map(AsRef::as_ref),
            &self.provider,
            auth.as_ref(),
        )
        .await?;
        let transport = ReqwestTransport::new(build_reqwest_client());
        let request_telemetry = self.build_request_telemetry();
        let client = ApiCompactClient::new(transport, api_provider, api_auth)
            .with_telemetry(Some(request_telemetry));

        let instructions = prompt
            .get_full_instructions(&self.get_model_family())
            .into_owned();
        let payload = ApiCompactionInput {
            model: &model,
            input: &prompt.input,
            instructions: &instructions,
        };

        let mut extra_headers = ApiHeaderMap::new();
        if let SessionSource::SubAgent(sub) = &self.session_source {
            let subagent = if let crate::protocol::SubAgentSource::Other(label) = sub {
                label.clone()
            } else {
                serde_json::to_value(sub)
                    .ok()
                    .and_then(|v| v.as_str().map(std::string::ToString::to_string))
                    .unwrap_or_else(|| "other".to_string())
            };
            if let Ok(val) = HeaderValue::from_str(&subagent) {
                extra_headers.insert("x-openai-subagent", val);
            }
        }

        client
            .compact_input(&payload, extra_headers)
            .await
            .map_err(map_api_error)
    }
}

impl ModelClient {
    /// Builds request and SSE telemetry for streaming API calls (Chat/Responses).
    fn build_streaming_telemetry(&self) -> (Arc<dyn RequestTelemetry>, Arc<dyn SseTelemetry>) {
        let telemetry = Arc::new(ApiTelemetry::new(self.otel_manager.clone()));
        let request_telemetry: Arc<dyn RequestTelemetry> = telemetry.clone();
        let sse_telemetry: Arc<dyn SseTelemetry> = telemetry;
        (request_telemetry, sse_telemetry)
    }

    /// Builds request telemetry for unary API calls (e.g., Compact endpoint).
    fn build_request_telemetry(&self) -> Arc<dyn RequestTelemetry> {
        let telemetry = Arc::new(ApiTelemetry::new(self.otel_manager.clone()));
        let request_telemetry: Arc<dyn RequestTelemetry> = telemetry;
        request_telemetry
    }
}

/// Adapts the core `Prompt` type into the `codex-api` payload shape.
fn build_api_prompt(prompt: &Prompt, instructions: String, tools_json: Vec<Value>) -> ApiPrompt {
    ApiPrompt {
        instructions,
        input: prompt.get_formatted_input(),
        tools: tools_json,
        parallel_tool_calls: prompt.parallel_tool_calls,
        output_schema: prompt.output_schema.clone(),
    }
}

fn map_response_stream<S>(api_stream: S, otel_manager: OtelManager) -> ResponseStream
where
    S: futures::Stream<Item = std::result::Result<ResponseEvent, ApiError>>
        + Unpin
        + Send
        + 'static,
{
    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent>>(1600);

    tokio::spawn(async move {
        let mut logged_error = false;
        let mut api_stream = api_stream;
        while let Some(event) = api_stream.next().await {
            match event {
                Ok(ResponseEvent::Completed {
                    response_id,
                    token_usage,
                }) => {
                    if let Some(usage) = &token_usage {
                        otel_manager.sse_event_completed(
                            usage.input_tokens,
                            usage.output_tokens,
                            Some(usage.cached_input_tokens),
                            Some(usage.reasoning_output_tokens),
                            usage.total_tokens,
                        );
                    }
                    if tx_event
                        .send(Ok(ResponseEvent::Completed {
                            response_id,
                            token_usage,
                        }))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                Ok(event) => {
                    if tx_event.send(Ok(event)).await.is_err() {
                        return;
                    }
                }
                Err(err) => {
                    let mapped = map_api_error(err);
                    if !logged_error {
                        otel_manager.see_event_completed_failed(&mapped);
                        logged_error = true;
                    }
                    if tx_event.send(Err(mapped)).await.is_err() {
                        return;
                    }
                }
            }
        }
    });

    ResponseStream { rx_event }
}

/// Handles a 401 response by optionally refreshing tokens once.
///
/// Supports both ChatGPT token refresh (via AuthManager) and Azure Entra ID
/// token refresh (via AzureAuth). When refresh succeeds, the caller should
/// retry the API call; otherwise the mapped `CodexErr` is returned.
async fn handle_unauthorized(
    status: StatusCode,
    refreshed: &mut bool,
    auth_manager: &Option<Arc<AuthManager>>,
    azure_auth: &Option<Arc<AzureAuth>>,
    auth: &Option<crate::auth::CodexAuth>,
) -> Result<()> {
    if *refreshed {
        return Err(map_unauthorized_status(status));
    }

    // Try Azure token refresh first if Azure auth is configured
    if let Some(azure) = azure_auth.as_ref() {
        // Clear cached token and retry - AzureAuth will fetch a fresh token
        azure.clear_cached_token().await;
        *refreshed = true;
        tracing::debug!("Cleared Azure token cache after 401, will retry with fresh token");
        return Ok(());
    }

    // Fall back to ChatGPT token refresh if using OpenAI auth
    if let Some(manager) = auth_manager.as_ref()
        && let Some(auth) = auth.as_ref()
        && auth.mode == AuthMode::ChatGPT
    {
        match manager.refresh_token().await {
            Ok(_) => {
                *refreshed = true;
                Ok(())
            }
            Err(RefreshTokenError::Permanent(failed)) => Err(CodexErr::RefreshTokenFailed(failed)),
            Err(RefreshTokenError::Transient(other)) => Err(CodexErr::Io(other)),
        }
    } else {
        Err(map_unauthorized_status(status))
    }
}

fn map_unauthorized_status(status: StatusCode) -> CodexErr {
    map_api_error(ApiError::Transport(TransportError::Http {
        status,
        headers: None,
        body: None,
    }))
}

struct ApiTelemetry {
    otel_manager: OtelManager,
}

impl ApiTelemetry {
    fn new(otel_manager: OtelManager) -> Self {
        Self { otel_manager }
    }
}

impl RequestTelemetry for ApiTelemetry {
    fn on_request(
        &self,
        attempt: u64,
        status: Option<HttpStatusCode>,
        error: Option<&TransportError>,
        duration: Duration,
    ) {
        let error_message = error.map(std::string::ToString::to_string);
        self.otel_manager.record_api_request(
            attempt,
            status.map(|s| s.as_u16()),
            error_message.as_deref(),
            duration,
        );
    }
}

impl SseTelemetry for ApiTelemetry {
    fn on_sse_poll(
        &self,
        result: &std::result::Result<
            Option<std::result::Result<Event, EventStreamError<TransportError>>>,
            tokio::time::error::Elapsed,
        >,
        duration: Duration,
    ) {
        self.otel_manager.log_sse_event(result, duration);
    }
}

/// Adjusts an Azure provider's configuration to use a different wire API.
///
/// Azure uses different URL formats for different APIs:
/// - Responses API: {endpoint}/openai/responses (model in body)
/// - Chat Completions API: {endpoint}/openai/deployments/{model}/chat/completions
///
/// This function creates a new provider with the correct base_url and wire_api
/// for the target wire API.
fn adjust_azure_provider_for_wire_api(
    provider: &ModelProviderInfo,
    target_wire_api: WireApi,
    model_name: &str,
) -> ModelProviderInfo {
    let mut adjusted = provider.clone();
    adjusted.wire_api = target_wire_api;

    // Adjust base_url based on target wire API
    if let Some(base_url) = &provider.base_url {
        adjusted.base_url = Some(match target_wire_api {
            WireApi::Responses => {
                // For Responses API: {endpoint}/openai
                // Strip /deployments if present
                if base_url.ends_with("/openai/deployments") {
                    base_url.trim_end_matches("/deployments").to_string()
                } else {
                    base_url.clone()
                }
            }
            WireApi::Chat => {
                // For Chat API: {endpoint}/openai/deployments/{model}
                // Add /deployments/{model} if not present
                if base_url.ends_with("/openai") {
                    format!("{base_url}/deployments/{model_name}")
                } else if base_url.ends_with("/openai/deployments") {
                    format!("{base_url}/{model_name}")
                } else {
                    base_url.clone()
                }
            }
        });
    }

    tracing::debug!(
        model = %model_name,
        original_wire_api = ?provider.wire_api,
        target_wire_api = ?target_wire_api,
        original_base_url = ?provider.base_url,
        adjusted_base_url = ?adjusted.base_url,
        "Adjusted Azure provider for wire API"
    );

    adjusted
}

/// Determines the appropriate wire API for an Azure deployment based on model name.
///
/// Azure OpenAI deployments have varying capabilities:
/// - Some models (like claude-*, grok-*) only support Chat Completions API
/// - Some models (like gpt-5.1-codex, gpt-5-codex) only support Responses API
/// - Most GPT models support both, and we prefer Responses for reasoning features
///
/// This function uses pattern matching on model names to determine the correct API.
fn determine_wire_api_for_azure_model(model_name: &str) -> WireApi {
    let name_lower = model_name.to_lowercase();

    // Models that ONLY support Chat Completions (no Responses API)
    // Claude and Grok models on Azure only support chatCompletion
    if name_lower.starts_with("claude-") || name_lower.starts_with("grok-") {
        tracing::debug!(
            model = %model_name,
            "Model only supports Chat Completions API"
        );
        return WireApi::Chat;
    }

    // All GPT models on Azure now support Responses API
    // This includes: gpt-5.2, gpt-5.1, gpt-5, gpt-5.1-codex, gpt-5.1-codex-max,
    // gpt-5-codex, gpt-5-pro, gpt-4o, gpt-4.1, o3-mini, o4-mini, etc.
    //
    // For codex models (gpt-5.1-codex, gpt-5-codex, gpt-5.1-codex-max),
    // chatCompletion is explicitly disabled, so Responses is the only option.
    //
    // For other models with both APIs, we prefer Responses because it enables
    // advanced features like reasoning_effort and verbosity.
    if name_lower.starts_with("gpt-")
        || name_lower.starts_with("o3-")
        || name_lower.starts_with("o4-")
    {
        tracing::debug!(
            model = %model_name,
            "Using Responses API for GPT/reasoning model"
        );
        return WireApi::Responses;
    }

    // Default to Responses for unknown models (backwards compatibility)
    tracing::debug!(
        model = %model_name,
        "Unknown model, defaulting to Responses API"
    );
    WireApi::Responses
}
