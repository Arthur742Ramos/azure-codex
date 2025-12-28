use std::sync::Arc;

use async_trait::async_trait;
use codex_protocol::items::TurnItem;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::AgentMessageContentDeltaEvent;
use codex_protocol::protocol::AgentMessageDeltaEvent;
use codex_protocol::protocol::BackgroundEventEvent;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ExitedReviewModeEvent;
use codex_protocol::protocol::ItemCompletedEvent;
use codex_protocol::protocol::ReviewOutputEvent;
use codex_protocol::protocol::WarningEvent;
use tokio_util::sync::CancellationToken;

use crate::codex::Session;
use crate::codex::TurnContext;
use crate::codex_delegate::run_codex_conversation_one_shot;
use crate::config::Constrained;
use crate::model_provider_info::WireApi;
use crate::review_format::format_review_findings_block;
use crate::review_format::render_review_output_text;
use crate::state::TaskKind;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::SandboxPolicy;
use codex_protocol::user_input::UserInput;

use super::SessionTask;
use super::SessionTaskContext;

#[derive(Clone, Copy)]
pub(crate) struct ReviewTask {
    auto_fix: bool,
}

impl ReviewTask {
    pub(crate) fn new(auto_fix: bool) -> Self {
        Self { auto_fix }
    }
}

#[async_trait]
impl SessionTask for ReviewTask {
    fn kind(&self) -> TaskKind {
        TaskKind::Review
    }

    async fn run(
        self: Arc<Self>,
        session: Arc<SessionTaskContext>,
        ctx: Arc<TurnContext>,
        input: Vec<UserInput>,
        cancellation_token: CancellationToken,
    ) -> Option<String> {
        let output = if self.auto_fix {
            review_with_auto_fix(
                session.clone(),
                ctx.clone(),
                input,
                cancellation_token.clone(),
            )
            .await
        } else {
            run_review_once(
                session.clone(),
                ctx.clone(),
                input,
                cancellation_token.clone(),
            )
            .await
        };
        if !cancellation_token.is_cancelled() {
            exit_review_mode(session.clone_session(), output.clone(), ctx.clone()).await;
        }
        None
    }

    async fn abort(&self, session: Arc<SessionTaskContext>, ctx: Arc<TurnContext>) {
        exit_review_mode(session.clone_session(), None, ctx).await;
    }
}

const MAX_REVIEW_FIX_ROUNDS: usize = 5;

async fn review_with_auto_fix(
    session: Arc<SessionTaskContext>,
    ctx: Arc<TurnContext>,
    input: Vec<UserInput>,
    cancellation_token: CancellationToken,
) -> Option<ReviewOutputEvent> {
    let mut last_review: Option<ReviewOutputEvent> = None;
    let mut previous_review: Option<ReviewOutputEvent> = None;
    let mut stopped_due_to_limit = true;

    for round in 1..=MAX_REVIEW_FIX_ROUNDS {
        emit_background(
            session.clone_session().as_ref(),
            ctx.as_ref(),
            format!("Reviewing changes ({round}/{MAX_REVIEW_FIX_ROUNDS})..."),
        )
        .await;
        let review_output = run_review_once(
            session.clone(),
            ctx.clone(),
            input.clone(),
            cancellation_token.clone(),
        )
        .await?;
        let has_findings = !review_output.findings.is_empty();
        let unchanged = previous_review
            .as_ref()
            .is_some_and(|prev| prev == &review_output);
        last_review = Some(review_output.clone());

        if !has_findings || unchanged {
            stopped_due_to_limit = false;
            break;
        }

        if !can_auto_fix(ctx.as_ref()) {
            emit_warning(
                session.clone_session().as_ref(),
                ctx.as_ref(),
                "Auto-fix may not apply because the current sandbox policy is read-only.",
            )
            .await;
        }

        emit_background(
            session.clone_session().as_ref(),
            ctx.as_ref(),
            format!("Applying fixes ({round}/{MAX_REVIEW_FIX_ROUNDS})..."),
        )
        .await;
        run_fix_once(
            session.clone(),
            ctx.clone(),
            &review_output,
            cancellation_token.clone(),
        )
        .await;
        previous_review = Some(review_output);
    }

    if stopped_due_to_limit
        && last_review
            .as_ref()
            .is_some_and(|review_output| !review_output.findings.is_empty())
    {
        emit_warning(
            session.clone_session().as_ref(),
            ctx.as_ref(),
            "Auto-fix stopped after the maximum number of attempts; remaining findings need attention.",
        )
        .await;
    }

    last_review
}

fn can_auto_fix(ctx: &TurnContext) -> bool {
    !matches!(ctx.sandbox_policy, SandboxPolicy::ReadOnly)
}

