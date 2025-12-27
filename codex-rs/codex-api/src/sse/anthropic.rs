//! Anthropic Messages API SSE parser.
//!
//! This module handles parsing Server-Sent Events from the Anthropic Messages API,
//! which is used for Claude models on Azure AI Services.

use crate::common::ResponseEvent;
use crate::common::ResponseStream;
use crate::error::ApiError;
use crate::telemetry::SseTelemetry;
use codex_client::StreamResponse;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ReasoningItemContent;
use codex_protocol::models::ResponseItem;
use eventsource_stream::Eventsource;
use futures::Stream;
use futures::StreamExt;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio::time::timeout;
use tracing::debug;
use tracing::trace;

pub(crate) fn spawn_anthropic_stream(
    stream_response: StreamResponse,
    idle_timeout: Duration,
    telemetry: Option<std::sync::Arc<dyn SseTelemetry>>,
) -> ResponseStream {
    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent, ApiError>>(1600);
    tokio::spawn(async move {
        process_anthropic_sse(stream_response.bytes, tx_event, idle_timeout, telemetry).await;
    });
    ResponseStream { rx_event }
}

pub async fn process_anthropic_sse<S>(
    stream: S,
    tx_event: mpsc::Sender<Result<ResponseEvent, ApiError>>,
    idle_timeout: Duration,
    telemetry: Option<std::sync::Arc<dyn SseTelemetry>>,
) where
    S: Stream<Item = Result<bytes::Bytes, codex_client::TransportError>> + Unpin,
{
    let mut stream = stream.eventsource();

    /// State for tracking a tool use block.
    #[derive(Default, Debug)]
    struct ToolUseState {
        id: Option<String>,
        name: Option<String>,
        input_json: String,
    }

    /// State for tracking extended thinking blocks.
    #[derive(Default, Debug)]
    struct ThinkingState {
        /// Accumulated thinking text.
        text: String,
        signature: Option<String>,
        block_type: Option<String>,
        redacted_data: Option<String>,
    }

    let mut assistant_item: Option<ResponseItem> = None;
    let mut tool_uses: HashMap<usize, ToolUseState> = HashMap::new();
    let mut tool_use_order: Vec<usize> = Vec::new();
    let mut thinking_states: HashMap<usize, ThinkingState> = HashMap::new();
    let mut completed_sent = false;

    loop {
        let start = Instant::now();
        let response = timeout(idle_timeout, stream.next()).await;
        if let Some(t) = telemetry.as_ref() {
            t.on_sse_poll(&response, start.elapsed());
        }
        let sse = match response {
            Ok(Some(Ok(sse))) => sse,
            Ok(Some(Err(e))) => {
                let _ = tx_event.send(Err(ApiError::Stream(e.to_string()))).await;
                return;
            }
            Ok(None) => {
                // Stream ended, finalize any pending items
                if let Some(assistant) = assistant_item {
                    let _ = tx_event
                        .send(Ok(ResponseEvent::OutputItemDone(assistant)))
                        .await;
                }
                if !completed_sent {
                    let _ = tx_event
                        .send(Ok(ResponseEvent::Completed {
                            response_id: String::new(),
                            token_usage: None,
                        }))
                        .await;
                }
                return;
            }
            Err(_) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream("idle timeout waiting for SSE".into())))
                    .await;
                return;
            }
        };

        trace!("Anthropic SSE event: {}", sse.data);

        if sse.data.trim().is_empty() {
            continue;
        }

        let value: serde_json::Value = match serde_json::from_str(&sse.data) {
            Ok(val) => val,
            Err(err) => {
                debug!(
                    "Failed to parse Anthropic SSE event: {err}, data: {}",
                    &sse.data
                );
                continue;
            }
        };

        let event_type = value.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match event_type {
            "message_start" => {
                // Initialize message tracking
                let _ = tx_event.send(Ok(ResponseEvent::Created)).await;
            }
            "content_block_start" => {
                let index = value
                    .get("index")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as usize;
                let content_block = value.get("content_block");

                if let Some(block) = content_block {
                    let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");

                    match block_type {
                        "text" => {
                            // Initialize assistant message if not present
                            if assistant_item.is_none() {
                                let item = ResponseItem::Message {
                                    id: None,
                                    role: "assistant".to_string(),
                                    content: vec![],
                                };
                                assistant_item = Some(item.clone());
                                let _ = tx_event
                                    .send(Ok(ResponseEvent::OutputItemAdded(item)))
                                    .await;
                            }
                        }
                        "tool_use" => {
                            // Start tracking a new tool use block
                            let id = block.get("id").and_then(|i| i.as_str()).map(String::from);
                            let name = block.get("name").and_then(|n| n.as_str()).map(String::from);

                            tool_uses.insert(
                                index,
                                ToolUseState {
                                    id,
                                    name,
                                    input_json: String::new(),
                                },
                            );
                            tool_use_order.push(index);
                        }
                        "thinking" => {
                            // Claude's extended thinking block - track for reasoning output
                            let thinking_text =
                                block.get("thinking").and_then(|t| t.as_str()).unwrap_or("");
                            thinking_states.insert(
                                index,
                                ThinkingState {
                                    text: thinking_text.to_string(),
                                    signature: block
                                        .get("signature")
                                        .and_then(|s| s.as_str())
                                        .map(String::from),
                                    block_type: Some(block_type.to_string()),
                                    redacted_data: None,
                                },
                            );
                            debug!("Started thinking block at index {index}");
                        }
                        "redacted_thinking" => {
                            thinking_states.insert(
                                index,
                                ThinkingState {
                                    text: String::new(),
                                    signature: block
                                        .get("signature")
                                        .and_then(|s| s.as_str())
                                        .map(String::from),
                                    block_type: Some(block_type.to_string()),
                                    redacted_data: block
                                        .get("data")
                                        .and_then(|d| d.as_str())
                                        .map(String::from),
                                },
                            );
                            debug!("Started redacted thinking block at index {index}");
                        }
                        _ => {}
                    }
                }
            }
            "content_block_delta" => {
                let index = value
                    .get("index")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as usize;
                let delta = value.get("delta");

                if let Some(d) = delta {
                    let delta_type = d.get("type").and_then(|t| t.as_str()).unwrap_or("");

                    match delta_type {
                        "text_delta" => {
                            if let Some(text) = d.get("text").and_then(|t| t.as_str()) {
                                append_assistant_text(
                                    &tx_event,
                                    &mut assistant_item,
                                    text.to_string(),
                                )
                                .await;
                            }
                        }
                        "input_json_delta" => {
                            // Accumulate tool input JSON
                            if let Some(partial) = d.get("partial_json").and_then(|p| p.as_str())
                                && let Some(state) = tool_uses.get_mut(&index)
                            {
                                state.input_json.push_str(partial);
                            }
                        }
                        "thinking_delta" => {
                            // Extended thinking delta - accumulate and emit reasoning events
                            if let Some(text) = d.get("thinking").and_then(|t| t.as_str())
                                && let Some(state) = thinking_states.get_mut(&index)
                            {
                                state.text.push_str(text);
                                // Emit reasoning content delta for streaming display
                                let _ = tx_event
                                    .send(Ok(ResponseEvent::ReasoningContentDelta {
                                        delta: text.to_string(),
                                        content_index: index as i64,
                                    }))
                                    .await;
                            }
                        }
                        "redacted_thinking_delta" => {
                            if let Some(data) = d.get("data").and_then(|t| t.as_str())
                                && let Some(state) = thinking_states.get_mut(&index)
                            {
                                if let Some(existing) = &mut state.redacted_data {
                                    existing.push_str(data);
                                } else {
                                    state.redacted_data = Some(data.to_string());
                                }
                            }
                        }
                        "signature_delta" => {
                            // Signature is sent as a delta just before content_block_stop
                            // This is critical for extended thinking - the signature validates
                            // the thinking content when sent back to the API
                            if let Some(signature) = d.get("signature").and_then(|s| s.as_str())
                                && let Some(state) = thinking_states.get_mut(&index)
                            {
                                // Append to existing signature or create new one
                                // (signature may come in multiple delta chunks)
                                if let Some(existing) = &mut state.signature {
                                    existing.push_str(signature);
                                } else {
                                    state.signature = Some(signature.to_string());
                                }
                                debug!("Received signature_delta for thinking block {index}");
                            }
                        }
                        _ => {}
                    }
                }
            }
            "content_block_stop" => {
                let index = value
                    .get("index")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as usize;

                // If this is a thinking block, emit the accumulated reasoning
                if let Some(state) = thinking_states.remove(&index)
                    && (!state.text.is_empty()
                        || state.redacted_data.is_some()
                        || state.signature.is_some())
                {
                    let reasoning_item = ResponseItem::Reasoning {
                        id: format!("thinking-{index}"),
                        summary: vec![],
                        content: if state.text.is_empty() {
                            None
                        } else {
                            Some(vec![ReasoningItemContent::ReasoningText {
                                text: state.text,
                            }])
                        },
                        encrypted_content: state.redacted_data,
                        thinking_signature: state.signature,
                        thinking_block_type: state.block_type,
                    };
                    let _ = tx_event
                        .send(Ok(ResponseEvent::OutputItemDone(reasoning_item)))
                        .await;
                    debug!("Emitted reasoning item for thinking block {index}");
                }
                // Tool use blocks wait for message_delta with stop_reason for proper ordering
            }
            "message_delta" => {
                let delta = value.get("delta");

                if let Some(d) = delta {
                    let stop_reason = d.get("stop_reason").and_then(|r| r.as_str());

                    match stop_reason {
                        Some("end_turn") | Some("stop") => {
                            // End of assistant turn - emit assistant message
                            if let Some(assistant) = assistant_item.take() {
                                let _ = tx_event
                                    .send(Ok(ResponseEvent::OutputItemDone(assistant)))
                                    .await;
                            }

                            if !completed_sent {
                                let _ = tx_event
                                    .send(Ok(ResponseEvent::Completed {
                                        response_id: String::new(),
                                        token_usage: None,
                                    }))
                                    .await;
                                completed_sent = true;
                            }
                        }
                        Some("tool_use") => {
                            // Emit all accumulated tool uses
                            for index in tool_use_order.drain(..) {
                                if let Some(state) = tool_uses.remove(&index) {
                                    let ToolUseState {
                                        id,
                                        name,
                                        input_json,
                                    } = state;

                                    let Some(name) = name else {
                                        debug!("Skipping tool use at index {index} - name missing");
                                        continue;
                                    };

                                    let item = ResponseItem::FunctionCall {
                                        id: None,
                                        name,
                                        arguments: input_json,
                                        call_id: id.unwrap_or_else(|| format!("tool-use-{index}")),
                                    };
                                    let _ = tx_event
                                        .send(Ok(ResponseEvent::OutputItemDone(item)))
                                        .await;
                                }
                            }
                        }
                        Some("max_tokens") => {
                            let _ = tx_event.send(Err(ApiError::ContextWindowExceeded)).await;
                            return;
                        }
                        _ => {}
                    }
                }
            }
            "message_stop" => {
                // Final message event - ensure cleanup
                if let Some(assistant) = assistant_item.take() {
                    let _ = tx_event
                        .send(Ok(ResponseEvent::OutputItemDone(assistant)))
                        .await;
                }
                if !completed_sent {
                    let _ = tx_event
                        .send(Ok(ResponseEvent::Completed {
                            response_id: String::new(),
                            token_usage: None,
                        }))
                        .await;
                    completed_sent = true;
                }
            }
            "error" => {
                let error_msg = value
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("Unknown Anthropic API error");
                let _ = tx_event
                    .send(Err(ApiError::Stream(error_msg.to_string())))
                    .await;
                return;
            }
            "ping" => {
                // Keep-alive event, ignore
            }
            _ => {
                trace!("Unknown Anthropic event type: {event_type}");
            }
        }
    }
}

