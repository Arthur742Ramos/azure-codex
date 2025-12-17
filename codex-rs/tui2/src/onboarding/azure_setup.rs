//! Azure OpenAI first-run setup widget.
//!
//! This widget guides users through configuring their Azure OpenAI endpoint
//! and selecting a model deployment when running Azure Codex for the first time.

#![allow(clippy::unwrap_used)]

use codex_core::azure::deployments::AzureDeploymentsManager;
use codex_protocol::openai_models::ModelPreset;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::prelude::Widget;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::Borders;
use ratatui::widgets::Paragraph;
use ratatui::widgets::WidgetRef;
use ratatui::widgets::Wrap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;

use crate::onboarding::onboarding_screen::KeyboardHandler;
use crate::onboarding::onboarding_screen::StepState;
use crate::onboarding::onboarding_screen::StepStateProvider;
use crate::shimmer::shimmer_spans;
use crate::theme;
use crate::tui::FrameRequester;

/// State machine for the Azure setup flow.
#[derive(Clone)]
pub enum AzureSetupState {
    /// User is entering the Azure OpenAI endpoint.
    EndpointEntry,
    /// Fetching models from Azure.
    FetchingModels,
    /// User is selecting a model from the list.
    ModelSelection,
    /// No models were found - show error.
    NoModelsFound,
    /// Saving configuration after model selection.
    Configuring,
    /// Setup is complete.
    Complete,
    /// User skipped setup.
    Skipped,
}

/// Maximum number of models visible in the selection list at once.
const MAX_VISIBLE_MODELS: usize = 8;

/// Widget for Azure OpenAI first-run setup.
pub struct AzureSetupWidget {
    /// Request a frame redraw.
    pub request_frame: FrameRequester,
    /// Current state of the setup flow.
    pub state: Arc<RwLock<AzureSetupState>>,
    /// User's endpoint input.
    pub endpoint_input: Arc<RwLock<String>>,
    /// Available models from Azure.
    pub models: Arc<RwLock<Vec<ModelPreset>>>,
    /// Currently selected model index.
    pub selected_model_idx: Arc<RwLock<usize>>,
    /// Scroll offset for model list (first visible model index).
    pub scroll_offset: Arc<RwLock<usize>>,
    /// Error message to display.
    pub error: Arc<RwLock<Option<String>>>,
    /// Path to codex home directory.
    pub codex_home: PathBuf,
    /// Whether animations are enabled.
    pub animations_enabled: bool,
    /// The configured endpoint (after setup completes).
    pub configured_endpoint: Arc<RwLock<Option<String>>>,
    /// The configured model (after setup completes).
    pub configured_model: Arc<RwLock<Option<String>>>,
}

impl AzureSetupWidget {
    pub fn new(
        codex_home: PathBuf,
        request_frame: FrameRequester,
        animations_enabled: bool,
    ) -> Self {
        Self {
            request_frame,
            state: Arc::new(RwLock::new(AzureSetupState::EndpointEntry)),
            endpoint_input: Arc::new(RwLock::new(String::new())),
            models: Arc::new(RwLock::new(Vec::new())),
            selected_model_idx: Arc::new(RwLock::new(0)),
            scroll_offset: Arc::new(RwLock::new(0)),
            error: Arc::new(RwLock::new(None)),
            codex_home,
            animations_enabled,
            configured_endpoint: Arc::new(RwLock::new(None)),
            configured_model: Arc::new(RwLock::new(None)),
        }
    }

    /// Get the configured endpoint after setup completes.
    pub fn get_configured_endpoint(&self) -> Option<String> {
        self.configured_endpoint.read().ok()?.clone()
    }

    /// Get the configured model after setup completes.
    pub fn get_configured_model(&self) -> Option<String> {
        self.configured_model.read().ok()?.clone()
    }

