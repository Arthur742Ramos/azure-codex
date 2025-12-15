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
        let [intro_area, input_area, footer_area] = Layout::vertical([
            Constraint::Min(8),
            Constraint::Length(3),
            Constraint::Min(4),
        ])
        .areas(area);

        let intro_lines: Vec<Line> = vec![
            Line::from(vec!["  ".into(), "Welcome to Azure Codex!".bold().cyan()]),
            "".into(),
            "  Enter your Azure OpenAI endpoint to get started.".into(),
            "".into(),
            Line::from(vec![
                "  ".into(),
                "Example: ".dim(),
                "https://your-resource.openai.azure.com".cyan(),
            ]),
            "".into(),
            "  You can find this in the Azure Portal under your"
                .dim()
                .into(),
            "  Azure OpenAI resource > Keys and Endpoint.".dim().into(),
            "".into(),
        ];

        Paragraph::new(intro_lines)
            .wrap(Wrap { trim: false })
            .render(intro_area, buf);

        let endpoint = self.endpoint_input.read().unwrap();
        let content_line: Line = if endpoint.is_empty() {
            vec!["https://".dim()].into()
        } else {
            // Use cyan styling to ensure visibility on all terminal backgrounds
            Line::from(endpoint.clone()).fg(Color::Cyan)
        };
        drop(endpoint);

        Paragraph::new(content_line)
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title("Azure OpenAI Endpoint")
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
        let models = self.models.read().unwrap();
        let selected_idx = *self.selected_model_idx.read().unwrap();
        let scroll_offset = *self.scroll_offset.read().unwrap();

        let mut lines: Vec<Line> = vec![
            Line::from(vec![
                "  ".into(),
                "Select a GPT model deployment".bold().cyan(),
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
                let marker = if is_selected { "●" } else { " " };
                let style = if is_selected {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default()
                };

                lines.push(Line::from(vec![
                    Span::styled(format!("  {marker} "), style),
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
        lines.push("  Use ↑↓ to select, Enter to confirm".dim().into());
        lines.push("  Press Esc to go back".dim().into());

        drop(models);

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_no_models_found(&self, area: Rect, buf: &mut Buffer) {
        let endpoint = self.endpoint_input.read().unwrap();
        let lines: Vec<Line> = vec![
            "".into(),
            Line::from(vec!["  ".into(), "No GPT deployments found".bold().red()]),
            "".into(),
            Line::from(vec!["  Endpoint: ".dim(), endpoint.clone().cyan()]),
            "".into(),
            "  Make sure you have:".into(),
            "".into(),
            "  1. Logged in with: az login".cyan().into(),
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

    fn render_complete(&self, area: Rect, buf: &mut Buffer) {
        let endpoint = self.configured_endpoint.read().unwrap();
        let model = self.configured_model.read().unwrap();

        let mut lines: Vec<Line> = vec!["✓ Azure OpenAI configured".fg(Color::Green).into()];

        if let Some(ep) = endpoint.as_ref() {
            lines.push(Line::from(vec!["  Endpoint: ".dim(), ep.clone().into()]));
        }
        if let Some(m) = model.as_ref() {
            lines.push(Line::from(vec!["  Model: ".dim(), m.clone().into()]));
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
            Line::from(vec!["  ".into(), "~/.azure-codex/config.toml".cyan()]),
        ];

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn start_fetching_models(&self) {
        let endpoint = self.endpoint_input.read().unwrap().clone();

        // Validate endpoint
        if endpoint.is_empty() {
            *self.error.write().unwrap() =
                Some("Please enter an Azure OpenAI endpoint".to_string());
            self.request_frame.schedule_frame();
            return;
        }

        if !endpoint.starts_with("https://") && !endpoint.starts_with("http://") {
            *self.error.write().unwrap() = Some("Endpoint must start with https://".to_string());
            self.request_frame.schedule_frame();
            return;
        }

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
        let endpoint = self.endpoint_input.read().unwrap().clone();
        let models = self.models.read().unwrap();
        let selected_idx = *self.selected_model_idx.read().unwrap();

        let model = models.get(selected_idx).map(|m| m.model.clone());
        drop(models);

        if let Some(model_name) = model {
            // Create config directory if it doesn't exist
            if let Err(e) = std::fs::create_dir_all(&self.codex_home) {
                *self.error.write().unwrap() =
                    Some(format!("Failed to create config directory: {e}"));
                self.request_frame.schedule_frame();
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
            let config_path = self.codex_home.join("config.toml");
            match std::fs::write(&config_path, config_content) {
                Ok(()) => {
                    *self.configured_endpoint.write().unwrap() = Some(endpoint);
                    *self.configured_model.write().unwrap() = Some(model_name);
                    *self.state.write().unwrap() = AzureSetupState::Complete;
                }
                Err(e) => {
                    *self.error.write().unwrap() = Some(format!("Failed to save config: {e}"));
                }
            }
        }

        self.request_frame.schedule_frame();
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
            AzureSetupState::ModelSelection => match key_event.code {
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
            },
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
            | AzureSetupState::NoModelsFound => StepState::InProgress,
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
