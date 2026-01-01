use crate::diff_render::create_diff_summary;
use crate::diff_render::display_path_for;
use crate::exec_cell::CommandOutput;
use crate::exec_cell::OutputLinesParams;
use crate::exec_cell::TOOL_CALL_MAX_LINES;
use crate::exec_cell::output_lines;
use crate::exec_cell::spinner;
use crate::exec_command::relativize_to_home;
use crate::exec_command::strip_bash_lc_and_escape;
use crate::markdown::append_markdown;
use crate::render::line_utils::line_to_static;
use crate::render::line_utils::prefix_lines;
use crate::render::renderable::Renderable;
use crate::style::user_message_style;
use crate::text_formatting::format_and_truncate_tool_result;
use crate::text_formatting::truncate_text;
use crate::theme;
use crate::tooltips;
use crate::ui_consts::LIVE_PREFIX_COLS;
use crate::version::CODEX_CLI_VERSION;
use crate::wrapping::RtOptions;
use crate::wrapping::word_wrap_line;
use crate::wrapping::word_wrap_lines;
use base64::Engine;
use codex_common::format_env_display::format_env_display;
use codex_core::config::Config;
use codex_core::config::types::McpServerTransportConfig;
use codex_core::protocol::FileChange;
use codex_core::protocol::McpAuthStatus;
use codex_core::protocol::McpInvocation;
use codex_core::protocol::SessionConfiguredEvent;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use codex_protocol::plan_tool::PlanItemArg;
use codex_protocol::plan_tool::StepStatus;
use codex_protocol::plan_tool::UpdatePlanArgs;
use image::DynamicImage;
use image::ImageReader;
use mcp_types::EmbeddedResourceResource;
use mcp_types::Resource;
use mcp_types::ResourceLink;
use mcp_types::ResourceTemplate;
use ratatui::prelude::*;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::style::Styled;
use ratatui::style::Stylize;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;
use std::any::Any;
use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;
use std::time::Instant;
use tracing::error;
use unicode_width::UnicodeWidthStr;

/// Visual transcript lines plus soft-wrap joiners.
///
/// A history cell can produce multiple "visual lines" once prefixes/indents and wrapping are
/// applied. Clipboard reconstruction needs more information than just those lines: users expect
/// soft-wrapped prose to copy as a single logical line, while explicit newlines and spacer rows
/// should remain hard breaks.
///
/// `joiner_before` records, for each output line, whether it is a continuation created by the
/// wrapping algorithm and what string should be inserted at the wrap boundary when joining lines.
/// This avoids heuristics like always inserting a space, and instead preserves the exact whitespace
/// that was skipped at the boundary.
///
/// ## Note for `codex-tui` vs `codex-tui2`
///
/// In `codex-tui`, `HistoryCell` only exposes `transcript_lines(...)` and the UI generally doesn't
/// need to reconstruct clipboard text across off-screen history or soft-wrap boundaries.
///
/// In `codex-tui2`, transcript selection and copy are app-driven (not terminal-driven) and may span
/// content that isn't currently visible. That means we need additional metadata to distinguish hard
/// breaks from soft wraps and to preserve the exact whitespace at wrap boundaries.
///
/// Invariants:
/// - `joiner_before.len() == lines.len()`
/// - `joiner_before[0]` is always `None`
/// - `None` represents a hard break
/// - `Some(joiner)` represents a soft wrap continuation
///
/// Consumers:
/// - `transcript_render` threads joiners through transcript flattening/wrapping.
/// - `transcript_copy` uses them to join wrapped prose while preserving hard breaks.
#[derive(Debug, Clone)]
pub(crate) struct TranscriptLinesWithJoiners {
    /// Visual transcript lines for a history cell, including any indent/prefix spans.
    ///
    /// This is the same shape used for on-screen transcript rendering: a single cell may expand
    /// to multiple `Line`s after wrapping and prefixing.
    pub(crate) lines: Vec<Line<'static>>,
    /// For each output line, whether and how to join it to the previous line when copying.
    pub(crate) joiner_before: Vec<Option<String>>,
}

/// Represents an event to display in the conversation history. Returns its
/// `Vec<Line<'static>>` representation to make it easier to display in a
/// scrollable list.
pub(crate) trait HistoryCell: std::fmt::Debug + Send + Sync + Any {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>>;

    fn desired_height(&self, width: u16) -> u16 {
        Paragraph::new(Text::from(self.display_lines(width)))
            .wrap(Wrap { trim: false })
            .line_count(width)
            .try_into()
            .unwrap_or(0)
    }

    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.display_lines(width)
    }

    /// Transcript lines plus soft-wrap joiners used for copy/paste fidelity.
    ///
    /// Most cells can use the default implementation (no joiners), but cells that apply wrapping
    /// should override this and return joiners derived from the same wrapping operation so
    /// clipboard reconstruction can distinguish hard breaks from soft wraps.
    fn transcript_lines_with_joiners(&self, width: u16) -> TranscriptLinesWithJoiners {
        let lines = self.transcript_lines(width);
        TranscriptLinesWithJoiners {
            joiner_before: vec![None; lines.len()],
            lines,
        }
    }

    fn desired_transcript_height(&self, width: u16) -> u16 {
        let lines = self.transcript_lines(width);
        // Workaround for ratatui bug: if there's only one line and it's whitespace-only, ratatui gives 2 lines.
        if let [line] = &lines[..]
            && line
                .spans
                .iter()
                .all(|s| s.content.chars().all(char::is_whitespace))
        {
            return 1;
        }

        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: false })
            .line_count(width)
            .try_into()
            .unwrap_or(0)
    }

    fn is_stream_continuation(&self) -> bool {
        false
    }
}

impl Renderable for Box<dyn HistoryCell> {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let lines = self.display_lines(area.width);
        let y = if area.height == 0 {
            0
        } else {
            let overflow = lines.len().saturating_sub(usize::from(area.height));
            u16::try_from(overflow).unwrap_or(u16::MAX)
        };
        Paragraph::new(Text::from(lines))
            .scroll((y, 0))
            .render(area, buf);
    }
    fn desired_height(&self, width: u16) -> u16 {
        HistoryCell::desired_height(self.as_ref(), width)
    }
}

impl dyn HistoryCell {
    pub(crate) fn as_any(&self) -> &dyn Any {
        self
    }

    pub(crate) fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[derive(Debug)]
pub(crate) struct UserHistoryCell {
    pub message: String,
}

impl HistoryCell for UserHistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.transcript_lines_with_joiners(width).lines
    }

    fn transcript_lines_with_joiners(&self, width: u16) -> TranscriptLinesWithJoiners {
        let wrap_width = width
            .saturating_sub(
                LIVE_PREFIX_COLS + 1, /* keep a one-column right margin for wrapping */
            )
            .max(1);

        let style = user_message_style();
        let max_inner_width: usize = 80;
        let wrap_width = usize::from(wrap_width).min(max_inner_width).max(1);

        let (wrapped, joiner_before) = crate::wrapping::word_wrap_lines_with_joiners(
            self.message
                .lines()
                .map(|line| Line::from(line).style(style)),
            // Wrap algorithm matches textarea.rs.
            RtOptions::new(wrap_width).wrap_algorithm(textwrap::WrapAlgorithm::FirstFit),
        );

        let mut lines: Vec<Line<'static>> = Vec::new();
        let mut joins: Vec<Option<String>> = Vec::new();

        // Add subtle separator with role label (OpenCode-style)
        lines.push(theme::message_separator(width as usize, "You"));
        joins.push(None);

        // Prefix content with subtle indent
        let prefixed = prefix_lines(wrapped, "  ".into(), "  ".into());
        for (line, joiner) in prefixed.into_iter().zip(joiner_before) {
            lines.push(line);
            joins.push(joiner);
        }

        lines.push(Line::from(""));
        joins.push(None);

        TranscriptLinesWithJoiners {
            lines,
            joiner_before: joins,
        }
    }

    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        let width = usize::from(width.max(1));
        let prefix = "You: ";
        let subsequent = " ".repeat(prefix.len());
        let opts = RtOptions::new(width)
            .initial_indent(prefix.into())
            .subsequent_indent(subsequent.into());
        let lines = self.message.lines().map(std::string::ToString::to_string);
        word_wrap_lines(lines, opts)
    }
}
#[derive(Debug)]
pub(crate) struct ReasoningSummaryCell {
    _header: String,
    content: String,
    transcript_only: bool,
}

impl ReasoningSummaryCell {
    pub(crate) fn new(header: String, content: String, transcript_only: bool) -> Self {
        Self {
            _header: header,
            content,
            transcript_only,
        }
    }

    fn lines(&self, width: u16) -> Vec<Line<'static>> {
        self.lines_with_joiners(width).lines
    }

    fn lines_with_joiners(&self, width: u16) -> TranscriptLinesWithJoiners {
        let mut out_lines: Vec<Line<'static>> = Vec::new();
        let mut joiner_before: Vec<Option<String>> = Vec::new();

        // Add elegant thinking header
        out_lines.push(theme::thinking_header(width as usize));
        joiner_before.push(None);

        // Parse and style the content with elegant formatting
        let mut md_lines: Vec<Line<'static>> = Vec::new();
        append_markdown(
            &self.content,
            Some((width as usize).saturating_sub(6)), // Account for indent
            &mut md_lines,
        );

        // Apply reasoning style and elegant indentation
        let reasoning_style = theme::reasoning_style();
        for line in md_lines {
            let styled_spans: Vec<_> = line
                .spans
                .into_iter()
                .map(|span| span.patch_style(reasoning_style))
                .collect();

            // Add indented content with subtle bullet for non-empty lines
            if styled_spans.iter().all(|s| s.content.trim().is_empty()) {
                out_lines.push(Line::from(""));
            } else {
                let mut final_spans = vec![Span::styled("    ".to_string(), Style::default())];
                final_spans.extend(styled_spans);
                out_lines.push(Line::from(final_spans));
            }
            joiner_before.push(None);
        }

        // Add subtle trailing spacer
        out_lines.push(Line::from(""));
        joiner_before.push(None);

        TranscriptLinesWithJoiners {
            lines: out_lines,
            joiner_before,
        }
    }
}

impl HistoryCell for ReasoningSummaryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        if self.transcript_only {
            Vec::new()
        } else {
            self.lines(width)
        }
    }

    fn desired_height(&self, width: u16) -> u16 {
        if self.transcript_only {
            0
        } else {
            self.lines(width).len() as u16
        }
    }

    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.lines(width)
    }

    fn transcript_lines_with_joiners(&self, width: u16) -> TranscriptLinesWithJoiners {
        self.lines_with_joiners(width)
    }

    fn desired_transcript_height(&self, width: u16) -> u16 {
        self.lines(width).len() as u16
    }
}

#[derive(Debug)]
pub(crate) struct AgentMessageCell {
    lines: Vec<Line<'static>>,
    is_first_line: bool,
}