    fn render_endpoint_entry(&self, area: Rect, buf: &mut Buffer) {
        // Constrain the overall width to avoid rendering issues on narrow terminals
        // or when the terminal reports incorrect buffer size instead of viewport size.
        const MAX_CONTENT_WIDTH: u16 = 80;
        let content_width = area.width.min(MAX_CONTENT_WIDTH);
        let content_area = Rect {
            x: area.x,
            y: area.y,
            width: content_width,
            height: area.height,
        };

        // Adapt layout based on available height
        // Compact mode: just input + minimal hints (height < 8)
        // Medium mode: input + short intro (height 8-14)
        // Full mode: complete UI (height >= 15)
        let is_compact = area.height < 8;
        let is_medium = area.height >= 8 && area.height < 15;

        let endpoint = self.endpoint_input.read().unwrap();
        let content_line: Line = if endpoint.is_empty() {
            vec!["https://".dim()].into()
        } else {
            Line::from(endpoint.clone()).fg(Color::Cyan)
        };
        drop(endpoint);

        if is_compact {
            // Ultra-compact: just input box and one line of hints
            let [input_area, footer_area] =
                Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).areas(content_area);

            Paragraph::new(content_line)
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .title("Azure Endpoint")
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(Color::Cyan)),
                )
                .render(input_area, buf);

            let mut footer_lines: Vec<Line> = vec!["Enter=continue, Esc=skip".dim().into()];

            if let Ok(error_guard) = self.error.read()
                && let Some(err) = error_guard.as_ref()
            {
                footer_lines.push(err.clone().red().into());
            }

            Paragraph::new(footer_lines)
                .wrap(Wrap { trim: false })
                .render(footer_area, buf);
        } else if is_medium {
            // Medium: short intro + input + compact footer
            let [intro_area, input_area, footer_area] = Layout::vertical([
                Constraint::Min(2),
                Constraint::Length(3),
                Constraint::Min(2),
            ])
            .areas(content_area);

            let intro_lines: Vec<Line> = vec![
                Line::from(vec!["  ".into(), theme::brand_span("Azure Codex Setup")]),
                Line::from(vec![
                    "  ".into(),
                    "Ex: ".dim(),
                    theme::path_span("my-resource"),
                    " or ".dim(),
                    theme::path_span("https://my-resource.openai.azure.com"),
                ]),
            ];

            Paragraph::new(intro_lines)
                .wrap(Wrap { trim: false })
                .render(intro_area, buf);

            Paragraph::new(content_line)
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .title("Resource Name or Endpoint")
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(Color::Cyan)),
                )
                .render(input_area, buf);

            let mut footer_lines: Vec<Line> = vec!["  Enter=continue, Esc=skip".dim().into()];

            if let Ok(error_guard) = self.error.read()
                && let Some(err) = error_guard.as_ref()
            {
                footer_lines.push(err.clone().red().into());
            }

            Paragraph::new(footer_lines)
                .wrap(Wrap { trim: false })
                .render(footer_area, buf);
        } else {
            // Full mode: complete UI
            let [intro_area, input_area, footer_area] = Layout::vertical([
                Constraint::Min(8),
                Constraint::Length(3),
                Constraint::Min(4),
            ])
            .areas(content_area);

            let intro_lines: Vec<Line> = vec![
                Line::from(vec![
                    "  ".into(),
                    theme::brand_span("Welcome to Azure Codex!"),
                ]),
                "".into(),
                "  Enter your Azure OpenAI resource name or endpoint.".into(),
                "".into(),
                Line::from(vec![
                    "  ".into(),
                    "Examples: ".dim(),
                    theme::path_span("my-resource"),
                ]),
                Line::from(vec![
                    "            ".into(),
                    theme::path_span("https://my-resource.openai.azure.com"),
                ]),
                "".into(),
                "  You can find your resource name in the Azure Portal"
                    .dim()
                    .into(),
                "  under your Azure OpenAI resource.".dim().into(),
                "".into(),
            ];

            Paragraph::new(intro_lines)
                .wrap(Wrap { trim: false })
                .render(intro_area, buf);

            Paragraph::new(content_line)
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .title("Azure OpenAI Resource Name or Endpoint")
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(Color::Cyan)),
                )
                .render(input_area, buf);

            let mut footer_lines: Vec<Line> = vec![
                "".into(),
                "  Press Enter to continue".dim().into(),
                "  Press Esc to skip and configure manually later"
                    .dim()
                    .into(),
            ];

            if let Ok(error_guard) = self.error.read()
                && let Some(err) = error_guard.as_ref()
            {
                footer_lines.push("".into());
                footer_lines.push(err.clone().red().into());
            }

            Paragraph::new(footer_lines)
                .wrap(Wrap { trim: false })
                .render(footer_area, buf);
        }
    }

    fn render_fetching_models(&self, area: Rect, buf: &mut Buffer) {
        let mut spans: Vec<Span> = vec!["  ".into()];
        if self.animations_enabled {
            self.request_frame
                .schedule_frame_in(std::time::Duration::from_millis(100));
            spans.extend(shimmer_spans("Discovering Azure OpenAI deployments..."));
        } else {
            spans.push("Discovering Azure OpenAI deployments...".into());
        }

        let endpoint = self.endpoint_input.read().unwrap();
        let lines: Vec<Line> = vec![
            "".into(),
            spans.into(),
            "".into(),
            Line::from(vec!["  Endpoint: ".dim(), endpoint.clone().cyan()]),
            "".into(),
            "  This may take a moment...".dim().into(),
        ];
        drop(endpoint);

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_model_selection(&self, area: Rect, buf: &mut Buffer) {
        // Constrain the overall width to avoid rendering issues on narrow terminals
        const MAX_CONTENT_WIDTH: u16 = 80;
        let content_width = area.width.min(MAX_CONTENT_WIDTH);
        let content_area = Rect {
            x: area.x,
            y: area.y,
            width: content_width,
            height: area.height,
        };

        let models = self.models.read().unwrap();
        let selected_idx = *self.selected_model_idx.read().unwrap();
        let scroll_offset = *self.scroll_offset.read().unwrap();

        // Adapt layout based on available height
        let is_compact = area.height < 10;

        let mut lines: Vec<Line> = vec![
            Line::from(vec![
                "  ".into(),
                theme::header_span("Select a GPT model deployment"),
            ]),
            "".into(),
        ];

        if models.is_empty() {
            lines.push("  No GPT models found.".dim().into());
        } else {
            let total_models = models.len();
            let visible_count = MAX_VISIBLE_MODELS.min(total_models);
            let end_idx = (scroll_offset + visible_count).min(total_models);

            // Show scroll indicator if there are models above
            if scroll_offset > 0 {
                lines.push(Line::from(vec![
                    "  ".into(),
                    format!("  ▲ {scroll_offset} more above").dim(),
                ]));
            } else {
                lines.push("  Available GPT deployments:".into());
            }
            lines.push("".into());

            // Render visible models
            for idx in scroll_offset..end_idx {
                let model = &models[idx];
                let is_selected = idx == selected_idx;
                let marker = if is_selected {
                    theme::selected_marker()
                } else {
                    theme::unselected_marker()
                };
                let style = if is_selected {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default()
                };

                lines.push(Line::from(vec![
                    "  ".into(),
                    marker,
                    " ".into(),
                    Span::styled(
                        model.display_name.clone(),
                        style.add_modifier(Modifier::BOLD),
                    ),
                ]));
                lines.push(Line::from(vec![
                    "      ".into(),
                    Span::styled(model.description.clone(), Style::default().dim()),
                ]));
            }

            // Show scroll indicator if there are models below
            let remaining_below = total_models.saturating_sub(end_idx);
            if remaining_below > 0 {
                lines.push("".into());
                lines.push(Line::from(vec![
                    "  ".into(),
                    format!("  ▼ {remaining_below} more below").dim(),
                ]));
            }
        }

        lines.push("".into());
        if is_compact {
            lines.push("  ↑↓=select, Enter=confirm, Esc=back".dim().into());
        } else {
            lines.push("  Use ↑↓ to select, Enter to confirm".dim().into());
            lines.push("  Press Esc to go back".dim().into());
        }

        drop(models);

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(content_area, buf);
    }

    fn render_no_models_found(&self, area: Rect, buf: &mut Buffer) {
        let endpoint = self.endpoint_input.read().unwrap();
        let lines: Vec<Line> = vec![
            "".into(),
            Line::from(vec![
                "  ".into(),
                theme::error_span("No GPT deployments found"),
            ]),
            "".into(),
            Line::from(vec!["  Endpoint: ".dim(), theme::path_span(&endpoint)]),
            "".into(),
            "  Make sure you have:".into(),
            "".into(),
            Line::from(vec!["  1. Logged in with: ".into(), "az login".cyan()]),
            "  2. Deployed a GPT model in your Azure OpenAI resource".into(),
            "  3. The endpoint URL is correct".into(),
            "".into(),
            "  Press Enter to try again".dim().into(),
            "  Press Esc to enter a different endpoint".dim().into(),
        ];
        drop(endpoint);

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_configuring(&self, area: Rect, buf: &mut Buffer) {
        let mut spans: Vec<Span> = vec!["  ".into()];
        if self.animations_enabled {
            self.request_frame
                .schedule_frame_in(std::time::Duration::from_millis(100));
            spans.extend(shimmer_spans("Saving configuration..."));
        } else {
            spans.push("Saving configuration...".into());
        }

        let endpoint = self.endpoint_input.read().unwrap();
        let models = self.models.read().unwrap();
        let selected_idx = *self.selected_model_idx.read().unwrap();
        let model_name = models.get(selected_idx).map(|m| m.display_name.clone());
        drop(models);

        let mut lines: Vec<Line> = vec![
            "".into(),
            spans.into(),
            "".into(),
            Line::from(vec!["  Endpoint: ".dim(), theme::path_span(&endpoint)]),
        ];
        if let Some(name) = model_name {
            lines.push(Line::from(vec!["  Model: ".dim(), theme::path_span(&name)]));
        }
        drop(endpoint);

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_complete(&self, area: Rect, buf: &mut Buffer) {
        let endpoint = self.configured_endpoint.read().unwrap();
        let model = self.configured_model.read().unwrap();

        let mut lines: Vec<Line> = vec![Line::from(vec![
            theme::checkmark(),
            " Azure OpenAI configured".into(),
        ])];

        if let Some(ep) = endpoint.as_ref() {
            lines.push(Line::from(vec!["  Endpoint: ".dim(), theme::path_span(ep)]));
        }
        if let Some(m) = model.as_ref() {
            lines.push(Line::from(vec!["  Model: ".dim(), theme::path_span(m)]));
        }

        drop(endpoint);
        drop(model);

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_skipped(&self, area: Rect, buf: &mut Buffer) {
        let lines: Vec<Line> = vec![
            "  Azure setup skipped".dim().into(),
            "".into(),
            "  You can configure Azure OpenAI later by editing:"
                .dim()
                .into(),
            Line::from(vec![
                "  ".into(),
                theme::path_span("~/.azure-codex/config.toml"),
            ]),
        ];

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    /// Normalize endpoint input to a full URL.
    /// Accepts either:
    /// - A full URL (e.g., "https://my-resource.openai.azure.com")
    /// - Just the resource name (e.g., "my-resource")
    fn normalize_endpoint(input: &str) -> String {
        let trimmed = input.trim();

        // If it already looks like a URL, return as-is
        if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
            return trimmed.to_string();
        }

        // If it contains a dot, assume it's a domain without the scheme
        if trimmed.contains('.') {
            return format!("https://{trimmed}");
        }

        // Otherwise, assume it's just the resource name
        format!("https://{trimmed}.openai.azure.com")
    }

    fn start_fetching_models(&self) {
        let raw_input = self.endpoint_input.read().unwrap().clone();

        // Validate endpoint
        if raw_input.trim().is_empty() {
            *self.error.write().unwrap() =
                Some("Please enter an Azure OpenAI endpoint or resource name".to_string());
            self.request_frame.schedule_frame();
            return;
        }

        // Normalize the input to a full URL
        let endpoint = Self::normalize_endpoint(&raw_input);

        // Update the input field to show the normalized endpoint
        *self.endpoint_input.write().unwrap() = endpoint.clone();

        *self.state.write().unwrap() = AzureSetupState::FetchingModels;
        *self.error.write().unwrap() = None;
        self.request_frame.schedule_frame();

        let state = self.state.clone();
        let models = self.models.clone();
        let error = self.error.clone();
        let request_frame = self.request_frame.clone();

        tokio::spawn(async move {
            let manager = AzureDeploymentsManager::new(Some(endpoint));
            let presets = manager.get_gpt_model_presets().await;

            if presets.is_empty() {
                *state.write().unwrap() = AzureSetupState::NoModelsFound;
            } else {
                *models.write().unwrap() = presets;
                *state.write().unwrap() = AzureSetupState::ModelSelection;
            }
            *error.write().unwrap() = None;
            request_frame.schedule_frame();
        });
    }

    fn save_config(&self) {
        // First, transition to Configuring state to show visual feedback
        *self.state.write().unwrap() = AzureSetupState::Configuring;
        self.request_frame.schedule_frame();

        let endpoint = self.endpoint_input.read().unwrap().clone();
        let models = self.models.read().unwrap();
        let selected_idx = *self.selected_model_idx.read().unwrap();
        let model = models.get(selected_idx).map(|m| m.model.clone());
        drop(models);

        let codex_home = self.codex_home.clone();
        let state = self.state.clone();
        let error = self.error.clone();
        let configured_endpoint = self.configured_endpoint.clone();
        let configured_model = self.configured_model.clone();
        let request_frame = self.request_frame.clone();

        // Do the actual config writing in an async task to not block the UI
        tokio::spawn(async move {
            // Small delay to ensure the Configuring state is visible
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;

            if let Some(model_name) = model {
                // Create config directory if it doesn't exist
                if let Err(e) = std::fs::create_dir_all(&codex_home) {
                    *error.write().unwrap() =
                        Some(format!("Failed to create config directory: {e}"));
                    *state.write().unwrap() = AzureSetupState::ModelSelection;
                    request_frame.schedule_frame();
                    return;
                }

                // Build config content
                let config_content = format!(
                    r#"# Azure Codex configuration
# Generated by first-run setup

azure_endpoint = "{endpoint}"
model = "{model_name}"
"#
                );

                // Write config file
                let config_path = codex_home.join("config.toml");
                match std::fs::write(&config_path, config_content) {
                    Ok(()) => {
                        *configured_endpoint.write().unwrap() = Some(endpoint);
                        *configured_model.write().unwrap() = Some(model_name);
                        *state.write().unwrap() = AzureSetupState::Complete;
                    }
                    Err(e) => {
                        *error.write().unwrap() = Some(format!("Failed to save config: {e}"));
                        *state.write().unwrap() = AzureSetupState::ModelSelection;
                    }
                }
            }

            request_frame.schedule_frame();
        });
    }
}

impl KeyboardHandler for AzureSetupWidget {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        let state = self.state.read().unwrap().clone();

        match state {
            AzureSetupState::EndpointEntry => match key_event.code {
                KeyCode::Esc => {
                    *self.state.write().unwrap() = AzureSetupState::Skipped;
                    self.request_frame.schedule_frame();
                }
                KeyCode::Enter => {
                    self.start_fetching_models();
                }
                KeyCode::Backspace => {
                    let mut input = self.endpoint_input.write().unwrap();
                    input.pop();
                    *self.error.write().unwrap() = None;
                    drop(input);
                    self.request_frame.schedule_frame();
                }
                KeyCode::Char(c)
                    if key_event.kind == KeyEventKind::Press
                        && !key_event.modifiers.contains(KeyModifiers::SUPER)
                        && !key_event.modifiers.contains(KeyModifiers::CONTROL)
                        && !key_event.modifiers.contains(KeyModifiers::ALT) =>
                {
                    let mut input = self.endpoint_input.write().unwrap();
                    input.push(c);
                    *self.error.write().unwrap() = None;
                    drop(input);
                    self.request_frame.schedule_frame();
                }
                _ => {}
            },
            AzureSetupState::FetchingModels => {
                // Can't interact while fetching
            }
            AzureSetupState::ModelSelection => {
                // Only process key press events, not release or repeat
                if key_event.kind != KeyEventKind::Press {
                    return;
                }
                match key_event.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        let mut idx = self.selected_model_idx.write().unwrap();
                        if *idx > 0 {
                            *idx -= 1;
                            // Adjust scroll to keep selection visible
                            let mut scroll = self.scroll_offset.write().unwrap();
                            if *idx < *scroll {
                                *scroll = *idx;
                            }
                        }
                        drop(idx);
                        self.request_frame.schedule_frame();
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        let models = self.models.read().unwrap();
                        let len = models.len();
                        drop(models);

                        let mut idx = self.selected_model_idx.write().unwrap();
                        if *idx < len.saturating_sub(1) {
                            *idx += 1;
                            // Adjust scroll to keep selection visible
                            let mut scroll = self.scroll_offset.write().unwrap();
                            if *idx >= *scroll + MAX_VISIBLE_MODELS {
                                *scroll = idx.saturating_sub(MAX_VISIBLE_MODELS - 1);
                            }
                        }
                        drop(idx);
                        self.request_frame.schedule_frame();
                    }
                    KeyCode::Enter => {
                        self.save_config();
                    }
                    KeyCode::Esc => {
                        *self.state.write().unwrap() = AzureSetupState::EndpointEntry;
                        self.request_frame.schedule_frame();
                    }
                    _ => {}
                }
            }
            AzureSetupState::Configuring => {
                // Can't interact while saving configuration
            }
            AzureSetupState::NoModelsFound => match key_event.code {
                KeyCode::Enter => {
                    self.start_fetching_models();
                }
                KeyCode::Esc => {
                    *self.state.write().unwrap() = AzureSetupState::EndpointEntry;
                    self.request_frame.schedule_frame();
                }
                _ => {}
            },
            AzureSetupState::Complete | AzureSetupState::Skipped => {
                // No interaction needed
            }
        }
    }

    fn handle_paste(&mut self, pasted: String) {
        let state = self.state.read().unwrap().clone();

        if matches!(state, AzureSetupState::EndpointEntry) {
            let trimmed = pasted.trim();
            if !trimmed.is_empty() {
                let mut input = self.endpoint_input.write().unwrap();
                input.push_str(trimmed);
                *self.error.write().unwrap() = None;
                drop(input);
                self.request_frame.schedule_frame();
            }
        }
    }
}

