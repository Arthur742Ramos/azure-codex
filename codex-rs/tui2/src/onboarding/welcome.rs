use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::Widget;
use ratatui::text::Line;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use ratatui::widgets::WidgetRef;
use ratatui::widgets::Wrap;

use crate::ascii_animation::AsciiAnimation;
use crate::onboarding::onboarding_screen::KeyboardHandler;
use crate::onboarding::onboarding_screen::StepStateProvider;
use crate::theme;
use crate::tui::FrameRequester;

use super::onboarding_screen::StepState;

const MIN_ANIMATION_HEIGHT: u16 = 20;
const MIN_ANIMATION_WIDTH: u16 = 60;

pub(crate) struct WelcomeWidget {
    #[allow(dead_code)] // Kept for potential future use
    pub is_logged_in: bool,
    animation: AsciiAnimation,
    animations_enabled: bool,
}

impl KeyboardHandler for WelcomeWidget {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        if !self.animations_enabled {
            return;
        }
        if key_event.kind == KeyEventKind::Press
            && key_event.code == KeyCode::Char('.')
            && key_event.modifiers.contains(KeyModifiers::CONTROL)
        {
            tracing::warn!("Welcome background to press '.'");
            let _ = self.animation.pick_random_variant();
        }
    }
}

impl WelcomeWidget {
    pub(crate) fn new(
        is_logged_in: bool,
        request_frame: FrameRequester,
        animations_enabled: bool,
    ) -> Self {
        Self {
            is_logged_in,
            animation: AsciiAnimation::new(request_frame),
            animations_enabled,
        }
    }
}

impl WidgetRef for &WelcomeWidget {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        use ratatui::style::Stylize;

        Clear.render(area, buf);
        if self.animations_enabled {
            self.animation.schedule_next_frame();
        }

        // Skip the animation when viewport is too small so we don't clip frames.
        // Animation needs at least 60 width and 20 height
        let show_animation = area.height >= MIN_ANIMATION_HEIGHT
            && area.width >= MIN_ANIMATION_WIDTH
            && self.animations_enabled;

        // For very narrow terminals, use compact mode
        let is_compact = area.width < 50;

        let mut lines: Vec<Line> = Vec::new();

        // Track actual content height as we build it
        let mut content_lines: u16 = 0;

        // Add some vertical padding at the top for larger windows
        let v_padding: u16 = if show_animation {
            1
        } else if area.height > 15 {
            ((area.height.saturating_sub(10)) / 4).min(4)
        } else {
            0
        };
        for _ in 0..v_padding {
            lines.push("".into());
            content_lines += 1;
        }

        if show_animation {
            let frame = self.animation.current_frame();
            // Center the animation block by adding left padding to each line
            let frame_lines: Vec<&str> = frame.lines().collect();
            let animation_height = frame_lines.len() as u16;

            let max_frame_width = frame_lines.iter().map(|l| l.len()).max().unwrap_or(0);
            let anim_padding = if area.width as usize > max_frame_width {
                (area.width as usize - max_frame_width) / 2
            } else {
                0
            };
            let padding_str: String = " ".repeat(anim_padding);

            for line in frame_lines {
                lines.push(format!("{padding_str}{line}").into());
            }
            lines.push("".into());
            content_lines += animation_height + 1;
        }

        // Welcome message - always centered for cleaner look
        let welcome_line = Line::from(vec![
            "Welcome to ".into(),
            theme::brand_span("Azure Codex"),
            ", your Azure-powered coding assistant".into(),
        ])
        .alignment(ratatui::layout::Alignment::Center);

        lines.push(welcome_line);
        content_lines += 1;

        // Add quick start hints if there's enough room
        // Need 7 lines: 2 blank + title + blank + 4 hints
        let space_used = content_lines + 2; // +2 for spacing before hints
        let has_room_for_hints = area.height >= space_used + 6; // 6 = title + blank + 4 hints

        if has_room_for_hints {
            lines.push("".into());
            if !is_compact {
                lines.push("".into());
            }

            let start_title = Line::from(vec!["Quick Start:".bold()])
                .alignment(ratatui::layout::Alignment::Center);
            lines.push(start_title);

            lines.push("".into());

            // Simple list format - centered for cleaner look
            let hint_lines: Vec<Vec<ratatui::text::Span>> = vec![
                vec!["/help".cyan(), "  Show all commands".dim()],
                vec!["/model".cyan(), " Switch AI model".dim()],
                vec!["@file".cyan(), "  Reference a file".dim()],
                vec!["Ctrl+C".cyan(), " Interrupt / Exit".dim()],
            ];

            for parts in hint_lines {
                let line = Line::from(parts).alignment(ratatui::layout::Alignment::Center);
                lines.push(line);
            }
        }

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }
}

impl StepStateProvider for WelcomeWidget {
    fn get_step_state(&self) -> StepState {
        // Always hidden - Azure Setup widget already shows welcome message
        // and needs the screen space for the endpoint input field
        StepState::Hidden
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    static VARIANT_A: [&str; 1] = ["frame-a"];
    static VARIANT_B: [&str; 1] = ["frame-b"];
    static VARIANTS: [&[&str]; 2] = [&VARIANT_A, &VARIANT_B];

    #[test]
    fn welcome_renders_animation_on_first_draw() {
        let widget = WelcomeWidget::new(false, FrameRequester::test_dummy(), true);
        // Animation requires width >= MIN_ANIMATION_WIDTH (60) and height >= MIN_ANIMATION_HEIGHT (20)
        let area = Rect::new(0, 0, MIN_ANIMATION_WIDTH, MIN_ANIMATION_HEIGHT + 5);
        let mut buf = Buffer::empty(area);
        (&widget).render(area, &mut buf);

        let mut found = false;
        let mut last_non_empty: Option<u16> = None;
        for y in 0..area.height {
            for x in 0..area.width {
                if !buf[(x, y)].symbol().trim().is_empty() {
                    found = true;
                    last_non_empty = Some(y);
                    break;
                }
            }
        }

        assert!(found, "expected welcome animation to render characters");
        let measured_rows = last_non_empty.map(|v| v + 2).unwrap_or(0);
        // Account for vertical padding (1 line at top) plus animation content
        assert!(
            measured_rows >= MIN_ANIMATION_HEIGHT,
            "expected measurement to report at least {MIN_ANIMATION_HEIGHT} rows, got {measured_rows}"
        );
    }

    #[test]
    fn ctrl_dot_changes_animation_variant() {
        let mut widget = WelcomeWidget {
            is_logged_in: false,
            animation: AsciiAnimation::with_variants(FrameRequester::test_dummy(), &VARIANTS, 0),
            animations_enabled: true,
        };

        let before = widget.animation.current_frame();
        widget.handle_key_event(KeyEvent::new(KeyCode::Char('.'), KeyModifiers::CONTROL));
        let after = widget.animation.current_frame();

        assert_ne!(
            before, after,
            "expected ctrl+. to switch welcome animation variant"
        );
    }
}