impl AgentMessageCell {
    pub(crate) fn new(lines: Vec<Line<'static>>, is_first_line: bool) -> Self {
        Self {
            lines,
            is_first_line,
        }
    }
}

impl HistoryCell for AgentMessageCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.transcript_lines_with_joiners(width).lines
    }

    fn transcript_lines_with_joiners(&self, width: u16) -> TranscriptLinesWithJoiners {
        use ratatui::style::Color;

        let mut out_lines: Vec<Line<'static>> = Vec::new();
        let mut joiner_before: Vec<Option<String>> = Vec::new();

        // Add separator with role label for first message block (OpenCode-style)
        if self.is_first_line {
            out_lines.push(theme::message_separator(width as usize, "Azure Codex"));
            joiner_before.push(None);
        }

        for line in &self.lines {
            let is_code_block_line = line.style.fg == Some(Color::Cyan);
            let initial_indent: Line<'static> = "  ".into();
            let subsequent_indent: Line<'static> = "  ".into();

            if is_code_block_line {
                let mut spans = initial_indent.spans;
                spans.extend(line.spans.iter().cloned());
                out_lines.push(Line::from(spans).style(line.style));
                joiner_before.push(None);
                continue;
            }

            let opts = RtOptions::new(width as usize)
                .initial_indent(initial_indent)
                .subsequent_indent(subsequent_indent.clone());
            let (wrapped, wrapped_joiners) =
                crate::wrapping::word_wrap_line_with_joiners(line, opts);
            for (l, j) in wrapped.into_iter().zip(wrapped_joiners) {
                out_lines.push(line_to_static(&l));
                joiner_before.push(j);
            }
        }

        TranscriptLinesWithJoiners {
            lines: out_lines,
            joiner_before,
        }
    }

    fn is_stream_continuation(&self) -> bool {
        !self.is_first_line
    }
}

#[derive(Debug)]
pub(crate) struct PlainHistoryCell {
    lines: Vec<Line<'static>>,
}

impl PlainHistoryCell {
    pub(crate) fn new(lines: Vec<Line<'static>>) -> Self {
        Self { lines }
    }
}

impl HistoryCell for PlainHistoryCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        self.lines.clone()
    }
}

#[derive(Debug)]
pub(crate) struct PrefixedWrappedHistoryCell {
    text: Text<'static>,
    initial_prefix: Line<'static>,
    subsequent_prefix: Line<'static>,
}

impl PrefixedWrappedHistoryCell {
    pub(crate) fn new(
        text: impl Into<Text<'static>>,
        initial_prefix: impl Into<Line<'static>>,
        subsequent_prefix: impl Into<Line<'static>>,
    ) -> Self {
        Self {
            text: text.into(),
            initial_prefix: initial_prefix.into(),
            subsequent_prefix: subsequent_prefix.into(),
        }
    }
}

impl HistoryCell for PrefixedWrappedHistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.transcript_lines_with_joiners(width).lines
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.display_lines(width).len() as u16
    }

    fn transcript_lines_with_joiners(&self, width: u16) -> TranscriptLinesWithJoiners {
        if width == 0 {
            return TranscriptLinesWithJoiners {
                lines: Vec::new(),
                joiner_before: Vec::new(),
            };
        }
        let opts = RtOptions::new(width.max(1) as usize)
            .initial_indent(self.initial_prefix.clone())
            .subsequent_indent(self.subsequent_prefix.clone());
        let (lines, joiner_before) =
            crate::wrapping::word_wrap_lines_with_joiners(&self.text, opts);
        TranscriptLinesWithJoiners {
            lines,
            joiner_before,
        }
    }
}

fn truncate_exec_snippet(full_cmd: &str) -> String {
    let mut snippet = match full_cmd.split_once('\n') {
        Some((first, _)) => format!("{first} ..."),
        None => full_cmd.to_string(),
    };
    snippet = truncate_text(&snippet, 80);
    snippet
}

fn exec_snippet(command: &[String]) -> String {
    let full_cmd = strip_bash_lc_and_escape(command);
    truncate_exec_snippet(&full_cmd)
}

pub fn new_approval_decision_cell(
    command: Vec<String>,
    decision: codex_core::protocol::ReviewDecision,
) -> Box<dyn HistoryCell> {
    use codex_core::protocol::ReviewDecision::*;

    // Helper to create styled approval/denial symbols using theme icons
    fn make_approved_symbol() -> Span<'static> {
        Span::from(format!("{} ", theme::icons::SUCCESS))
            .green()
            .bold()
    }
    fn make_denied_symbol() -> Span<'static> {
        Span::from(format!("{} ", theme::icons::ERROR)).red().bold()
    }

    let (symbol, summary): (Span<'static>, Vec<Span<'static>>) = match decision {
        Approved => {
            let snippet = Span::from(exec_snippet(&command)).dim();
            (
                make_approved_symbol(),
                vec![
                    "You ".into(),
                    "approved".bold(),
                    " codex to run ".into(),
                    snippet,
                    " this time".bold(),
                ],
            )
        }
        ApprovedExecpolicyAmendment { .. } => {
            let snippet = Span::from(exec_snippet(&command)).dim();
            (
                make_approved_symbol(),
                vec![
                    "You ".into(),
                    "approved".bold(),
                    " codex to run ".into(),
                    snippet,
                    " and applied the execpolicy amendment".bold(),
                ],
            )
        }
        ApprovedForSession => {
            let snippet = Span::from(exec_snippet(&command)).dim();
            (
                make_approved_symbol(),
                vec![
                    "You ".into(),
                    "approved".bold(),
                    " codex to run ".into(),
                    snippet,
                    " every time this session".bold(),
                ],
            )
        }
        Denied => {
            let snippet = Span::from(exec_snippet(&command)).dim();
            (
                make_denied_symbol(),
                vec![
                    "You ".into(),
                    "did not approve".bold(),
                    " codex to run ".into(),
                    snippet,
                ],
            )
        }
        Abort => {
            let snippet = Span::from(exec_snippet(&command)).dim();
            (
                make_denied_symbol(),
                vec![
                    "You ".into(),
                    "canceled".bold(),
                    " the request to run ".into(),
                    snippet,
                ],
            )
        }
    };

    Box::new(PrefixedWrappedHistoryCell::new(
        Line::from(summary),
        symbol,
        "  ",
    ))
}

/// Cyan history cell line showing the current review status.
pub(crate) fn new_review_status_line(message: String) -> PlainHistoryCell {
    PlainHistoryCell {
        lines: vec![Line::from(message.cyan())],
    }
}

#[derive(Debug)]
pub(crate) struct PatchHistoryCell {
    changes: HashMap<PathBuf, FileChange>,
    cwd: PathBuf,
}

impl HistoryCell for PatchHistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        create_diff_summary(&self.changes, &self.cwd, width as usize)
    }
}

#[derive(Debug)]
struct CompletedMcpToolCallWithImageOutput {
    _image: DynamicImage,
}
impl HistoryCell for CompletedMcpToolCallWithImageOutput {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        vec!["tool result (image output)".into()]
    }
}

pub(crate) const SESSION_HEADER_MAX_INNER_WIDTH: usize = 56; // Just an eyeballed value

pub(crate) fn card_inner_width(width: u16, max_inner_width: usize) -> Option<usize> {
    if width < 4 {
        return None;
    }
    let inner_width = std::cmp::min(width.saturating_sub(4) as usize, max_inner_width);
    Some(inner_width)
}

/// Render `lines` inside a border sized to the widest span in the content.
pub(crate) fn with_border(lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    with_border_internal(lines, None)
}

/// Render `lines` inside a border whose inner width is at least `inner_width`.
///
/// This is useful when callers have already clamped their content to a
/// specific width and want the border math centralized here instead of
/// duplicating padding logic in the TUI widgets themselves.
pub(crate) fn with_border_with_inner_width(
    lines: Vec<Line<'static>>,
    inner_width: usize,
) -> Vec<Line<'static>> {
    with_border_internal(lines, Some(inner_width))
}

fn with_border_internal(
    lines: Vec<Line<'static>>,
    forced_inner_width: Option<usize>,
) -> Vec<Line<'static>> {
    // Use rounded border characters from theme for consistency
    use crate::theme::rounded;

    let max_line_width = lines
        .iter()
        .map(|line| {
            line.iter()
                .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0);
    let content_width = forced_inner_width
        .unwrap_or(max_line_width)
        .max(max_line_width);

    let mut out = Vec::with_capacity(lines.len() + 2);
    let border_inner_width = content_width + 2;

    // Top border with rounded corners
    out.push(
        vec![
            format!(
                "{}{}{}",
                rounded::TL,
                rounded::H.repeat(border_inner_width),
                rounded::TR
            )
            .dim(),
        ]
        .into(),
    );

    // Content lines with side borders
    for line in lines.into_iter() {
        let used_width: usize = line
            .iter()
            .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
            .sum();
        let span_count = line.spans.len();
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(span_count + 4);
        spans.push(Span::from(format!("{} ", rounded::V)).dim());
        spans.extend(line.into_iter());
        if used_width < content_width {
            spans.push(Span::from(" ".repeat(content_width - used_width)).dim());
        }
        spans.push(Span::from(format!(" {}", rounded::V)).dim());
        out.push(Line::from(spans));
    }

    // Bottom border with rounded corners
    out.push(
        vec![
            format!(
                "{}{}{}",
                rounded::BL,
                rounded::H.repeat(border_inner_width),
                rounded::BR
            )
            .dim(),
        ]
        .into(),
    );

    out
}

/// Return the emoji followed by a hair space (U+200A).
/// Using only the hair space avoids excessive padding after the emoji while
/// still providing a small visual gap across terminals.
#[allow(dead_code)]
pub(crate) fn padded_emoji(emoji: &str) -> String {
    format!("{emoji}\u{200A}")
}

#[derive(Debug)]
struct TooltipHistoryCell {
    tip: &'static str,
}

impl TooltipHistoryCell {
    fn new(tip: &'static str) -> Self {
        Self { tip }
    }
}

impl HistoryCell for TooltipHistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let indent = "  ";
        let indent_width = UnicodeWidthStr::width(indent);
        let wrap_width = usize::from(width.max(1))
            .saturating_sub(indent_width)
            .max(1);
        let mut lines: Vec<Line<'static>> = Vec::new();
        append_markdown(
            &format!("**Tip:** {}", self.tip),
            Some(wrap_width),
            &mut lines,
        );

        prefix_lines(lines, indent.into(), indent.into())
    }
}

#[derive(Debug)]
pub struct SessionInfoCell(CompositeHistoryCell);

impl HistoryCell for SessionInfoCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.0.display_lines(width)
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.0.desired_height(width)
    }

    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.0.transcript_lines(width)
    }
}