impl StepStateProvider for AzureSetupWidget {
    fn get_step_state(&self) -> StepState {
        let state = self.state.read().unwrap();
        match &*state {
            AzureSetupState::EndpointEntry
            | AzureSetupState::FetchingModels
            | AzureSetupState::ModelSelection
            | AzureSetupState::NoModelsFound
            | AzureSetupState::Configuring => StepState::InProgress,
            AzureSetupState::Complete | AzureSetupState::Skipped => StepState::Complete,
        }
    }
}

impl WidgetRef for AzureSetupWidget {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        let state = self.state.read().unwrap();
        match &*state {
            AzureSetupState::EndpointEntry => {
                drop(state);
                self.render_endpoint_entry(area, buf);
            }
            AzureSetupState::FetchingModels => {
                drop(state);
                self.render_fetching_models(area, buf);
            }
            AzureSetupState::ModelSelection => {
                drop(state);
                self.render_model_selection(area, buf);
            }
            AzureSetupState::NoModelsFound => {
                drop(state);
                self.render_no_models_found(area, buf);
            }
            AzureSetupState::Configuring => {
                drop(state);
                self.render_configuring(area, buf);
            }
            AzureSetupState::Complete => {
                drop(state);
                self.render_complete(area, buf);
            }
            AzureSetupState::Skipped => {
                drop(state);
                self.render_skipped(area, buf);
            }
        }
    }
}
