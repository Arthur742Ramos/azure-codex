//! Anthropic Messages API request builder.
//!
//! This module handles building requests for the Anthropic Messages API,
//! which is used for Claude models on Azure AI Services.

use crate::error::ApiError;
use crate::provider::Provider;
use crate::requests::headers::build_conversation_headers;
use crate::requests::headers::insert_header;
use crate::requests::headers::subagent_header;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::ReasoningItemContent;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::SessionSource;
use http::HeaderMap;
use serde_json::Value;
use serde_json::json;
use tracing::debug;

/// Default max tokens for Anthropic API (without thinking).
const DEFAULT_MAX_TOKENS: i64 = 8192;

/// Buffer for response tokens when thinking is enabled.
/// max_tokens = budget_tokens + this buffer
const RESPONSE_TOKEN_BUFFER: i64 = 16000;

/// Maps reasoning effort to Anthropic thinking budget tokens.
/// Returns None if thinking should be disabled.
/// Note: max_tokens must be > budget_tokens, and Claude Opus 4.5 has a 64k output limit.
/// With RESPONSE_TOKEN_BUFFER of 16k, max safe budget is ~48k.
fn effort_to_budget_tokens(effort: ReasoningEffort) -> Option<i64> {
    match effort {
        ReasoningEffort::None => None,           // Disable thinking
        ReasoningEffort::Minimal => Some(4_000), // Anthropic recommended minimum
        ReasoningEffort::Low => Some(8_000),
        ReasoningEffort::Medium => Some(16_000),
        ReasoningEffort::High => Some(32_000),
        ReasoningEffort::XHigh => Some(48_000), // Max safe with 16k buffer under 64k limit
    }
}

/// Assembled request body plus headers for Anthropic Messages API streaming calls.
pub struct AnthropicRequest {
    pub body: Value,
    pub headers: HeaderMap,
}

pub struct AnthropicRequestBuilder<'a> {
    model: &'a str,
    instructions: &'a str,
    input: &'a [ResponseItem],
    tools: &'a [Value],
    conversation_id: Option<String>,
    session_source: Option<SessionSource>,
    max_tokens: i64,
    /// Reasoning effort for extended thinking. When set, enables Claude's
    /// extended thinking capability with a budget proportional to the effort level.
    reasoning_effort: Option<ReasoningEffort>,
}

impl<'a> AnthropicRequestBuilder<'a> {
    pub fn new(
        model: &'a str,
        instructions: &'a str,
        input: &'a [ResponseItem],
        tools: &'a [Value],
    ) -> Self {
        Self {
            model,
            instructions,
            input,
            tools,
            conversation_id: None,
            session_source: None,
            max_tokens: DEFAULT_MAX_TOKENS,
            reasoning_effort: None,
        }
    }

    pub fn conversation_id(mut self, id: Option<String>) -> Self {
        self.conversation_id = id;
        self
    }

    pub fn session_source(mut self, source: Option<SessionSource>) -> Self {
        self.session_source = source;
        self
    }

    pub fn max_tokens(mut self, max_tokens: i64) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    /// Sets the reasoning effort for extended thinking.
    /// When set, Claude will use extended thinking with a budget proportional to the effort.
    pub fn reasoning_effort(mut self, effort: Option<ReasoningEffort>) -> Self {
        self.reasoning_effort = effort;
        self
    }