pub(crate) fn new_session_info(
    config: &Config,
    requested_model: &str,
    event: SessionConfiguredEvent,
    is_first_event: bool,
    model_state: SharedModelState,
) -> SessionInfoCell {
    let SessionConfiguredEvent { model, .. } = event;
    // Header box rendered as history (so it appears at the very top)
    let header = SessionHeaderHistoryCell::new(
        model_state,
        config.cwd.clone(),
        CODEX_CLI_VERSION,
        config.azure_endpoint.clone(),
    );
    let mut parts: Vec<Box<dyn HistoryCell>> = vec![Box::new(header)];

    if is_first_event {
        // Elegant welcome message with styled commands
        let help_lines: Vec<Line<'static>> = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("  💡 ", Style::default().dim()),
                Span::styled(
                    "Describe a task or try these commands:".to_string(),
                    Style::default().dim().italic(),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("     ", Style::default()),
                Span::styled(
                    "/init".to_string(),
                    Style::default().fg(theme::COLOR_PRIMARY),
                ),
                Span::styled(
                    "      create project instructions".to_string(),
                    Style::default().dim(),
                ),
            ]),
            Line::from(vec![
                Span::styled("     ", Style::default()),
                Span::styled(
                    "/model".to_string(),
                    Style::default().fg(theme::COLOR_PRIMARY),
                ),
                Span::styled(
                    "     change model or reasoning".to_string(),
                    Style::default().dim(),
                ),
            ]),
            Line::from(vec![
                Span::styled("     ", Style::default()),
                Span::styled(
                    "/approvals".to_string(),
                    Style::default().fg(theme::COLOR_PRIMARY),
                ),
                Span::styled(
                    " configure auto-approval".to_string(),
                    Style::default().dim(),
                ),
            ]),
            Line::from(vec![
                Span::styled("     ", Style::default()),
                Span::styled(
                    "/review".to_string(),
                    Style::default().fg(theme::COLOR_PRIMARY),
                ),
                Span::styled(
                    "    review code changes".to_string(),
                    Style::default().dim(),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    "Ctrl+K".to_string(),
                    Style::default().fg(theme::COLOR_SECONDARY).bold(),
                ),
                Span::styled(" commands  ·  ".to_string(), Style::default().dim()),
                Span::styled(
                    "?".to_string(),
                    Style::default().fg(theme::COLOR_SECONDARY).bold(),
                ),
                Span::styled(" shortcuts  ·  ".to_string(), Style::default().dim()),
                Span::styled(
                    "Esc".to_string(),
                    Style::default().fg(theme::COLOR_SECONDARY).bold(),
                ),
                Span::styled(" quit".to_string(), Style::default().dim()),
            ]),
        ];

        parts.push(Box::new(PlainHistoryCell { lines: help_lines }));
    } else {
        if config.show_tooltips
            && let Some(tooltips) = tooltips::random_tooltip().map(TooltipHistoryCell::new)
        {
            parts.push(Box::new(tooltips));
        }
        if requested_model != model {
            let lines = vec![
                "model changed:".magenta().bold().into(),
                format!("requested: {requested_model}").into(),
                format!("used: {model}").into(),
            ];
            parts.push(Box::new(PlainHistoryCell { lines }));
        }
    }

    SessionInfoCell(CompositeHistoryCell { parts })
}

pub(crate) fn new_user_prompt(message: String) -> UserHistoryCell {
    UserHistoryCell { message }
}

/// Shared state for the session header that can be updated when the model changes.
/// This allows the session header displayed in history to always show the current model.
#[derive(Debug, Clone)]
pub(crate) struct SharedModelState {
    inner: Arc<RwLock<ModelStateInner>>,
}

#[derive(Debug, Clone)]
struct ModelStateInner {
    model: String,
    reasoning_effort: Option<ReasoningEffortConfig>,
}

impl SharedModelState {
    pub(crate) fn new(model: String, reasoning_effort: Option<ReasoningEffortConfig>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(ModelStateInner {
                model,
                reasoning_effort,
            })),
        }
    }

    /// Update the model and reasoning effort. Called when the model changes via /model.
    pub(crate) fn update(&self, model: String, reasoning_effort: Option<ReasoningEffortConfig>) {
        if let Ok(mut guard) = self.inner.write() {
            guard.model = model;
            guard.reasoning_effort = reasoning_effort;
        }
    }

    pub(crate) fn get(&self) -> (String, Option<ReasoningEffortConfig>) {
        if let Ok(guard) = self.inner.read() {
            (guard.model.clone(), guard.reasoning_effort)
        } else {
            // Fallback if lock is poisoned
            ("unknown".to_string(), None)
        }
    }
}

#[derive(Debug)]
struct SessionHeaderHistoryCell {
    version: &'static str,
    model_state: SharedModelState,
    directory: PathBuf,
    azure_endpoint: Option<String>,
}

impl SessionHeaderHistoryCell {
    fn new(
        model_state: SharedModelState,
        directory: PathBuf,
        version: &'static str,
        azure_endpoint: Option<String>,
    ) -> Self {
        Self {
            version,
            model_state,
            directory,
            azure_endpoint,
        }
    }

    fn format_directory(&self, max_width: Option<usize>) -> String {
        Self::format_directory_inner(&self.directory, max_width)
    }

    fn format_directory_inner(directory: &Path, max_width: Option<usize>) -> String {
        let formatted = if let Some(rel) = relativize_to_home(directory) {
            if rel.as_os_str().is_empty() {
                "~".to_string()
            } else {
                format!("~{}{}", std::path::MAIN_SEPARATOR, rel.display())
            }
        } else {
            directory.display().to_string()
        };

        if let Some(max_width) = max_width {
            if max_width == 0 {
                return String::new();
            }
            if UnicodeWidthStr::width(formatted.as_str()) > max_width {
                return crate::text_formatting::center_truncate_path(&formatted, max_width);
            }
        }

        formatted
    }

    fn reasoning_label(effort: Option<ReasoningEffortConfig>) -> Option<&'static str> {
        effort.map(|effort| match effort {
            ReasoningEffortConfig::Minimal => "minimal",
            ReasoningEffortConfig::Low => "low",
            ReasoningEffortConfig::Medium => "medium",
            ReasoningEffortConfig::High => "high",
            ReasoningEffortConfig::XHigh => "xhigh",
            ReasoningEffortConfig::None => "none",
        })
    }
}

impl HistoryCell for SessionHeaderHistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let Some(inner_width) = card_inner_width(width, SESSION_HEADER_MAX_INNER_WIDTH) else {
            return Vec::new();
        };

        let make_row = |spans: Vec<Span<'static>>| Line::from(spans);

        // Elegant title with logo-style prefix
        let title_spans: Vec<Span<'static>> = vec![
            Span::styled("▸ ", Style::default().fg(theme::COLOR_PRIMARY).bold()),
            Span::styled(
                codex_branding::APP_NAME.to_string(),
                Style::default().fg(theme::COLOR_PRIMARY).bold(),
            ),
            Span::styled(format!(" v{}", self.version), Style::default().dim()),
        ];

        // Read current model from shared state (updated when model changes)
        let (model, reasoning_effort) = self.model_state.get();
        let reasoning_label = Self::reasoning_label(reasoning_effort);

        const CHANGE_MODEL_HINT_COMMAND: &str = "/model";
        const CHANGE_MODEL_HINT_EXPLANATION: &str = " to change";
        const DIR_LABEL: &str = "directory:";
        let label_width = DIR_LABEL.len();
        let model_label = format!(
            "{model_label:<label_width$}",
            model_label = "model:",
            label_width = label_width
        );
        let mut model_spans: Vec<Span<'static>> = vec![
            Span::from(format!("{model_label} ")).dim(),
            Span::from(model),
        ];
        if let Some(reasoning) = reasoning_label {
            model_spans.push(Span::from(" "));
            model_spans.push(Span::from(reasoning));
        }

        // Calculate current width of model spans
        let model_base_width: usize = model_spans
            .iter()
            .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
            .sum();

        // Only add the "/model to change" hint if there's enough room
        let hint_full_width =
            3 + CHANGE_MODEL_HINT_COMMAND.len() + CHANGE_MODEL_HINT_EXPLANATION.len(); // "   /model to change"
        let hint_short_width = 3 + CHANGE_MODEL_HINT_COMMAND.len(); // "   /model"

        if model_base_width + hint_full_width <= inner_width {
            // Full hint fits
            model_spans.push("   ".dim());
            model_spans.push(CHANGE_MODEL_HINT_COMMAND.cyan());
            model_spans.push(CHANGE_MODEL_HINT_EXPLANATION.dim());
        } else if model_base_width + hint_short_width <= inner_width {
            // Only short hint fits
            model_spans.push("   ".dim());
            model_spans.push(CHANGE_MODEL_HINT_COMMAND.cyan());
        }
        // Otherwise, omit the hint entirely for very narrow terminals

        // Optional endpoint line for Azure
        let endpoint_spans: Option<Vec<Span<'static>>> = self.azure_endpoint.as_ref().map(|ep| {
            let endpoint_label = format!(
                "{endpoint_label:<label_width$}",
                endpoint_label = "endpoint:"
            );
            let endpoint_prefix = format!("{endpoint_label} ");
            let endpoint_prefix_width = UnicodeWidthStr::width(endpoint_prefix.as_str());
            let endpoint_max_width = inner_width.saturating_sub(endpoint_prefix_width);
            // Extract just the hostname from the endpoint URL for brevity
            let display_endpoint = ep
                .strip_prefix("https://")
                .or_else(|| ep.strip_prefix("http://"))
                .unwrap_or(ep);
            let truncated = if UnicodeWidthStr::width(display_endpoint) > endpoint_max_width {
                crate::text_formatting::center_truncate_path(display_endpoint, endpoint_max_width)
            } else {
                display_endpoint.to_string()
            };
            vec![
                Span::from(endpoint_prefix).dim(),
                Span::from(truncated).cyan(),
            ]
        });

        let dir_label = format!("{DIR_LABEL:<label_width$}");
        let dir_prefix = format!("{dir_label} ");
        let dir_prefix_width = UnicodeWidthStr::width(dir_prefix.as_str());
        let dir_max_width = inner_width.saturating_sub(dir_prefix_width);
        let dir = self.format_directory(Some(dir_max_width));
        let dir_spans = vec![Span::from(dir_prefix).dim(), Span::from(dir)];

        let mut lines = vec![
            make_row(title_spans),
            make_row(Vec::new()),
            make_row(model_spans),
        ];
        if let Some(ep_spans) = endpoint_spans {
            lines.push(make_row(ep_spans));
        }
        lines.push(make_row(dir_spans));

        with_border(lines)
    }
}

#[derive(Debug)]
pub(crate) struct CompositeHistoryCell {
    parts: Vec<Box<dyn HistoryCell>>,
}

impl CompositeHistoryCell {
    pub(crate) fn new(parts: Vec<Box<dyn HistoryCell>>) -> Self {
        Self { parts }
    }
}