async fn emit_background(session: &Session, ctx: &TurnContext, message: String) {
    session
        .send_event(
            ctx,
            EventMsg::BackgroundEvent(BackgroundEventEvent { message }),
        )
        .await;
}

async fn emit_warning(session: &Session, ctx: &TurnContext, message: &str) {
    session
        .send_event(
            ctx,
            EventMsg::Warning(WarningEvent {
                message: message.to_string(),
            }),
        )
        .await;
}

async fn run_review_once(
    session: Arc<SessionTaskContext>,
    ctx: Arc<TurnContext>,
    input: Vec<UserInput>,
    cancellation_token: CancellationToken,
) -> Option<ReviewOutputEvent> {
    // Start sub-codex conversation and get the receiver for events.
    match start_review_conversation(
        session.clone(),
        ctx.clone(),
        input,
        cancellation_token.clone(),
    )
    .await
    {
        Some(receiver) => process_review_events(session, ctx, receiver).await,
        None => None,
    }
}

async fn run_fix_once(
    session: Arc<SessionTaskContext>,
    ctx: Arc<TurnContext>,
    review_output: &ReviewOutputEvent,
    cancellation_token: CancellationToken,
) {
    let prompt = build_fix_prompt(review_output);
    let input = vec![UserInput::Text { text: prompt }];
    let mut config = (*ctx.client.config()).clone();
    config.approval_policy = Constrained::allow_any(AskForApproval::Never);

    let output = run_codex_conversation_one_shot(
        config,
        session.auth_manager(),
        session.models_manager(),
        input,
        session.clone_session(),
        ctx.clone(),
        cancellation_token,
        None,
    )
    .await;

    if let Ok(receiver) = output.map(|io| io.rx_event) {
        process_fix_events(session, ctx, receiver).await;
    }
}

fn build_fix_prompt(review_output: &ReviewOutputEvent) -> String {
    let findings = format_review_findings_block(&review_output.findings, None);
    format!(
        "Fix the issues described in the review findings below. Apply the minimal changes needed, use apply_patch for edits, and avoid unrelated refactors.\n{findings}\n\nAfter applying fixes, briefly summarize what changed."
    )
}

async fn start_review_conversation(
    session: Arc<SessionTaskContext>,
    ctx: Arc<TurnContext>,
    input: Vec<UserInput>,
    cancellation_token: CancellationToken,
) -> Option<async_channel::Receiver<Event>> {
    let config = ctx.client.config();
    let mut sub_agent_config = config.as_ref().clone();
    // Run with only reviewer rubric — drop outer user_instructions
    sub_agent_config.user_instructions = None;
    // Avoid loading project docs; reviewer only needs findings
    sub_agent_config.project_doc_max_bytes = 0;
    // Carry over review-only feature restrictions so the delegate cannot
    // re-enable blocked tools (web search, view image).
    sub_agent_config
        .features
        .disable(crate::features::Feature::WebSearchRequest)
        .disable(crate::features::Feature::ViewImageTool);

    // Set explicit review rubric for the sub-agent
    sub_agent_config.base_instructions = Some(crate::REVIEW_PROMPT.to_string());

    // For Azure (both OpenAI and Anthropic), the default review_model (gpt-5.1-codex-max)
    // likely won't exist as a deployment, so use the main model instead. For standard
    // OpenAI, use the configured review_model which may be optimized for code review.
    let is_azure = config.azure_endpoint.is_some();
    let is_anthropic = config.model_provider.wire_api == WireApi::Anthropic;
    let use_main_model = is_azure || is_anthropic;
    if use_main_model && config.model.is_some() {
        tracing::debug!(
            "Using main model {:?} for review (Azure/Anthropic provider)",
            config.model
        );
        sub_agent_config.model = config.model.clone();
    } else {
        sub_agent_config.model = Some(config.review_model.clone());
    }
    (run_codex_conversation_one_shot(
        sub_agent_config,
        session.auth_manager(),
        session.models_manager(),
        input,
        session.clone_session(),
        ctx.clone(),
        cancellation_token,
        None,
    )
    .await)
        .ok()
        .map(|io| io.rx_event)
}