async fn append_assistant_text(
    tx_event: &mpsc::Sender<Result<ResponseEvent, ApiError>>,
    assistant_item: &mut Option<ResponseItem>,
    text: String,
) {
    if assistant_item.is_none() {
        let item = ResponseItem::Message {
            id: None,
            role: "assistant".to_string(),
            content: vec![],
        };
        *assistant_item = Some(item.clone());
        let _ = tx_event
            .send(Ok(ResponseEvent::OutputItemAdded(item)))
            .await;
    }

    if let Some(ResponseItem::Message { content, .. }) = assistant_item {
        content.push(ContentItem::OutputText { text: text.clone() });
        let _ = tx_event
            .send(Ok(ResponseEvent::OutputTextDelta(text.clone())))
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::models::ResponseItem;
    use futures::TryStreamExt;
    use serde_json::json;
    use tokio::sync::mpsc;
    use tokio_util::io::ReaderStream;

    fn build_anthropic_body(events: &[serde_json::Value]) -> String {
        let mut body = String::new();
        for e in events {
            let event_type = e.get("type").and_then(|t| t.as_str()).unwrap_or("message");
            body.push_str(&format!("event: {event_type}\ndata: {e}\n\n"));
        }
        body
    }

    async fn collect_events(body: &str) -> Vec<ResponseEvent> {
        let reader = ReaderStream::new(std::io::Cursor::new(body.to_string()))
            .map_err(|err| codex_client::TransportError::Network(err.to_string()));
        let (tx, mut rx) = mpsc::channel::<Result<ResponseEvent, ApiError>>(16);
        tokio::spawn(process_anthropic_sse(
            reader,
            tx,
            Duration::from_millis(1000),
            None,
        ));

        let mut out = Vec::new();
        while let Some(ev) = rx.recv().await {
            out.push(ev.expect("stream error"));
        }
        out
    }

    #[tokio::test]
    async fn parses_basic_text_response() {
        let events = vec![
            json!({"type": "message_start", "message": {"id": "msg_1"}}),
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Hello"}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": " world"}}),
            json!({"type": "content_block_stop", "index": 0}),
            json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}}),
            json!({"type": "message_stop"}),
        ];

        let body = build_anthropic_body(&events);
        let result = collect_events(&body).await;

        // Should have Created, OutputItemAdded, two text deltas, OutputItemDone, Completed
        assert!(result.iter().any(|e| matches!(e, ResponseEvent::Created)));
        assert!(
            result
                .iter()
                .any(|e| matches!(e, ResponseEvent::OutputItemAdded(_)))
        );
        assert!(
            result
                .iter()
                .any(|e| matches!(e, ResponseEvent::OutputTextDelta(_)))
        );
        assert!(
            result
                .iter()
                .any(|e| matches!(e, ResponseEvent::Completed { .. }))
        );
    }

    #[tokio::test]
    async fn parses_tool_use_response() {
        let events = vec![
            json!({"type": "message_start", "message": {"id": "msg_1"}}),
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": "tool_1", "name": "get_weather"}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": "{\"location\":"}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": "\"NYC\"}"}}),
            json!({"type": "content_block_stop", "index": 0}),
            json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}}),
            json!({"type": "message_stop"}),
        ];

        let body = build_anthropic_body(&events);
        let result = collect_events(&body).await;

        let tool_call = result.iter().find_map(|e| {
            if let ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
                name,
                arguments,
                call_id,
                ..
            }) = e
            {
                Some((name.clone(), arguments.clone(), call_id.clone()))
            } else {
                None
            }
        });

        assert!(tool_call.is_some());
        let (name, args, call_id) = tool_call.unwrap();
        assert_eq!(name, "get_weather");
        assert_eq!(args, "{\"location\":\"NYC\"}");
        assert_eq!(call_id, "tool_1");
    }

    #[tokio::test]
    async fn parses_signed_thinking_block() {
        // The signature is sent via signature_delta just before content_block_stop
        // This is the correct Anthropic streaming format for extended thinking
        let events = vec![
            json!({"type": "message_start", "message": {"id": "msg_1"}}),
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "thinking", "thinking": ""}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": "foo"}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "signature_delta", "signature": "sig-1"}}),
            json!({"type": "content_block_stop", "index": 0}),
            json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}}),
            json!({"type": "message_stop"}),
        ];

        let body = build_anthropic_body(&events);
        let result = collect_events(&body).await;

        let reasoning = result.iter().find_map(|e| {
            if let ResponseEvent::OutputItemDone(ResponseItem::Reasoning {
                thinking_signature,
                thinking_block_type,
                content,
                ..
            }) = e
            {
                Some((
                    thinking_signature.clone(),
                    thinking_block_type.clone(),
                    content.clone(),
                ))
            } else {
                None
            }
        });

        let (signature, block_type, content) =
            reasoning.expect("expected reasoning item from thinking block");
        assert_eq!(signature.as_deref(), Some("sig-1"));
        assert_eq!(block_type.as_deref(), Some("thinking"));
        assert_eq!(
            content,
            Some(vec![ReasoningItemContent::ReasoningText {
                text: "foo".to_string(),
            }])
        );
    }
}