impl HistoryCell for CompositeHistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut out: Vec<Line<'static>> = Vec::new();
        let mut first = true;
        for part in &self.parts {
            let mut lines = part.display_lines(width);
            if !lines.is_empty() {
                if !first {
                    out.push(Line::from(""));
                }
                out.append(&mut lines);
                first = false;
            }
        }
        out
    }
}

#[derive(Debug)]
pub(crate) struct McpToolCallCell {
    call_id: String,
    invocation: McpInvocation,
    start_time: Instant,
    duration: Option<Duration>,
    result: Option<Result<mcp_types::CallToolResult, String>>,
    animations_enabled: bool,
}

impl McpToolCallCell {
    pub(crate) fn new(
        call_id: String,
        invocation: McpInvocation,
        animations_enabled: bool,
    ) -> Self {
        Self {
            call_id,
            invocation,
            start_time: Instant::now(),
            duration: None,
            result: None,
            animations_enabled,
        }
    }

    pub(crate) fn call_id(&self) -> &str {
        &self.call_id
    }

    pub(crate) fn complete(
        &mut self,
        duration: Duration,
        result: Result<mcp_types::CallToolResult, String>,
    ) -> Option<Box<dyn HistoryCell>> {
        let image_cell = try_new_completed_mcp_tool_call_with_image_output(&result)
            .map(|cell| Box::new(cell) as Box<dyn HistoryCell>);
        self.duration = Some(duration);
        self.result = Some(result);
        image_cell
    }

    fn success(&self) -> Option<bool> {
        match self.result.as_ref() {
            Some(Ok(result)) => Some(!result.is_error.unwrap_or(false)),
            Some(Err(_)) => Some(false),
            None => None,
        }
    }

    pub(crate) fn mark_failed(&mut self) {
        let elapsed = self.start_time.elapsed();
        self.duration = Some(elapsed);
        self.result = Some(Err("interrupted".to_string()));
    }

    fn render_content_block(block: &mcp_types::ContentBlock, width: usize) -> String {
        match block {
            mcp_types::ContentBlock::TextContent(text) => {
                format_and_truncate_tool_result(&text.text, TOOL_CALL_MAX_LINES, width)
            }
            mcp_types::ContentBlock::ImageContent(_) => "<image content>".to_string(),
            mcp_types::ContentBlock::AudioContent(_) => "<audio content>".to_string(),
            mcp_types::ContentBlock::EmbeddedResource(resource) => {
                let uri = match &resource.resource {
                    EmbeddedResourceResource::TextResourceContents(text) => text.uri.clone(),
                    EmbeddedResourceResource::BlobResourceContents(blob) => blob.uri.clone(),
                };
                format!("embedded resource: {uri}")
            }
            mcp_types::ContentBlock::ResourceLink(ResourceLink { uri, .. }) => {
                format!("link: {uri}")
            }
        }
    }
}

impl HistoryCell for McpToolCallCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        let status = self.success();

        // Elegant status indicators with icons (OpenCode-style)
        let (bullet, header_text, status_style) = match status {
            Some(true) => (
                Span::styled("🔌 ", Style::default().fg(theme::COLOR_SUCCESS).dim()),
                "Called",
                Style::default().fg(theme::COLOR_SUCCESS).dim(),
            ),
            Some(false) => (
                Span::styled("🔌 ", Style::default().fg(theme::COLOR_ERROR).bold()),
                "Failed",
                Style::default().fg(theme::COLOR_ERROR),
            ),
            None => (
                spinner(Some(self.start_time), self.animations_enabled),
                "Calling",
                Style::default().fg(theme::COLOR_PRIMARY).bold(),
            ),
        };

        let invocation_line = line_to_static(&format_mcp_invocation(self.invocation.clone()));
        let mut compact_spans = vec![
            bullet.clone(),
            Span::styled(format!("{header_text} "), status_style),
        ];
        let mut compact_header = Line::from(compact_spans.clone());
        let reserved = compact_header.width();

        let inline_invocation =
            invocation_line.width() <= (width as usize).saturating_sub(reserved);

        if inline_invocation {
            compact_header.extend(invocation_line.spans.clone());
            lines.push(compact_header);
        } else {
            compact_spans.pop(); // drop trailing space for standalone header
            lines.push(Line::from(compact_spans));

            let opts = RtOptions::new((width as usize).saturating_sub(4))
                .initial_indent("".into())
                .subsequent_indent("    ".into());
            let wrapped = word_wrap_line(&invocation_line, opts);
            let body_lines: Vec<Line<'static>> = wrapped.iter().map(line_to_static).collect();
            // Use rounded tree connector for elegant appearance
            lines.extend(prefix_lines(
                body_lines,
                format!("  {} ", theme::rounded::BL).dim(),
                "    ".into(),
            ));
        }

        let mut detail_lines: Vec<Line<'static>> = Vec::new();
        // Reserve four columns for the tree prefix ("  └ "/"    ") and ensure the wrapper still has at least one cell to work with.
        let detail_wrap_width = (width as usize).saturating_sub(4).max(1);

        if let Some(result) = &self.result {
            match result {
                Ok(mcp_types::CallToolResult { content, .. }) => {
                    if !content.is_empty() {
                        for block in content {
                            let text = Self::render_content_block(block, detail_wrap_width);
                            for segment in text.split('\n') {
                                let line = Line::from(segment.to_string().dim());
                                let wrapped = word_wrap_line(
                                    &line,
                                    RtOptions::new(detail_wrap_width)
                                        .initial_indent("".into())
                                        .subsequent_indent("    ".into()),
                                );
                                detail_lines.extend(wrapped.iter().map(line_to_static));
                            }
                        }
                    }
                }
                Err(err) => {
                    let err_text = format_and_truncate_tool_result(
                        &format!("Error: {err}"),
                        TOOL_CALL_MAX_LINES,
                        width as usize,
                    );
                    let err_line = Line::from(err_text.dim());
                    let wrapped = word_wrap_line(
                        &err_line,
                        RtOptions::new(detail_wrap_width)
                            .initial_indent("".into())
                            .subsequent_indent("    ".into()),
                    );
                    detail_lines.extend(wrapped.iter().map(line_to_static));
                }
            }
        }

        if !detail_lines.is_empty() {
            // Use rounded tree connector for elegant appearance
            let initial_prefix: Span<'static> = if inline_invocation {
                Span::from(format!("  {} ", theme::rounded::BL)).dim()
            } else {
                "    ".into()
            };
            lines.extend(prefix_lines(detail_lines, initial_prefix, "    ".into()));
        }

        lines
    }
}

pub(crate) fn new_active_mcp_tool_call(
    call_id: String,
    invocation: McpInvocation,
    animations_enabled: bool,
) -> McpToolCallCell {
    McpToolCallCell::new(call_id, invocation, animations_enabled)
}

pub(crate) fn new_web_search_call(query: String) -> PrefixedWrappedHistoryCell {
    let text: Text<'static> = Line::from(vec!["Searched".bold(), " ".into(), query.into()]).into();
    PrefixedWrappedHistoryCell::new(text, "• ".dim(), "  ")
}

/// If the first content is an image, return a new cell with the image.
/// TODO(rgwood-dd): Handle images properly even if they're not the first result.
fn try_new_completed_mcp_tool_call_with_image_output(
    result: &Result<mcp_types::CallToolResult, String>,
) -> Option<CompletedMcpToolCallWithImageOutput> {
    match result {
        Ok(mcp_types::CallToolResult { content, .. }) => {
            if let Some(mcp_types::ContentBlock::ImageContent(image)) = content.first() {
                let raw_data = match base64::engine::general_purpose::STANDARD.decode(&image.data) {
                    Ok(data) => data,
                    Err(e) => {
                        error!("Failed to decode image data: {e}");
                        return None;
                    }
                };
                let reader = match ImageReader::new(Cursor::new(raw_data)).with_guessed_format() {
                    Ok(reader) => reader,
                    Err(e) => {
                        error!("Failed to guess image format: {e}");
                        return None;
                    }
                };

                let image = match reader.decode() {
                    Ok(image) => image,
                    Err(e) => {
                        error!("Image decoding failed: {e}");
                        return None;
                    }
                };

                Some(CompletedMcpToolCallWithImageOutput { _image: image })
            } else {
                None
            }
        }
        _ => None,
    }
}

#[allow(clippy::disallowed_methods)]
pub(crate) fn new_warning_event(message: String) -> PrefixedWrappedHistoryCell {
    PrefixedWrappedHistoryCell::new(message.yellow(), "⚠ ".yellow(), "  ")
}

#[derive(Debug)]
pub(crate) struct DeprecationNoticeCell {
    summary: String,
    details: Option<String>,
}

pub(crate) fn new_deprecation_notice(
    summary: String,
    details: Option<String>,
) -> DeprecationNoticeCell {
    DeprecationNoticeCell { summary, details }
}

impl HistoryCell for DeprecationNoticeCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        lines.push(vec!["⚠ ".red().bold(), self.summary.clone().red()].into());

        let wrap_width = width.saturating_sub(4).max(1) as usize;

        if let Some(details) = &self.details {
            let line = textwrap::wrap(details, wrap_width)
                .into_iter()
                .map(|s| s.to_string().dim().into())
                .collect::<Vec<_>>();
            lines.extend(line);
        }

        lines
    }
}

/// Render a summary of configured MCP servers from the current `Config`.
pub(crate) fn empty_mcp_output() -> PlainHistoryCell {
    let lines: Vec<Line<'static>> = vec![
        "/mcp".magenta().into(),
        "".into(),
        vec!["🔌  ".into(), "MCP Tools".bold()].into(),
        "".into(),
        "  • No MCP servers configured.".italic().into(),
        Line::from(vec![
            "    See the ".into(),
            "\u{1b}]8;;https://github.com/Arthur742Ramos/azure-codex/blob/main/docs/config.md#mcp_servers\u{7}MCP docs\u{1b}]8;;\u{7}".underlined(),
            " to configure them.".into(),
        ])
        .style(Style::default().add_modifier(Modifier::DIM)),
    ];

    PlainHistoryCell { lines }
}