    pub fn build(self, _provider: &Provider) -> Result<AnthropicRequest, ApiError> {
        let mut messages = Vec::<Value>::new();
        let reasoning_effort = self
            .reasoning_effort
            .and_then(|effort| effort_to_budget_tokens(effort).map(|_| effort));
        let thinking_enabled = reasoning_effort.is_some();

        // First pass: collect all tool_result IDs that exist in the conversation.
        // Anthropic requires every tool_use to have a corresponding tool_result.
        // We'll only include tool_use blocks that have matching tool_results.
        let mut tool_result_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for item in self.input {
            if let ResponseItem::FunctionCallOutput { call_id, .. } = item {
                tool_result_ids.insert(call_id.clone());
            }
        }

        // Anthropic requires tool_use blocks to be followed immediately by tool_result blocks.
        // We need to batch consecutive FunctionCalls into one assistant message,
        // and consecutive FunctionCallOutputs into one user message.
        let mut pending_tool_uses: Vec<Value> = Vec::new();
        let mut pending_tool_results: Vec<Value> = Vec::new();
        let mut deferred_messages: Vec<Value> = Vec::new();
        let mut pending_thinking: Vec<Value> = Vec::new();

        // Helper to flush pending tool uses as an assistant message
        let flush_tool_uses = |messages: &mut Vec<Value>,
                               tool_uses: &mut Vec<Value>,
                               pending_thinking: &mut Vec<Value>| {
            if !tool_uses.is_empty() {
                let mut content = Vec::new();
                if thinking_enabled {
                    if !pending_thinking.is_empty() {
                        content.append(pending_thinking);
                    } else {
                        debug!("Missing signed thinking block before tool_use");
                    }
                } else {
                    pending_thinking.clear();
                }
                content.append(tool_uses);
                messages.push(json!({
                    "role": "assistant",
                    "content": content,
                }));
            }
        };

        // Helper to flush pending tool results as a user message
        let flush_tool_results = |messages: &mut Vec<Value>, tool_results: &mut Vec<Value>| {
            if !tool_results.is_empty() {
                messages.push(json!({
                    "role": "user",
                    "content": std::mem::take(tool_results),
                }));
            }
        };

        let flush_deferred_messages = |messages: &mut Vec<Value>, deferred: &mut Vec<Value>| {
            if !deferred.is_empty() {
                messages.append(deferred);
            }
        };

        let flush_thinking_only = |messages: &mut Vec<Value>, pending: &mut Vec<Value>| {
            if !thinking_enabled {
                pending.clear();
                return;
            }
            if !pending.is_empty() {
                messages.push(json!({
                    "role": "assistant",
                    "content": std::mem::take(pending),
                }));
            }
        };

        // Build messages from input (Anthropic doesn't have system role in messages)
        for item in self.input {
            match item {
                ResponseItem::Reasoning {
                    content,
                    encrypted_content,
                    thinking_signature,
                    thinking_block_type,
                    ..
                } => {
                    if !thinking_enabled {
                        continue;
                    }

                    let Some(signature) = thinking_signature else {
                        continue;
                    };

                    let mut thinking_text = String::new();
                    if let Some(content) = content {
                        for item in content {
                            if let ReasoningItemContent::ReasoningText { text } = item {
                                thinking_text.push_str(text);
                            }
                        }
                    }

                    let is_redacted = match thinking_block_type.as_deref() {
                        Some("redacted_thinking") => true,
                        Some("thinking") => false,
                        Some(other) => {
                            debug!(block_type = %other, "Skipping signed thinking block with unknown type");
                            continue;
                        }
                        None => thinking_text.is_empty() && encrypted_content.is_some(),
                    };

                    if is_redacted {
                        let Some(data) = encrypted_content.as_ref() else {
                            debug!("Skipping redacted thinking block without encrypted content");
                            continue;
                        };
                        pending_thinking.push(json!({
                            "type": "redacted_thinking",
                            "data": data,
                            "signature": signature,
                        }));
                        continue;
                    }

                    if thinking_text.is_empty() {
                        debug!("Skipping signed thinking block without reasoning text");
                        continue;
                    }

                    pending_thinking.push(json!({
                        "type": "thinking",
                        "thinking": thinking_text,
                        "signature": signature,
                    }));
                }
                ResponseItem::Message { role, content, .. } => {
                    let anthropic_content = self.build_anthropic_content(content);
                    if anthropic_content.is_empty() {
                        continue;
                    }

                    // Anthropic only supports "user" and "assistant" roles
                    let anthropic_role = match role.as_str() {
                        "user" => "user",
                        "assistant" => "assistant",
                        "system" => continue, // System messages handled separately
                        _ => "user",          // Default unknown roles to user
                    };

                    let message = json!({
                        "role": anthropic_role,
                        "content": anthropic_content,
                    });

                    if !pending_tool_uses.is_empty() && pending_tool_results.is_empty() {
                        // Defer messages until after tool_result to satisfy Anthropic ordering.
                        deferred_messages.push(message);
                        continue;
                    }

                    // Flush any pending tool interactions before adding a regular message
                    flush_tool_uses(&mut messages, &mut pending_tool_uses, &mut pending_thinking);
                    flush_tool_results(&mut messages, &mut pending_tool_results);
                    flush_deferred_messages(&mut messages, &mut deferred_messages);

                    if anthropic_role == "assistant" {
                        if thinking_enabled && !pending_thinking.is_empty() {
                            let mut content = std::mem::take(&mut pending_thinking);
                            if let Some(items) = message.get("content").and_then(|c| c.as_array()) {
                                content.extend(items.clone());
                            }
                            messages.push(json!({
                                "role": "assistant",
                                "content": content,
                            }));
                        } else {
                            if !thinking_enabled {
                                pending_thinking.clear();
                            }
                            messages.push(message);
                        }
                    } else {
                        flush_thinking_only(&mut messages, &mut pending_thinking);
                        messages.push(message);
                    }
                }
                ResponseItem::FunctionCall {
                    name,
                    arguments,
                    call_id,
                    ..
                } => {
                    // Only include tool_use blocks that have corresponding tool_results
                    // Anthropic requires every tool_use to have a matching tool_result
                    if !tool_result_ids.contains(call_id) {
                        debug!(
                            call_id = %call_id,
                            name = %name,
                            "Skipping tool_use without corresponding tool_result"
                        );
                        continue;
                    }

                    // Flush any pending tool results before starting new tool uses
                    if !pending_tool_results.is_empty() {
                        flush_tool_uses(
                            &mut messages,
                            &mut pending_tool_uses,
                            &mut pending_thinking,
                        );
                        flush_tool_results(&mut messages, &mut pending_tool_results);
                        flush_deferred_messages(&mut messages, &mut deferred_messages);
                    }

                    // Anthropic uses tool_use blocks for function calls
                    let args: Value = serde_json::from_str(arguments).unwrap_or(json!({}));
                    pending_tool_uses.push(json!({
                        "type": "tool_use",
                        "id": call_id,
                        "name": name,
                        "input": args,
                    }));
                }
                ResponseItem::FunctionCallOutput { call_id, output } => {
                    // Flush any pending tool uses before adding tool results
                    flush_tool_uses(&mut messages, &mut pending_tool_uses, &mut pending_thinking);

                    // Anthropic uses tool_result blocks for function outputs
                    // Note: Anthropic requires non-whitespace text in content blocks
                    let content_value = if let Some(items) = &output.content_items {
                        let mapped: Vec<Value> = items
                            .iter()
                            .filter_map(|it| match it {
                                FunctionCallOutputContentItem::InputText { text } => {
                                    // Filter out whitespace-only text
                                    if text.trim().is_empty() {
                                        None
                                    } else {
                                        Some(json!({"type": "text", "text": text}))
                                    }
                                }
                                FunctionCallOutputContentItem::InputImage { image_url } => {
                                    // Anthropic image format for tool results
                                    if let Some(base64_data) = extract_base64_image(image_url) {
                                        Some(json!({
                                            "type": "image",
                                            "source": {
                                                "type": "base64",
                                                "media_type": "image/png",
                                                "data": base64_data,
                                            }
                                        }))
                                    } else {
                                        // URL-based images aren't directly supported in tool results
                                        Some(json!({"type": "text", "text": format!("[Image: {image_url}]")}))
                                    }
                                }
                            })
                            .collect();
                        // If all content was filtered out, provide a placeholder
                        if mapped.is_empty() {
                            json!([{"type": "text", "text": "[empty]"}])
                        } else {
                            json!(mapped)
                        }
                    } else if output.content.trim().is_empty() {
                        // Handle whitespace-only content
                        json!([{"type": "text", "text": "[empty]"}])
                    } else {
                        json!([{"type": "text", "text": output.content}])
                    };

                    pending_tool_results.push(json!({
                        "type": "tool_result",
                        "tool_use_id": call_id,
                        "content": content_value,
                    }));
                }
                ResponseItem::LocalShellCall { .. }
                | ResponseItem::CustomToolCall { .. }
                | ResponseItem::CustomToolCallOutput { .. }
                | ResponseItem::WebSearchCall { .. }
                | ResponseItem::GhostSnapshot { .. }
                | ResponseItem::Compaction { .. }
                | ResponseItem::Other => {
                    continue;
                }
            }
        }

        // Flush any remaining pending tool interactions
        flush_tool_uses(&mut messages, &mut pending_tool_uses, &mut pending_thinking);
        flush_tool_results(&mut messages, &mut pending_tool_results);
        flush_deferred_messages(&mut messages, &mut deferred_messages);
        flush_thinking_only(&mut messages, &mut pending_thinking);

        // Convert OpenAI-style tools to Anthropic format
        let anthropic_tools = self.convert_tools_to_anthropic();

        let mut payload = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "messages": messages,
            "stream": true,
        });

        // Add system prompt if present
        if !self.instructions.is_empty() {
            payload["system"] = json!(self.instructions);
        }

        // Add tools if present
        if !anthropic_tools.is_empty() {
            payload["tools"] = json!(anthropic_tools);
        }

        // Add extended thinking if reasoning effort is set and not None
        // Anthropic's thinking parameter format:
        // { "type": "enabled", "budget_tokens": N }
        // Note: max_tokens must be greater than budget_tokens
        if let Some(effort) = reasoning_effort
            && let Some(budget_tokens) = effort_to_budget_tokens(effort)
        {
            // Ensure max_tokens > budget_tokens
            let required_max_tokens = budget_tokens + RESPONSE_TOKEN_BUFFER;
            if required_max_tokens > self.max_tokens {
                payload["max_tokens"] = json!(required_max_tokens);
            }
            payload["thinking"] = json!({
                "type": "enabled",
                "budget_tokens": budget_tokens
            });
            debug!(
                effort = ?effort,
                budget_tokens = budget_tokens,
                max_tokens = required_max_tokens,
                "Enabled extended thinking for Anthropic request"
            );
        }

        let mut headers = build_conversation_headers(self.conversation_id);
        if let Some(subagent) = subagent_header(&self.session_source) {
            insert_header(&mut headers, "x-openai-subagent", &subagent);
        }

        // Anthropic API version header (required for Azure AI Services)
        // Use 2023-06-01 which is widely supported
        // Note: Extended thinking requires newer versions - Azure may not support it yet
        insert_header(&mut headers, "anthropic-version", "2023-06-01");

        Ok(AnthropicRequest {
            body: payload,
            headers,
        })
    }

    /// Build Anthropic-format content from ResponseItem content.
    fn build_anthropic_content(&self, content: &[ContentItem]) -> Vec<Value> {
        content
            .iter()
            .filter_map(|c| match c {
                ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                    // Anthropic requires non-whitespace text in content blocks
                    if text.trim().is_empty() {
                        None
                    } else {
                        Some(json!({"type": "text", "text": text}))
                    }
                }
                ContentItem::InputImage { image_url } => {
                    // Anthropic requires base64-encoded images or specific URL formats
                    if let Some(base64_data) = extract_base64_image(image_url) {
                        let media_type = detect_media_type(image_url);
                        Some(json!({
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": media_type,
                                "data": base64_data,
                            }
                        }))
                    } else {
                        // For URL-based images, try using URL source type
                        Some(json!({
                            "type": "image",
                            "source": {
                                "type": "url",
                                "url": image_url,
                            }
                        }))
                    }
                }
            })
            .collect()
    }

    /// Convert OpenAI-style tools to Anthropic format.
    fn convert_tools_to_anthropic(&self) -> Vec<Value> {
        self.tools
            .iter()
            .filter_map(|tool| {
                // OpenAI format: {"type": "function", "function": {"name": ..., "parameters": ...}}
                // Anthropic format: {"name": ..., "input_schema": ...}
                let function = tool.get("function")?;
                let name = function.get("name")?.as_str()?;
                let description = function.get("description").and_then(|d| d.as_str());
                let parameters = function.get("parameters").cloned().unwrap_or(json!({}));

                let mut anthropic_tool = json!({
                    "name": name,
                    "input_schema": parameters,
                });

                if let Some(desc) = description {
                    anthropic_tool["description"] = json!(desc);
                }

                Some(anthropic_tool)
            })
            .collect()
    }
}

