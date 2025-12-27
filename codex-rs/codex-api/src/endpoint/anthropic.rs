//! Anthropic Messages API client.
//!
//! This module provides a client for the Anthropic Messages API,
//! which is used for Claude models on Azure AI Services.

use crate::auth::AuthProvider;
use crate::common::Prompt as ApiPrompt;
use crate::common::ResponseStream;
use crate::endpoint::streaming::StreamingClient;
use crate::error::ApiError;
use crate::provider::Provider;
use crate::provider::WireApi;
use crate::requests::AnthropicRequest;
use crate::sse::anthropic::spawn_anthropic_stream;
use crate::telemetry::SseTelemetry;
use codex_client::HttpTransport;
use codex_client::RequestTelemetry;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::SessionSource;
use http::HeaderMap;
use serde_json::Value;
use std::sync::Arc;

/// Options for Anthropic Messages API requests.
#[derive(Default)]
pub struct AnthropicOptions {
    /// Reasoning effort for extended thinking.
    /// Maps to Anthropic's thinking.budget_tokens parameter.
    pub reasoning_effort: Option<ReasoningEffort>,
    /// Conversation ID for request correlation.
    pub conversation_id: Option<String>,
    /// Session source for telemetry.
    pub session_source: Option<SessionSource>,
}

pub struct AnthropicClient<T: HttpTransport, A: AuthProvider> {
    streaming: StreamingClient<T, A>,
}

impl<T: HttpTransport, A: AuthProvider> AnthropicClient<T, A> {
    pub fn new(transport: T, provider: Provider, auth: A) -> Self {
        Self {
            streaming: StreamingClient::new(transport, provider, auth),
        }
    }

    pub fn with_telemetry(
        self,
        request: Option<Arc<dyn RequestTelemetry>>,
        sse: Option<Arc<dyn SseTelemetry>>,
    ) -> Self {
        Self {
            streaming: self.streaming.with_telemetry(request, sse),
        }
    }

    pub async fn stream_request(
        &self,
        request: AnthropicRequest,
    ) -> Result<ResponseStream, ApiError> {
        self.stream(request.body, request.headers).await
    }

    pub async fn stream_prompt(
        &self,
        model: &str,
        prompt: &ApiPrompt,
        options: AnthropicOptions,
    ) -> Result<ResponseStream, ApiError> {
        use crate::requests::AnthropicRequestBuilder;

        let AnthropicOptions {
            reasoning_effort,
            conversation_id,
            session_source,
        } = options;

        let request =
            AnthropicRequestBuilder::new(model, &prompt.instructions, &prompt.input, &prompt.tools)
                .conversation_id(conversation_id)
                .session_source(session_source)
                .reasoning_effort(reasoning_effort)
                .build(self.streaming.provider())?;

        self.stream_request(request).await
    }

    fn path(&self) -> &'static str {
        match self.streaming.provider().wire {
            WireApi::Anthropic => "messages",
            _ => "messages", // Default for Anthropic API
        }
    }

    pub async fn stream(
        &self,
        body: Value,
        extra_headers: HeaderMap,
    ) -> Result<ResponseStream, ApiError> {
        self.streaming
            .stream(self.path(), body, extra_headers, spawn_anthropic_stream)
            .await
    }
}