/// Render MCP tools grouped by connection using the fully-qualified tool names.
pub(crate) fn new_mcp_tools_output(
    config: &Config,
    tools: HashMap<String, mcp_types::Tool>,
    resources: HashMap<String, Vec<Resource>>,
    resource_templates: HashMap<String, Vec<ResourceTemplate>>,
    auth_statuses: &HashMap<String, McpAuthStatus>,
) -> PlainHistoryCell {
    let mut lines: Vec<Line<'static>> = vec![
        "/mcp".magenta().into(),
        "".into(),
        vec!["🔌  ".into(), "MCP Tools".bold()].into(),
        "".into(),
    ];

    if tools.is_empty() {
        lines.push("  • No MCP tools available.".italic().into());
        lines.push("".into());
        return PlainHistoryCell { lines };
    }

    let mut servers: Vec<_> = config.mcp_servers.iter().collect();
    servers.sort_by(|(a, _), (b, _)| a.cmp(b));

    for (server, cfg) in servers {
        let prefix = format!("mcp__{server}__");
        let mut names: Vec<String> = tools
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .map(|k| k[prefix.len()..].to_string())
            .collect();
        names.sort();

        let auth_status = auth_statuses
            .get(server.as_str())
            .copied()
            .unwrap_or(McpAuthStatus::Unsupported);
        let mut header: Vec<Span<'static>> = vec!["  • ".into(), server.clone().into()];
        if !cfg.enabled {
            header.push(" ".into());
            header.push("(disabled)".red());
            lines.push(header.into());
            lines.push(Line::from(""));
            continue;
        }
        lines.push(header.into());
        lines.push(vec!["    • Status: ".into(), "enabled".green()].into());
        lines.push(vec!["    • Auth: ".into(), auth_status.to_string().into()].into());

        match &cfg.transport {
            McpServerTransportConfig::Stdio {
                command,
                args,
                env,
                env_vars,
                cwd,
            } => {
                let args_suffix = if args.is_empty() {
                    String::new()
                } else {
                    format!(" {}", args.join(" "))
                };
                let cmd_display = format!("{command}{args_suffix}");
                lines.push(vec!["    • Command: ".into(), cmd_display.into()].into());

                if let Some(cwd) = cwd.as_ref() {
                    lines.push(vec!["    • Cwd: ".into(), cwd.display().to_string().into()].into());
                }

                let env_display = format_env_display(env.as_ref(), env_vars);
                if env_display != "-" {
                    lines.push(vec!["    • Env: ".into(), env_display.into()].into());
                }
            }
            McpServerTransportConfig::StreamableHttp {
                url,
                http_headers,
                env_http_headers,
                ..
            } => {
                lines.push(vec!["    • URL: ".into(), url.clone().into()].into());
                if let Some(headers) = http_headers.as_ref()
                    && !headers.is_empty()
                {
                    let mut pairs: Vec<_> = headers.iter().collect();
                    pairs.sort_by(|(a, _), (b, _)| a.cmp(b));
                    let display = pairs
                        .into_iter()
                        .map(|(name, _)| format!("{name}=*****"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    lines.push(vec!["    • HTTP headers: ".into(), display.into()].into());
                }
                if let Some(headers) = env_http_headers.as_ref()
                    && !headers.is_empty()
                {
                    let mut pairs: Vec<_> = headers.iter().collect();
                    pairs.sort_by(|(a, _), (b, _)| a.cmp(b));
                    let display = pairs
                        .into_iter()
                        .map(|(name, var)| format!("{name}={var}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    lines.push(vec!["    • Env HTTP headers: ".into(), display.into()].into());
                }
            }
        }

        if names.is_empty() {
            lines.push("    • Tools: (none)".into());
        } else {
            lines.push(vec!["    • Tools: ".into(), names.join(", ").into()].into());
        }

        let server_resources: Vec<Resource> =
            resources.get(server.as_str()).cloned().unwrap_or_default();
        if server_resources.is_empty() {
            lines.push("    • Resources: (none)".into());
        } else {
            let mut spans: Vec<Span<'static>> = vec!["    • Resources: ".into()];

            for (idx, resource) in server_resources.iter().enumerate() {
                if idx > 0 {
                    spans.push(", ".into());
                }

                let label = resource.title.as_ref().unwrap_or(&resource.name);
                spans.push(label.clone().into());
                spans.push(" ".into());
                spans.push(format!("({})", resource.uri).dim());
            }

            lines.push(spans.into());
        }

        let server_templates: Vec<ResourceTemplate> = resource_templates
            .get(server.as_str())
            .cloned()
            .unwrap_or_default();
        if server_templates.is_empty() {
            lines.push("    • Resource templates: (none)".into());
        } else {
            let mut spans: Vec<Span<'static>> = vec!["    • Resource templates: ".into()];

            for (idx, template) in server_templates.iter().enumerate() {
                if idx > 0 {
                    spans.push(", ".into());
                }

                let label = template.title.as_ref().unwrap_or(&template.name);
                spans.push(label.clone().into());
                spans.push(" ".into());
                spans.push(format!("({})", template.uri_template).dim());
            }

            lines.push(spans.into());
        }

        lines.push(Line::from(""));
    }

    PlainHistoryCell { lines }
}
pub(crate) fn new_info_event(message: String, hint: Option<String>) -> PlainHistoryCell {
    let mut line = vec!["• ".dim(), message.into()];
    if let Some(hint) = hint {
        line.push(" ".into());
        line.push(hint.dark_gray());
    }
    let lines: Vec<Line<'static>> = vec![line.into()];
    PlainHistoryCell { lines }
}

/// Creates a compact bordered card showing the updated model configuration.
/// This is displayed when the user changes the model via /model command,
/// providing visual feedback that mirrors the session header format.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct ModelChangedCell {
    model: String,
    reasoning_effort: Option<ReasoningEffortConfig>,
}

#[allow(dead_code)]
impl ModelChangedCell {
    pub(crate) fn new(model: String, reasoning_effort: Option<ReasoningEffortConfig>) -> Self {
        Self {
            model,
            reasoning_effort,
        }
    }

    fn reasoning_label(effort: Option<ReasoningEffortConfig>) -> Option<&'static str> {
        effort.map(|effort| match effort {
            ReasoningEffortConfig::Minimal => "minimal",
            ReasoningEffortConfig::Low => "low",
            ReasoningEffortConfig::Medium => "medium",
            ReasoningEffortConfig::High => "high",
            ReasoningEffortConfig::XHigh => "xhigh",
            ReasoningEffortConfig::None => "none",
        })
    }
}

impl HistoryCell for ModelChangedCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let Some(inner_width) = card_inner_width(width, SESSION_HEADER_MAX_INNER_WIDTH) else {
            return Vec::new();
        };

        let mut model_spans: Vec<Span<'static>> = vec![
            Span::from("model:     ").dim(),
            Span::from(self.model.clone()),
        ];
        if let Some(reasoning) = Self::reasoning_label(self.reasoning_effort) {
            model_spans.push(Span::from(" "));
            model_spans.push(Span::from(reasoning));
        }

        let lines = vec![Line::from(model_spans)];

        with_border_with_inner_width(lines, inner_width)
    }
}

/// Creates a model changed card to display when the model is updated via /model.
#[allow(dead_code)]
pub(crate) fn new_model_changed_card(
    model: String,
    reasoning_effort: Option<ReasoningEffortConfig>,
) -> ModelChangedCell {
    ModelChangedCell::new(model, reasoning_effort)
}

pub(crate) fn new_error_event(message: String) -> PlainHistoryCell {
    // Use a hair space (U+200A) to create a subtle, near-invisible separation
    // before the text. VS16 is intentionally omitted to keep spacing tighter
    // in terminals like Ghostty.
    let lines: Vec<Line<'static>> = vec![vec![format!("■ {message}").red()].into()];
    PlainHistoryCell { lines }
}

/// Render a user‑friendly plan update styled like a checkbox todo list.
pub(crate) fn new_plan_update(update: UpdatePlanArgs) -> PlanUpdateCell {
    let UpdatePlanArgs { explanation, plan } = update;
    PlanUpdateCell { explanation, plan }
}

#[derive(Debug)]
pub(crate) struct PlanUpdateCell {
    explanation: Option<String>,
    plan: Vec<PlanItemArg>,
}

impl HistoryCell for PlanUpdateCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let render_note = |text: &str| -> Vec<Line<'static>> {
            let wrap_width = width.saturating_sub(4).max(1) as usize;
            textwrap::wrap(text, wrap_width)
                .into_iter()
                .map(|s| s.to_string().dim().italic().into())
                .collect()
        };

        let render_step = |status: &StepStatus, text: &str| -> Vec<Line<'static>> {
            let (box_str, box_prefix, step_style) = match status {
                StepStatus::Completed => (
                    "✔ ",
                    "✔ ".green().dim(),
                    Style::default().crossed_out().dim(),
                ),
                StepStatus::InProgress => {
                    ("▣ ", "▣ ".cyan().bold(), Style::default().cyan().bold())
                }
                StepStatus::Pending => ("□ ", "□ ".dim(), Style::default().dim()),
            };
            let wrap_width = (width as usize)
                .saturating_sub(4)
                .saturating_sub(box_str.width())
                .max(1);
            let parts = textwrap::wrap(text, wrap_width);
            let step_text = parts
                .into_iter()
                .map(|s| s.to_string().set_style(step_style).into())
                .collect();
            prefix_lines(step_text, box_prefix, "  ".into())
        };

        let completed = self
            .plan
            .iter()
            .filter(|item| matches!(&item.status, StepStatus::Completed))
            .count();
        let total = self.plan.len();

        let mut header: Vec<Span<'static>> = vec!["• ".dim(), "Updated Plan".bold()];
        if total > 0 {
            header.push(" ".into());
            header.push(format!("({completed}/{total})").dim());
        }

        let mut lines: Vec<Line<'static>> = vec![];
        lines.push(header.into());

        let mut indented_lines = vec![];
        let note = self
            .explanation
            .as_ref()
            .map(|s| s.trim())
            .filter(|t| !t.is_empty());
        if let Some(expl) = note {
            indented_lines.extend(render_note(expl));
        };

        if self.plan.is_empty() {
            indented_lines.push(Line::from("(no steps provided)".dim().italic()));
        } else {
            for PlanItemArg { step, status } in self.plan.iter() {
                indented_lines.extend(render_step(status, step));
            }
        }
        lines.extend(prefix_lines(indented_lines, "  └ ".dim(), "    ".into()));

        lines
    }
}

/// Create a new `PendingPatch` cell that lists the file‑level summary of
/// a proposed patch. The summary lines should already be formatted (e.g.
/// "A path/to/file.rs").
pub(crate) fn new_patch_event(
    changes: HashMap<PathBuf, FileChange>,
    cwd: &Path,
) -> PatchHistoryCell {
    PatchHistoryCell {
        changes,
        cwd: cwd.to_path_buf(),
    }
}

pub(crate) fn new_patch_apply_failure(stderr: String) -> PlainHistoryCell {
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Failure title
    lines.push(Line::from("✘ Failed to apply patch".magenta().bold()));

    if !stderr.trim().is_empty() {
        let output = output_lines(
            Some(&CommandOutput {
                exit_code: 1,
                formatted_output: String::new(),
                aggregated_output: stderr,
            }),
            OutputLinesParams {
                line_limit: TOOL_CALL_MAX_LINES,
                only_err: true,
                include_angle_pipe: true,
                include_prefix: true,
            },
        );
        lines.extend(output.lines);
    }

    PlainHistoryCell { lines }
}