/// Extract base64 data from a data URL.
fn extract_base64_image(url: &str) -> Option<&str> {
    if url.starts_with("data:image/") {
        url.find(";base64,").map(|idx| &url[idx + 8..])
    } else {
        None
    }
}

/// Detect media type from image URL or data URL.
fn detect_media_type(url: &str) -> &str {
    if url.starts_with("data:image/png") {
        "image/png"
    } else if url.starts_with("data:image/jpeg") || url.starts_with("data:image/jpg") {
        "image/jpeg"
    } else if url.starts_with("data:image/gif") {
        "image/gif"
    } else if url.starts_with("data:image/webp") {
        "image/webp"
    } else if url.ends_with(".png") {
        "image/png"
    } else if url.ends_with(".jpg") || url.ends_with(".jpeg") {
        "image/jpeg"
    } else if url.ends_with(".gif") {
        "image/gif"
    } else if url.ends_with(".webp") {
        "image/webp"
    } else {
        "image/png" // Default to PNG
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::RetryConfig;
    use crate::provider::WireApi;
    use codex_protocol::models::FunctionCallOutputPayload;
    use codex_protocol::models::ReasoningItemContent;
    use pretty_assertions::assert_eq;
    use std::time::Duration;

    fn provider() -> Provider {
        Provider {
            name: "azure-anthropic".to_string(),
            base_url: "https://test.services.ai.azure.com/anthropic/v1".to_string(),
            query_params: None,
            wire: WireApi::Anthropic,
            headers: HeaderMap::new(),
            retry: RetryConfig {
                max_attempts: 1,
                base_delay: Duration::from_millis(10),
                retry_429: false,
                retry_5xx: true,
                retry_transport: true,
            },
            stream_idle_timeout: Duration::from_secs(1),
        }
    }

    #[test]
    fn builds_basic_anthropic_request() {
        let input = vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Hello".to_string(),
            }],
        }];

        let req = AnthropicRequestBuilder::new("claude-3-opus", "Be helpful", &input, &[])
            .build(&provider())
            .expect("request");

        assert_eq!(req.body["model"], "claude-3-opus");
        assert_eq!(req.body["system"], "Be helpful");
        assert!(req.body["stream"].as_bool().unwrap_or(false));
        assert_eq!(req.body["messages"][0]["role"], "user");
    }

    #[test]
    fn converts_tools_to_anthropic_format() {
        let openai_tools = vec![json!({
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Get the weather",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "location": {"type": "string"}
                    }
                }
            }
        })];

        let input = vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Hi".to_string(),
            }],
        }];

        let req = AnthropicRequestBuilder::new("claude-3-opus", "", &input, &openai_tools)
            .build(&provider())
            .expect("request");

        let tools = req.body["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "get_weather");
        assert!(tools[0]["input_schema"].is_object());
    }

    #[test]
    fn defers_messages_until_after_tool_results() {
        let input = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "Hi".to_string(),
                }],
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "get_weather".to_string(),
                arguments: "{}".to_string(),
                call_id: "call-1".to_string(),
            },
            ResponseItem::Message {
                id: None,
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "working".to_string(),
                }],
            },
            ResponseItem::FunctionCallOutput {
                call_id: "call-1".to_string(),
                output: FunctionCallOutputPayload {
                    content: "ok".to_string(),
                    ..Default::default()
                },
            },
            ResponseItem::Message {
                id: None,
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "done".to_string(),
                }],
            },
        ];

        let req = AnthropicRequestBuilder::new("claude-3-opus", "", &input, &[])
            .reasoning_effort(Some(ReasoningEffort::Low))
            .build(&provider())
            .expect("request");

        let messages = req.body["messages"].as_array().expect("messages array");

        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"][0]["type"], "tool_use");
        assert_eq!(messages[2]["role"], "user");
        assert_eq!(messages[2]["content"][0]["type"], "tool_result");
        assert_eq!(messages[2]["content"][0]["tool_use_id"], "call-1");
        assert_eq!(messages[3]["role"], "assistant");
        assert_eq!(messages[3]["content"][0]["text"], "working");
        assert_eq!(messages[4]["role"], "assistant");
        assert_eq!(messages[4]["content"][0]["text"], "done");
    }

    #[test]
    fn includes_signed_thinking_before_tool_use() {
        let input = vec![
            ResponseItem::Reasoning {
                id: "thinking-1".to_string(),
                summary: vec![],
                content: Some(vec![ReasoningItemContent::ReasoningText {
                    text: "thinking text".to_string(),
                }]),
                encrypted_content: None,
                thinking_signature: Some("sig-1".to_string()),
                thinking_block_type: Some("thinking".to_string()),
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "get_weather".to_string(),
                arguments: "{}".to_string(),
                call_id: "call-1".to_string(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: "call-1".to_string(),
                output: FunctionCallOutputPayload {
                    content: "ok".to_string(),
                    ..Default::default()
                },
            },
        ];

        let req = AnthropicRequestBuilder::new("claude-3-opus", "", &input, &[])
            .reasoning_effort(Some(ReasoningEffort::Low))
            .build(&provider())
            .expect("request");

        let messages = req.body["messages"].as_array().expect("messages array");

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[0]["content"][0]["type"], "thinking");
        assert_eq!(messages[0]["content"][0]["thinking"], "thinking text");
        assert_eq!(messages[0]["content"][0]["signature"], "sig-1");
        assert_eq!(messages[0]["content"][1]["type"], "tool_use");
        assert!(req.body.get("thinking").is_some());
    }

    #[test]
    fn treats_missing_block_type_with_encrypted_content_as_redacted() {
        let input = vec![
            ResponseItem::Reasoning {
                id: "thinking-1".to_string(),
                summary: vec![],
                content: None,
                encrypted_content: Some("redacted-data".to_string()),
                thinking_signature: Some("sig-1".to_string()),
                thinking_block_type: None,
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "get_weather".to_string(),
                arguments: "{}".to_string(),
                call_id: "call-1".to_string(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: "call-1".to_string(),
                output: FunctionCallOutputPayload {
                    content: "ok".to_string(),
                    ..Default::default()
                },
            },
        ];

        let req = AnthropicRequestBuilder::new("claude-3-opus", "", &input, &[])
            .reasoning_effort(Some(ReasoningEffort::Low))
            .build(&provider())
            .expect("request");

        let messages = req.body["messages"].as_array().expect("messages array");

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[0]["content"][0]["type"], "redacted_thinking");
        assert_eq!(messages[0]["content"][0]["data"], "redacted-data");
        assert_eq!(messages[0]["content"][0]["signature"], "sig-1");
        assert_eq!(messages[0]["content"][1]["type"], "tool_use");
    }

    #[test]
    fn reasoning_content_is_ignored_when_thinking_disabled() {
        let input = vec![
            ResponseItem::Reasoning {
                id: "reasoning-1".to_string(),
                summary: vec![],
                content: Some(vec![ReasoningItemContent::ReasoningText {
                    text: "thinking text".to_string(),
                }]),
                encrypted_content: None,
                thinking_signature: Some("sig-1".to_string()),
                thinking_block_type: Some("thinking".to_string()),
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "get_weather".to_string(),
                arguments: "{}".to_string(),
                call_id: "call-1".to_string(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: "call-1".to_string(),
                output: FunctionCallOutputPayload {
                    content: "ok".to_string(),
                    ..Default::default()
                },
            },
        ];

        let req = AnthropicRequestBuilder::new("claude-3-opus", "", &input, &[])
            .build(&provider())
            .expect("request");

        let messages = req.body["messages"].as_array().expect("messages array");

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[0]["content"][0]["type"], "tool_use");
        assert!(req.body.get("thinking").is_none());
    }
}