async fn process_review_events(
    session: Arc<SessionTaskContext>,
    ctx: Arc<TurnContext>,
    receiver: async_channel::Receiver<Event>,
) -> Option<ReviewOutputEvent> {
    let mut prev_agent_message: Option<Event> = None;
    while let Ok(event) = receiver.recv().await {
        match event.clone().msg {
            EventMsg::AgentMessage(_) => {
                if let Some(prev) = prev_agent_message.take() {
                    session
                        .clone_session()
                        .send_event(ctx.as_ref(), prev.msg)
                        .await;
                }
                prev_agent_message = Some(event);
            }
            // Suppress ItemCompleted only for assistant messages: forwarding it
            // would trigger legacy AgentMessage via as_legacy_events(), which this
            // review flow intentionally hides in favor of structured output.
            EventMsg::ItemCompleted(ItemCompletedEvent {
                item: TurnItem::AgentMessage(_),
                ..
            })
            | EventMsg::AgentMessageDelta(AgentMessageDeltaEvent { .. })
            | EventMsg::AgentMessageContentDelta(AgentMessageContentDeltaEvent { .. }) => {}
            EventMsg::TaskComplete(task_complete) => {
                // Parse review output from the last agent message (if present).
                let out = task_complete
                    .last_agent_message
                    .as_deref()
                    .map(parse_review_output_event);
                return out;
            }
            EventMsg::TurnAborted(_) => {
                // Cancellation or abort: consumer will finalize with None.
                return None;
            }
            other => {
                session
                    .clone_session()
                    .send_event(ctx.as_ref(), other)
                    .await;
            }
        }
    }
    // Channel closed without TaskComplete: treat as interrupted.
    None
}

async fn process_fix_events(
    session: Arc<SessionTaskContext>,
    ctx: Arc<TurnContext>,
    receiver: async_channel::Receiver<Event>,
) {
    while let Ok(event) = receiver.recv().await {
        match event.msg {
            EventMsg::AgentMessage(_)
            | EventMsg::AgentMessageDelta(_)
            | EventMsg::AgentMessageContentDelta(_)
            | EventMsg::ReasoningContentDelta(_)
            | EventMsg::ReasoningRawContentDelta(_) => {}
            EventMsg::ItemCompleted(ItemCompletedEvent {
                item: TurnItem::AgentMessage(_),
                ..
            }) => {}
            EventMsg::TaskComplete(_) | EventMsg::TurnAborted(_) => break,
            other => {
                session
                    .clone_session()
                    .send_event(ctx.as_ref(), other)
                    .await;
            }
        }
    }
}

/// Parse a ReviewOutputEvent from a text blob returned by the reviewer model.
/// If the text is valid JSON matching ReviewOutputEvent, deserialize it.
/// Otherwise, attempt to extract the first JSON object substring and parse it.
/// If parsing still fails, return a structured fallback carrying the plain text
/// in `overall_explanation`.
fn parse_review_output_event(text: &str) -> ReviewOutputEvent {
    if let Ok(ev) = serde_json::from_str::<ReviewOutputEvent>(text) {
        return ev;
    }
    if let (Some(start), Some(end)) = (text.find('{'), text.rfind('}'))
        && start < end
        && let Some(slice) = text.get(start..=end)
        && let Ok(ev) = serde_json::from_str::<ReviewOutputEvent>(slice)
    {
        return ev;
    }
    ReviewOutputEvent {
        overall_explanation: text.to_string(),
        ..Default::default()
    }
}

/// Emits an ExitedReviewMode Event with optional ReviewOutput,
/// and records a developer message with the review output.
pub(crate) async fn exit_review_mode(
    session: Arc<Session>,
    review_output: Option<ReviewOutputEvent>,
    ctx: Arc<TurnContext>,
) {
    const REVIEW_USER_MESSAGE_ID: &str = "review:rollout:user";
    const REVIEW_ASSISTANT_MESSAGE_ID: &str = "review:rollout:assistant";
    let (user_message, assistant_message) = if let Some(out) = review_output.clone() {
        let mut findings_str = String::new();
        let text = out.overall_explanation.trim();
        if !text.is_empty() {
            findings_str.push_str(text);
        }
        if !out.findings.is_empty() {
            let block = format_review_findings_block(&out.findings, None);
            findings_str.push_str(&format!("\n{block}"));
        }
        let rendered =
            crate::client_common::REVIEW_EXIT_SUCCESS_TMPL.replace("{results}", &findings_str);
        let assistant_message = render_review_output_text(&out);
        (rendered, assistant_message)
    } else {
        let rendered = crate::client_common::REVIEW_EXIT_INTERRUPTED_TMPL.to_string();
        let assistant_message =
            "Review was interrupted. Please re-run /review and wait for it to complete."
                .to_string();
        (rendered, assistant_message)
    };

    session
        .record_conversation_items(
            &ctx,
            &[ResponseItem::Message {
                id: Some(REVIEW_USER_MESSAGE_ID.to_string()),
                role: "user".to_string(),
                content: vec![ContentItem::InputText { text: user_message }],
            }],
        )
        .await;
    session
        .send_event(
            ctx.as_ref(),
            EventMsg::ExitedReviewMode(ExitedReviewModeEvent { review_output }),
        )
        .await;
    session
        .record_response_item_and_emit_turn_item(
            ctx.as_ref(),
            ResponseItem::Message {
                id: Some(REVIEW_ASSISTANT_MESSAGE_ID.to_string()),
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: assistant_message,
                }],
            },
        )
        .await;
}