pub(crate) fn new_view_image_tool_call(path: PathBuf, cwd: &Path) -> PlainHistoryCell {
    let display_path = display_path_for(&path, cwd);

    let lines: Vec<Line<'static>> = vec![
        vec!["• ".dim(), "Viewed Image".bold()].into(),
        vec!["  └ ".dim(), display_path.dim()].into(),
    ];

    PlainHistoryCell { lines }
}

pub(crate) fn new_reasoning_summary_block(full_reasoning_buffer: String) -> Box<dyn HistoryCell> {
    // Experimental format is following:
    // ** header **
    //
    // reasoning summary
    //
    // So we need to strip header from reasoning summary
    let full_reasoning_buffer = full_reasoning_buffer.trim();
    if let Some(open) = full_reasoning_buffer.find("**") {
        let after_open = &full_reasoning_buffer[(open + 2)..];
        if let Some(close) = after_open.find("**") {
            let after_close_idx = open + 2 + close + 2;
            // if we don't have anything beyond `after_close_idx`
            // then we don't have a summary to inject into history
            if after_close_idx < full_reasoning_buffer.len() {
                let header_buffer = full_reasoning_buffer[..after_close_idx].to_string();
                let summary_buffer = full_reasoning_buffer[after_close_idx..].to_string();
                return Box::new(ReasoningSummaryCell::new(
                    header_buffer,
                    summary_buffer,
                    false,
                ));
            }
        }
    }
    // For raw thinking (e.g., Claude extended thinking), show a condensed preview
    // instead of hiding. Use false for transcript_only to display in the UI.
    Box::new(ReasoningSummaryCell::new(
        "".to_string(),
        full_reasoning_buffer.to_string(),
        false,
    ))
}

#[derive(Debug)]
pub struct FinalMessageSeparator {
    elapsed_seconds: Option<u64>,
}
impl FinalMessageSeparator {
    pub(crate) fn new(elapsed_seconds: Option<u64>) -> Self {
        Self { elapsed_seconds }
    }
}
impl HistoryCell for FinalMessageSeparator {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let elapsed_seconds = self
            .elapsed_seconds
            .map(super::status_indicator_widget::fmt_elapsed_compact);
        if let Some(elapsed_seconds) = elapsed_seconds {
            let worked_for = format!("─ Worked for {elapsed_seconds} ─");
            let worked_for_width = worked_for.width();
            vec![
                Line::from_iter([
                    worked_for,
                    "─".repeat((width as usize).saturating_sub(worked_for_width)),
                ])
                .dim(),
            ]
        } else {
            vec![Line::from_iter(["─".repeat(width as usize).dim()])]
        }
    }
}

fn format_mcp_invocation<'a>(invocation: McpInvocation) -> Line<'a> {
    let args_str = invocation
        .arguments
        .as_ref()
        .map(|v: &serde_json::Value| {
            // Use compact form to keep things short but readable.
            serde_json::to_string(v).unwrap_or_else(|_| v.to_string())
        })
        .unwrap_or_default();

    let invocation_spans = vec![
        invocation.server.clone().cyan(),
        ".".into(),
        invocation.tool.cyan(),
        "(".into(),
        args_str.dim(),
        ")".into(),
    ];
    invocation_spans.into()
}

/// History cell that displays an update available notification banner.
#[cfg(not(debug_assertions))]
#[derive(Debug)]
pub struct UpdateAvailableHistoryCell {
    latest_version: String,
    update_action: Option<crate::update_action::UpdateAction>,
}

#[cfg(not(debug_assertions))]
impl UpdateAvailableHistoryCell {
    pub fn new(
        latest_version: String,
        update_action: Option<crate::update_action::UpdateAction>,
    ) -> Self {
        Self {
            latest_version,
            update_action,
        }
    }
}

