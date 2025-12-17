//! Centralized theme and color constants for Azure Codex TUI.
//!
//! This module provides semantic color constants and style helpers to ensure
//! visual consistency across the application. All colors use ANSI codes for
//! broad terminal compatibility.
//!
//! ## Design Philosophy
//! - **Azure brand**: Cyan as primary accent (reflects Azure branding)
//! - **Semantic colors**: Green for success, Red for errors, Magenta for warnings
//! - **Contrast**: Bold for emphasis, Dim for secondary content
//! - **Accessibility**: High contrast ratios, no yellow (poor visibility)

use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::text::Span;

// ============================================================================
// Brand Colors (ANSI)
// ============================================================================

/// Primary brand color - Azure's signature cyan
pub const COLOR_PRIMARY: Color = Color::Cyan;

/// Secondary accent - Blue for variety
pub const COLOR_SECONDARY: Color = Color::Blue;

/// Tertiary accent - Magenta for special highlights
pub const COLOR_ACCENT: Color = Color::Magenta;

// ============================================================================
// Semantic Colors
// ============================================================================

/// Success states, confirmations, additions
pub const COLOR_SUCCESS: Color = Color::Green;

/// Error states, failures, deletions
pub const COLOR_ERROR: Color = Color::Red;

/// Warning states, caution indicators
pub const COLOR_WARNING: Color = Color::Magenta;

/// Informational content
pub const COLOR_INFO: Color = Color::Cyan;

// ============================================================================
// UI Element Colors
// ============================================================================

/// File paths and directory names
pub const COLOR_PATH: Color = Color::Cyan;

/// Shell prompts and command prefixes
pub const COLOR_SHELL_PROMPT: Color = Color::Magenta;

/// Tool names (Read, Search, Run, etc.)
pub const COLOR_TOOL_NAME: Color = Color::Cyan;

/// Search queries and patterns
pub const COLOR_QUERY: Color = Color::Green;

/// Line numbers in diffs and code
pub const COLOR_LINE_NUMBER: Color = Color::Blue;

/// Section headers and titles
pub const COLOR_HEADER: Color = Color::Cyan;

// ============================================================================
// Style Helpers
// ============================================================================

/// Style for the application title/brand
pub fn brand_style() -> Style {
    Style::default()
        .fg(COLOR_PRIMARY)
        .add_modifier(Modifier::BOLD)
}

/// Style for section headers
pub fn header_style() -> Style {
    Style::default()
        .fg(COLOR_HEADER)
        .add_modifier(Modifier::BOLD)
}

/// Style for success indicators
pub fn success_style() -> Style {
    Style::default()
        .fg(COLOR_SUCCESS)
        .add_modifier(Modifier::BOLD)
}

/// Style for error indicators
pub fn error_style() -> Style {
    Style::default()
        .fg(COLOR_ERROR)
        .add_modifier(Modifier::BOLD)
}

/// Style for file paths
pub fn path_style() -> Style {
    Style::default().fg(COLOR_PATH)
}

/// Style for tool names in exploration mode
pub fn tool_name_style() -> Style {
    Style::default().fg(COLOR_TOOL_NAME)
}

/// Style for shell prompt markers
pub fn shell_prompt_style() -> Style {
    Style::default().fg(COLOR_SHELL_PROMPT)
}

/// Style for search queries
pub fn query_style() -> Style {
    Style::default().fg(COLOR_QUERY)
}

/// Style for key binding hints (more visible than plain dim)
pub fn key_hint_style() -> Style {
    Style::default()
        .fg(COLOR_SECONDARY)
        .add_modifier(Modifier::BOLD)
}

/// Style for secondary/muted text
pub fn muted_style() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

/// Style for emphasized text
pub fn emphasis_style() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

// ============================================================================
// Span Helpers
// ============================================================================

/// Create a branded span (for app title)
pub fn brand_span(text: &str) -> Span<'static> {
    Span::styled(text.to_string(), brand_style())
}

/// Create a header span
pub fn header_span(text: &str) -> Span<'static> {
    Span::styled(text.to_string(), header_style())
}

/// Create a success indicator span
pub fn success_span(text: &str) -> Span<'static> {
    Span::styled(text.to_string(), success_style())
}

/// Create an error indicator span
pub fn error_span(text: &str) -> Span<'static> {
    Span::styled(text.to_string(), error_style())
}

/// Create a path span
pub fn path_span(text: &str) -> Span<'static> {
    Span::styled(text.to_string(), path_style())
}

/// Create a tool name span
pub fn tool_span(text: &str) -> Span<'static> {
    Span::styled(text.to_string(), tool_name_style())
}

/// Create a key hint span (styled key binding)
pub fn key_span(text: &str) -> Span<'static> {
    Span::styled(text.to_string(), key_hint_style())
}

/// Create a muted span
pub fn muted_span(text: &str) -> Span<'static> {
    Span::styled(text.to_string(), muted_style())
}

// ============================================================================
// Bullet/Indicator Helpers
// ============================================================================

/// Active/working indicator bullet
pub fn bullet_active() -> Span<'static> {
    "●".cyan().bold()
}

/// Success bullet
pub fn bullet_success() -> Span<'static> {
    "●".green().bold()
}

/// Error bullet
pub fn bullet_error() -> Span<'static> {
    "●".red().bold()
}

/// Neutral/pending bullet
pub fn bullet_neutral() -> Span<'static> {
    "○".dim()
}

/// Checkmark indicator
pub fn checkmark() -> Span<'static> {
    "✓".green().bold()
}

/// Cross indicator
pub fn crossmark() -> Span<'static> {
    "✗".red().bold()
}

/// Arrow indicator
pub fn arrow_right() -> Span<'static> {
    "→".cyan()
}

/// Selected item marker
pub fn selected_marker() -> Span<'static> {
    "●".cyan().bold()
}

/// Unselected item marker
pub fn unselected_marker() -> Span<'static> {
    "○".dim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brand_style_is_cyan_bold() {
        let style = brand_style();
        assert_eq!(style.fg, Some(Color::Cyan));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn success_style_is_green_bold() {
        let style = success_style();
        assert_eq!(style.fg, Some(Color::Green));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn error_style_is_red_bold() {
        let style = error_style();
        assert_eq!(style.fg, Some(Color::Red));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }
}