#[cfg(not(debug_assertions))]
impl HistoryCell for UpdateAvailableHistoryCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let current_version = env!("CARGO_PKG_VERSION");
        let mut lines = vec![Line::from(vec![
            padded_emoji("  ✨").bold().cyan(),
            "Update available: ".bold(),
            format!("{current_version} → {}", self.latest_version).dim(),
        ])];

        if let Some(action) = &self.update_action {
            lines.push(Line::from(vec![
                "     Run: ".dim(),
                action.command_str().cyan(),
            ]));
        }

        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec_cell::CommandOutput;
    use crate::exec_cell::ExecCall;
    use crate::exec_cell::ExecCell;
    use codex_core::config::Config;
    use codex_core::config::ConfigBuilder;
    use codex_core::config::types::McpServerConfig;
    use codex_core::config::types::McpServerTransportConfig;
    use codex_core::protocol::McpAuthStatus;
    use codex_protocol::parse_command::ParsedCommand;
    use dirs::home_dir;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::collections::HashMap;

    use codex_core::protocol::ExecCommandSource;
    use mcp_types::CallToolResult;
    use mcp_types::ContentBlock;
    use mcp_types::TextContent;
    use mcp_types::Tool;
    use mcp_types::ToolInputSchema;
    async fn test_config() -> Config {
        let codex_home = std::env::temp_dir();
        ConfigBuilder::default()
            .codex_home(codex_home.clone())
            .build()
            .await
            .expect("config")
    }

    fn render_lines(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }

    fn render_transcript(cell: &dyn HistoryCell) -> Vec<String> {
        render_lines(&cell.transcript_lines(u16::MAX))
    }

    #[tokio::test]
    async fn mcp_tools_output_masks_sensitive_values() {
        let mut config = test_config().await;
        let mut env = HashMap::new();
        env.insert("TOKEN".to_string(), "secret".to_string());
        let stdio_config = McpServerConfig {
            transport: McpServerTransportConfig::Stdio {
                command: "docs-server".to_string(),
                args: vec![],
                env: Some(env),
                env_vars: vec!["APP_TOKEN".to_string()],
                cwd: None,
            },
            enabled: true,
            startup_timeout_sec: None,
            tool_timeout_sec: None,
            enabled_tools: None,
            disabled_tools: None,
        };
        config.mcp_servers.insert("docs".to_string(), stdio_config);

        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer secret".to_string());
        let mut env_headers = HashMap::new();
        env_headers.insert("X-API-Key".to_string(), "API_KEY_ENV".to_string());
        let http_config = McpServerConfig {
            transport: McpServerTransportConfig::StreamableHttp {
                url: "https://example.com/mcp".to_string(),
                bearer_token_env_var: Some("MCP_TOKEN".to_string()),
                http_headers: Some(headers),
                env_http_headers: Some(env_headers),
            },
            enabled: true,
            startup_timeout_sec: None,
            tool_timeout_sec: None,
            enabled_tools: None,
            disabled_tools: None,
        };
        config.mcp_servers.insert("http".to_string(), http_config);

        let mut tools: HashMap<String, Tool> = HashMap::new();
        tools.insert(
            "mcp__docs__list".to_string(),
            Tool {
                annotations: None,
                description: None,
                input_schema: ToolInputSchema {
                    properties: None,
                    required: None,
                    r#type: "object".to_string(),
                },
                name: "list".to_string(),
                output_schema: None,
                title: None,
            },
        );
        tools.insert(
            "mcp__http__ping".to_string(),
            Tool {
                annotations: None,
                description: None,
                input_schema: ToolInputSchema {
                    properties: None,
                    required: None,
                    r#type: "object".to_string(),
                },
                name: "ping".to_string(),
                output_schema: None,
                title: None,
            },
        );

        let auth_statuses: HashMap<String, McpAuthStatus> = HashMap::new();
        let cell = new_mcp_tools_output(
            &config,
            tools,
            HashMap::new(),
            HashMap::new(),
            &auth_statuses,
        );
        let rendered = render_lines(&cell.display_lines(120)).join("\n");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn empty_agent_message_cell_transcript() {
        let cell = AgentMessageCell::new(vec![Line::default()], false);
        assert_eq!(cell.transcript_lines(80), vec![Line::from("  ")]);
        assert_eq!(cell.desired_transcript_height(80), 1);
    }

    #[test]
    fn prefixed_wrapped_history_cell_indents_wrapped_lines() {
        let summary = Line::from(vec![
            "You ".into(),
            "approved".bold(),
            " codex to run ".into(),
            "echo something really long to ensure wrapping happens".dim(),
            " this time".bold(),
        ]);
        let cell = PrefixedWrappedHistoryCell::new(summary, "✔ ".green(), "  ");
        let rendered = render_lines(&cell.display_lines(24));
        assert_eq!(
            rendered,
            vec![
                "✔ You approved codex to".to_string(),
                "  run echo something".to_string(),
                "  really long to ensure".to_string(),
                "  wrapping happens this".to_string(),
                "  time".to_string(),
            ]
        );
    }

    #[test]
    fn web_search_history_cell_snapshot() {
        let cell = new_web_search_call(
            "example search query with several generic words to exercise wrapping".to_string(),
        );
        let rendered = render_lines(&cell.display_lines(64)).join("\n");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn web_search_history_cell_wraps_with_indented_continuation() {
        let cell = new_web_search_call(
            "example search query with several generic words to exercise wrapping".to_string(),
        );
        let rendered = render_lines(&cell.display_lines(64));

        assert_eq!(
            rendered,
            vec![
                "• Searched example search query with several generic words to".to_string(),
                "  exercise wrapping".to_string(),
            ]
        );
    }

    #[test]
    fn web_search_history_cell_short_query_does_not_wrap() {
        let cell = new_web_search_call("short query".to_string());
        let rendered = render_lines(&cell.display_lines(64));

        assert_eq!(rendered, vec!["• Searched short query".to_string()]);
    }

    #[test]
    fn web_search_history_cell_transcript_snapshot() {
        let cell = new_web_search_call(
            "example search query with several generic words to exercise wrapping".to_string(),
        );
        let rendered = render_lines(&cell.transcript_lines(64)).join("\n");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn active_mcp_tool_call_snapshot() {
        let invocation = McpInvocation {
            server: "search".into(),
            tool: "find_docs".into(),
            arguments: Some(json!({
                "query": "ratatui styling",
                "limit": 3,
            })),
        };

        let cell = new_active_mcp_tool_call("call-1".into(), invocation, true);
        let rendered = render_lines(&cell.display_lines(80)).join("\n");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn completed_mcp_tool_call_success_snapshot() {
        let invocation = McpInvocation {
            server: "search".into(),
            tool: "find_docs".into(),
            arguments: Some(json!({
                "query": "ratatui styling",
                "limit": 3,
            })),
        };

        let result = CallToolResult {
            content: vec![ContentBlock::TextContent(TextContent {
                annotations: None,
                text: "Found styling guidance in styles.md".into(),
                r#type: "text".into(),
            })],
            is_error: None,
            structured_content: None,
        };

        let mut cell = new_active_mcp_tool_call("call-2".into(), invocation, true);
        assert!(
            cell.complete(Duration::from_millis(1420), Ok(result))
                .is_none()
        );

        let rendered = render_lines(&cell.display_lines(80)).join("\n");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn completed_mcp_tool_call_error_snapshot() {
        let invocation = McpInvocation {
            server: "search".into(),
            tool: "find_docs".into(),
            arguments: Some(json!({
                "query": "ratatui styling",
                "limit": 3,
            })),
        };

        let mut cell = new_active_mcp_tool_call("call-3".into(), invocation, true);
        assert!(
            cell.complete(Duration::from_secs(2), Err("network timeout".into()))
                .is_none()
        );

        let rendered = render_lines(&cell.display_lines(80)).join("\n");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn completed_mcp_tool_call_multiple_outputs_snapshot() {
        let invocation = McpInvocation {
            server: "search".into(),
            tool: "find_docs".into(),
            arguments: Some(json!({
                "query": "ratatui styling",
                "limit": 3,
            })),
        };

        let result = CallToolResult {
            content: vec![
                ContentBlock::TextContent(TextContent {
                    annotations: None,
                    text: "Found styling guidance in styles.md and additional notes in CONTRIBUTING.md.".into(),
                    r#type: "text".into(),
                }),
                ContentBlock::ResourceLink(ResourceLink {
                    annotations: None,
                    description: Some("Link to styles documentation".into()),
                    mime_type: None,
                    name: "styles.md".into(),
                    size: None,
                    title: Some("Styles".into()),
                    r#type: "resource_link".into(),
                    uri: "file:///docs/styles.md".into(),
                }),
            ],
            is_error: None,
            structured_content: None,
        };

        let mut cell = new_active_mcp_tool_call("call-4".into(), invocation, true);
        assert!(
            cell.complete(Duration::from_millis(640), Ok(result))
                .is_none()
        );

        let rendered = render_lines(&cell.display_lines(48)).join("\n");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn completed_mcp_tool_call_wrapped_outputs_snapshot() {
        let invocation = McpInvocation {
            server: "metrics".into(),
            tool: "get_nearby_metric".into(),
            arguments: Some(json!({
                "query": "very_long_query_that_needs_wrapping_to_display_properly_in_the_history",
                "limit": 1,
            })),
        };

        let result = CallToolResult {
            content: vec![ContentBlock::TextContent(TextContent {
                annotations: None,
                text: "Line one of the response, which is quite long and needs wrapping.\nLine two continues the response with more detail.".into(),
                r#type: "text".into(),
            })],
            is_error: None,
            structured_content: None,
        };

        let mut cell = new_active_mcp_tool_call("call-5".into(), invocation, true);
        assert!(
            cell.complete(Duration::from_millis(1280), Ok(result))
                .is_none()
        );

        let rendered = render_lines(&cell.display_lines(40)).join("\n");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn completed_mcp_tool_call_multiple_outputs_inline_snapshot() {
        let invocation = McpInvocation {
            server: "metrics".into(),
            tool: "summary".into(),
            arguments: Some(json!({
                "metric": "trace.latency",
                "window": "15m",
            })),
        };

        let result = CallToolResult {
            content: vec![
                ContentBlock::TextContent(TextContent {
                    annotations: None,
                    text: "Latency summary: p50=120ms, p95=480ms.".into(),
                    r#type: "text".into(),
                }),
                ContentBlock::TextContent(TextContent {
                    annotations: None,
                    text: "No anomalies detected.".into(),
                    r#type: "text".into(),
                }),
            ],
            is_error: None,
            structured_content: None,
        };

        let mut cell = new_active_mcp_tool_call("call-6".into(), invocation, true);
        assert!(
            cell.complete(Duration::from_millis(320), Ok(result))
                .is_none()
        );

        let rendered = render_lines(&cell.display_lines(120)).join("\n");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn session_header_shows_model_and_reasoning() {
        let model_state =
            SharedModelState::new("gpt-4o".to_string(), Some(ReasoningEffortConfig::High));
        let cell = SessionHeaderHistoryCell::new(model_state, std::env::temp_dir(), "test", None);

        let lines = render_lines(&cell.display_lines(80));
        let model_line = lines
            .iter()
            .find(|line| line.contains("model:"))
            .expect("model line");

        assert!(model_line.contains("gpt-4o"));
        assert!(model_line.contains("high"));
        assert!(model_line.contains("/model"));
    }

    #[test]
    fn session_header_updates_when_model_changes() {
        let model_state =
            SharedModelState::new("gpt-4o".to_string(), Some(ReasoningEffortConfig::High));
        let cell =
            SessionHeaderHistoryCell::new(model_state.clone(), std::env::temp_dir(), "test", None);

        // Initial state
        let lines = render_lines(&cell.display_lines(80));
        let model_line = lines
            .iter()
            .find(|line| line.contains("model:"))
            .expect("model line");
        assert!(model_line.contains("gpt-4o"));
        assert!(model_line.contains("high"));

        // Update the model
        model_state.update(
            "gpt-5.1-codex-max".to_string(),
            Some(ReasoningEffortConfig::XHigh),
        );

        // Verify the header now shows the new model
        let lines = render_lines(&cell.display_lines(80));
        let model_line = lines
            .iter()
            .find(|line| line.contains("model:"))
            .expect("model line");
        assert!(model_line.contains("gpt-5.1-codex-max"));
        assert!(model_line.contains("xhigh"));
    }

    #[test]
    fn session_header_directory_center_truncates() {
        let mut dir = home_dir().expect("home directory");
        for part in ["hello", "the", "fox", "is", "very", "fast"] {
            dir.push(part);
        }

        let formatted = SessionHeaderHistoryCell::format_directory_inner(&dir, Some(24));
        let sep = std::path::MAIN_SEPARATOR;
        let expected = format!("~{sep}hello{sep}the{sep}…{sep}very{sep}fast");
        assert_eq!(formatted, expected);
    }

    #[test]
    fn session_header_directory_front_truncates_long_segment() {
        let mut dir = home_dir().expect("home directory");
        dir.push("supercalifragilisticexpialidocious");

        let formatted = SessionHeaderHistoryCell::format_directory_inner(&dir, Some(18));
        let sep = std::path::MAIN_SEPARATOR;
        let expected = format!("~{sep}…cexpialidocious");
        assert_eq!(formatted, expected);
    }

    #[test]
    fn coalesces_sequential_reads_within_one_call() {
        // Build one exec cell with a Search followed by two Reads
        let call_id = "c1".to_string();
        let mut cell = ExecCell::new(
            ExecCall {
                call_id: call_id.clone(),
                command: vec!["bash".into(), "-lc".into(), "echo".into()],
                parsed: vec![
                    ParsedCommand::Search {
                        query: Some("shimmer_spans".into()),
                        path: None,
                        cmd: "rg shimmer_spans".into(),
                    },
                    ParsedCommand::Read {
                        name: "shimmer.rs".into(),
                        cmd: "cat shimmer.rs".into(),
                        path: "shimmer.rs".into(),
                    },
                    ParsedCommand::Read {
                        name: "status_indicator_widget.rs".into(),
                        cmd: "cat status_indicator_widget.rs".into(),
                        path: "status_indicator_widget.rs".into(),
                    },
                ],
                output: None,
                source: ExecCommandSource::Agent,
                start_time: Some(Instant::now()),
                duration: None,
                interaction_input: None,
            },
            true,
        );
        // Mark call complete so markers are ✓
        cell.complete_call(&call_id, CommandOutput::default(), Duration::from_millis(1));

        let lines = cell.display_lines(80);
        let rendered = render_lines(&lines).join("\n");
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn coalesces_reads_across_multiple_calls() {
        let mut cell = ExecCell::new(
            ExecCall {
                call_id: "c1".to_string(),
                command: vec!["bash".into(), "-lc".into(), "echo".into()],
                parsed: vec![ParsedCommand::Search {
                    query: Some("shimmer_spans".into()),
                    path: None,
                    cmd: "rg shimmer_spans".into(),
                }],
                output: None,
                source: ExecCommandSource::Agent,
                start_time: Some(Instant::now()),
                duration: None,
                interaction_input: None,
            },
            true,
        );
        // Call 1: Search only
        cell.complete_call("c1", CommandOutput::default(), Duration::from_millis(1));
        // Call 2: Read A
        cell = cell
            .with_added_call(
                "c2".into(),
                vec!["bash".into(), "-lc".into(), "echo".into()],
                vec![ParsedCommand::Read {
                    name: "shimmer.rs".into(),
                    cmd: "cat shimmer.rs".into(),
                    path: "shimmer.rs".into(),
                }],
                ExecCommandSource::Agent,
                None,
            )
            .unwrap();
        cell.complete_call("c2", CommandOutput::default(), Duration::from_millis(1));
        // Call 3: Read B
        cell = cell
            .with_added_call(
                "c3".into(),
                vec!["bash".into(), "-lc".into(), "echo".into()],
                vec![ParsedCommand::Read {
                    name: "status_indicator_widget.rs".into(),
                    cmd: "cat status_indicator_widget.rs".into(),
                    path: "status_indicator_widget.rs".into(),
                }],
                ExecCommandSource::Agent,
                None,
            )
            .unwrap();
        cell.complete_call("c3", CommandOutput::default(), Duration::from_millis(1));

        let lines = cell.display_lines(80);
        let rendered = render_lines(&lines).join("\n");
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn coalesced_reads_dedupe_names() {
        let mut cell = ExecCell::new(
            ExecCall {
                call_id: "c1".to_string(),
                command: vec!["bash".into(), "-lc".into(), "echo".into()],
                parsed: vec![
                    ParsedCommand::Read {
                        name: "auth.rs".into(),
                        cmd: "cat auth.rs".into(),
                        path: "auth.rs".into(),
                    },
                    ParsedCommand::Read {
                        name: "auth.rs".into(),
                        cmd: "cat auth.rs".into(),
                        path: "auth.rs".into(),
                    },
                    ParsedCommand::Read {
                        name: "shimmer.rs".into(),
                        cmd: "cat shimmer.rs".into(),
                        path: "shimmer.rs".into(),
                    },
                ],
                output: None,
                source: ExecCommandSource::Agent,
                start_time: Some(Instant::now()),
                duration: None,
                interaction_input: None,
            },
            true,
        );
        cell.complete_call("c1", CommandOutput::default(), Duration::from_millis(1));
        let lines = cell.display_lines(80);
        let rendered = render_lines(&lines).join("\n");
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn multiline_command_wraps_with_extra_indent_on_subsequent_lines() {
        // Create a completed exec cell with a multiline command
        let cmd = "set -o pipefail\ncargo test --all-features --quiet".to_string();
        let call_id = "c1".to_string();
        let mut cell = ExecCell::new(
            ExecCall {
                call_id: call_id.clone(),
                command: vec!["bash".into(), "-lc".into(), cmd],
                parsed: Vec::new(),
                output: None,
                source: ExecCommandSource::Agent,
                start_time: Some(Instant::now()),
                duration: None,
                interaction_input: None,
            },
            true,
        );
        // Mark call complete so it renders as "Ran"
        cell.complete_call(&call_id, CommandOutput::default(), Duration::from_millis(1));

        // Small width to force wrapping on both lines
        let width: u16 = 28;
        let lines = cell.display_lines(width);
        let rendered = render_lines(&lines).join("\n");
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn single_line_command_compact_when_fits() {
        let call_id = "c1".to_string();
        let mut cell = ExecCell::new(
            ExecCall {
                call_id: call_id.clone(),
                command: vec!["echo".into(), "ok".into()],
                parsed: Vec::new(),
                output: None,
                source: ExecCommandSource::Agent,
                start_time: Some(Instant::now()),
                duration: None,
                interaction_input: None,
            },
            true,
        );
        cell.complete_call(&call_id, CommandOutput::default(), Duration::from_millis(1));
        // Wide enough that it fits inline
        let lines = cell.display_lines(80);
        let rendered = render_lines(&lines).join("\n");
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn single_line_command_wraps_with_four_space_continuation() {
        let call_id = "c1".to_string();
        let long = "a_very_long_token_without_spaces_to_force_wrapping".to_string();
        let mut cell = ExecCell::new(
            ExecCall {
                call_id: call_id.clone(),
                command: vec!["bash".into(), "-lc".into(), long],
                parsed: Vec::new(),
                output: None,
                source: ExecCommandSource::Agent,
                start_time: Some(Instant::now()),
                duration: None,
                interaction_input: None,
            },
            true,
        );
        cell.complete_call(&call_id, CommandOutput::default(), Duration::from_millis(1));
        let lines = cell.display_lines(24);
        let rendered = render_lines(&lines).join("\n");
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn multiline_command_without_wrap_uses_branch_then_eight_spaces() {
        let call_id = "c1".to_string();
        let cmd = "echo one\necho two".to_string();
        let mut cell = ExecCell::new(
            ExecCall {
                call_id: call_id.clone(),
                command: vec!["bash".into(), "-lc".into(), cmd],
                parsed: Vec::new(),
                output: None,
                source: ExecCommandSource::Agent,
                start_time: Some(Instant::now()),
                duration: None,
                interaction_input: None,
            },
            true,
        );
        cell.complete_call(&call_id, CommandOutput::default(), Duration::from_millis(1));
        let lines = cell.display_lines(80);
        let rendered = render_lines(&lines).join("\n");
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn multiline_command_both_lines_wrap_with_correct_prefixes() {
        let call_id = "c1".to_string();
        let cmd = "first_token_is_long_enough_to_wrap\nsecond_token_is_also_long_enough_to_wrap"
            .to_string();
        let mut cell = ExecCell::new(
            ExecCall {
                call_id: call_id.clone(),
                command: vec!["bash".into(), "-lc".into(), cmd],
                parsed: Vec::new(),
                output: None,
                source: ExecCommandSource::Agent,
                start_time: Some(Instant::now()),
                duration: None,
                interaction_input: None,
            },
            true,
        );
        cell.complete_call(&call_id, CommandOutput::default(), Duration::from_millis(1));
        let lines = cell.display_lines(28);
        let rendered = render_lines(&lines).join("\n");
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn stderr_tail_more_than_five_lines_snapshot() {
        // Build an exec cell with a non-zero exit and 10 lines on stderr to exercise
        // the head/tail rendering and gutter prefixes.
        let call_id = "c_err".to_string();
        let mut cell = ExecCell::new(
            ExecCall {
                call_id: call_id.clone(),
                command: vec!["bash".into(), "-lc".into(), "seq 1 10 1>&2 && false".into()],
                parsed: Vec::new(),
                output: None,
                source: ExecCommandSource::Agent,
                start_time: Some(Instant::now()),
                duration: None,
                interaction_input: None,
            },
            true,
        );
        let stderr: String = (1..=10)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        cell.complete_call(
            &call_id,
            CommandOutput {
                exit_code: 1,
                formatted_output: String::new(),
                aggregated_output: stderr,
            },
            Duration::from_millis(1),
        );

        let rendered = cell
            .display_lines(80)
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn ran_cell_multiline_with_stderr_snapshot() {
        // Build an exec cell that completes (so it renders as "Ran") with a
        // command long enough that it must render on its own line under the
        // header, and include a couple of stderr lines to verify the output
        // block prefixes and wrapping.
        let call_id = "c_wrap_err".to_string();
        let long_cmd =
            "echo this_is_a_very_long_single_token_that_will_wrap_across_the_available_width";
        let mut cell = ExecCell::new(
            ExecCall {
                call_id: call_id.clone(),
                command: vec!["bash".into(), "-lc".into(), long_cmd.to_string()],
                parsed: Vec::new(),
                output: None,
                source: ExecCommandSource::Agent,
                start_time: Some(Instant::now()),
                duration: None,
                interaction_input: None,
            },
            true,
        );

        let stderr = "error: first line on stderr\nerror: second line on stderr".to_string();
        cell.complete_call(
            &call_id,
            CommandOutput {
                exit_code: 1,
                formatted_output: String::new(),
                aggregated_output: stderr,
            },
            Duration::from_millis(5),
        );

        // Narrow width to force the command to render under the header line.
        let width: u16 = 28;
        let rendered = cell
            .display_lines(width)
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        insta::assert_snapshot!(rendered);
    }
    #[test]
    fn user_history_cell_wraps_and_prefixes_each_line_snapshot() {
        let msg = "one two three four five six seven";
        let cell = UserHistoryCell {
            message: msg.to_string(),
        };

        // Small width to force wrapping more clearly. Effective wrap width is width-2 due to the ▌ prefix and trailing space.
        let width: u16 = 12;
        let lines = cell.display_lines(width);
        let rendered = render_lines(&lines).join("\n");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn plan_update_with_note_and_wrapping_snapshot() {
        // Long explanation forces wrapping; include long step text to verify step wrapping and alignment.
        let update = UpdatePlanArgs {
            explanation: Some(
                "I’ll update Grafana call error handling by adding retries and clearer messages when the backend is unreachable."
                    .to_string(),
            ),
            plan: vec![
                PlanItemArg {
                    step: "Investigate existing error paths and logging around HTTP timeouts".into(),
                    status: StepStatus::Completed,
                },
                PlanItemArg {
                    step: "Harden Grafana client error handling with retry/backoff and user‑friendly messages".into(),
                    status: StepStatus::InProgress,
                },
                PlanItemArg {
                    step: "Add tests for transient failure scenarios and surfacing to the UI".into(),
                    status: StepStatus::Pending,
                },
            ],
        };

        let cell = new_plan_update(update);
        // Narrow width to force wrapping for both the note and steps
        let lines = cell.display_lines(32);
        let rendered = render_lines(&lines).join("\n");
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn plan_update_without_note_snapshot() {
        let update = UpdatePlanArgs {
            explanation: None,
            plan: vec![
                PlanItemArg {
                    step: "Define error taxonomy".into(),
                    status: StepStatus::InProgress,
                },
                PlanItemArg {
                    step: "Implement mapping to user messages".into(),
                    status: StepStatus::Pending,
                },
            ],
        };

        let cell = new_plan_update(update);
        let lines = cell.display_lines(40);
        let rendered = render_lines(&lines).join("\n");
        insta::assert_snapshot!(rendered);
    }
    #[test]
    fn reasoning_summary_block() {
        let cell = new_reasoning_summary_block(
            "**High level reasoning**\n\nDetailed reasoning goes here.".to_string(),
        );

        // New elegant format with thinking header
        let rendered_display = render_lines(&cell.display_lines(80));
        assert!(
            rendered_display[0].contains("Thinking"),
            "first line should have thinking header"
        );
        assert!(
            rendered_display
                .iter()
                .any(|l| l.contains("Detailed reasoning")),
            "should contain reasoning content"
        );
    }

    #[test]
    fn reasoning_summary_block_returns_reasoning_cell_when_feature_disabled() {
        let cell = new_reasoning_summary_block("Detailed reasoning goes here.".to_string());

        let rendered = render_transcript(cell.as_ref());
        assert!(
            rendered[0].contains("Thinking"),
            "first line should have thinking header"
        );
        assert!(
            rendered.iter().any(|l| l.contains("Detailed reasoning")),
            "should contain reasoning content"
        );
    }

    #[tokio::test]
    async fn reasoning_summary_block_respects_config_overrides() {
        let mut config = test_config().await;
        config.model = Some("gpt-3.5-turbo".to_string());
        config.model_supports_reasoning_summaries = Some(true);

        let cell = new_reasoning_summary_block(
            "**High level reasoning**\n\nDetailed reasoning goes here.".to_string(),
        );

        // New elegant format with thinking header
        let rendered_display = render_lines(&cell.display_lines(80));
        assert!(
            rendered_display[0].contains("Thinking"),
            "first line should have thinking header"
        );
    }

    #[test]
    fn reasoning_summary_block_falls_back_when_header_is_missing() {
        let cell =
            new_reasoning_summary_block("**High level reasoning without closing".to_string());

        let rendered = render_transcript(cell.as_ref());
        assert!(
            rendered[0].contains("Thinking"),
            "first line should have thinking header"
        );
    }

    #[test]
    fn reasoning_summary_block_falls_back_when_summary_is_missing() {
        let cell =
            new_reasoning_summary_block("**High level reasoning without closing**".to_string());

        let rendered = render_transcript(cell.as_ref());
        assert!(
            rendered[0].contains("Thinking"),
            "first line should have thinking header"
        );

        let cell = new_reasoning_summary_block(
            "**High level reasoning without closing**\n\n  ".to_string(),
        );

        let rendered = render_transcript(cell.as_ref());
        assert!(
            rendered[0].contains("Thinking"),
            "first line should have thinking header"
        );
    }

    #[test]
    fn reasoning_summary_block_splits_header_and_summary_when_present() {
        let cell = new_reasoning_summary_block(
            "**High level plan**\n\nWe should fix the bug next.".to_string(),
        );

        // New elegant format with thinking header
        let rendered_display = render_lines(&cell.display_lines(80));
        assert!(
            rendered_display[0].contains("Thinking"),
            "first line should have thinking header"
        );
        assert!(
            rendered_display.iter().any(|l| l.contains("fix the bug")),
            "should contain reasoning content"
        );
    }

    #[test]
    fn deprecation_notice_renders_summary_with_details() {
        let cell = new_deprecation_notice(
            "Feature flag `foo`".to_string(),
            Some("Use flag `bar` instead.".to_string()),
        );
        let lines = cell.display_lines(80);
        let rendered = render_lines(&lines);
        assert_eq!(
            rendered,
            vec![
                "⚠ Feature flag `foo`".to_string(),
                "Use flag `bar` instead.".to_string(),
            ]
        );
    }
}
