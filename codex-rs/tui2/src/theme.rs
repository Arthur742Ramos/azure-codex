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

// ============================================================================
// Progress Bar Helpers
// ============================================================================

/// Render a visual progress bar using Unicode block characters.
///
/// # Arguments
/// * `percent` - Progress percentage (0-100)
/// * `width` - Total width of the bar in characters
///
/// # Returns
/// A vector of styled spans representing the progress bar
pub fn progress_bar(percent: i64, width: usize) -> Vec<Span<'static>> {
    let percent = percent.clamp(0, 100) as usize;
    let filled = (width * percent) / 100;
    let empty = width.saturating_sub(filled);

    // Create the filled portion with color based on percentage
    // Using Stylize trait methods for reliable color application
    let filled_str = "█".repeat(filled);
    let filled_span: Span<'static> = if percent >= 80 {
        Span::from(filled_str).red().bold()
    } else if percent >= 60 {
        Span::from(filled_str).magenta().bold()
    } else if percent >= 40 {
        Span::from(filled_str).cyan().bold()
    } else {
        Span::from(filled_str).green().bold()
    };

    vec![filled_span, Span::from("░".repeat(empty)).dim()]
}

/// Render a compact progress bar with percentage label.
///
/// Example: "████░░░░ 45%"
pub fn progress_bar_labeled(percent: i64, bar_width: usize) -> Vec<Span<'static>> {
    let mut spans = progress_bar(percent, bar_width);
    spans.push(Span::from(format!(" {percent}%")).dim());
    spans
}

/// Render a progress bar showing REMAINING percentage (for context window).
///
/// The filled portion represents what's LEFT, with color indicating health:
/// - Green: 60-100% remaining (healthy)
/// - Cyan: 40-59% remaining (moderate)
/// - Magenta: 20-39% remaining (warning)
/// - Red: 0-19% remaining (critical)
///
/// Example with 71% remaining: "███████░░░ 71% left"
pub fn progress_bar_remaining(percent_remaining: i64, width: usize) -> Vec<Span<'static>> {
    let percent = percent_remaining.clamp(0, 100) as usize;
    let filled = (width * percent) / 100;
    let empty = width.saturating_sub(filled);

    // Color based on how much is LEFT (inverse of usage-based coloring)
    let filled_str = "█".repeat(filled);
    let filled_span: Span<'static> = if percent >= 60 {
        Span::from(filled_str).green().bold()
    } else if percent >= 40 {
        Span::from(filled_str).cyan().bold()
    } else if percent >= 20 {
        Span::from(filled_str).magenta().bold()
    } else {
        Span::from(filled_str).red().bold()
    };

    vec![filled_span, Span::from("░".repeat(empty)).dim()]
}

/// Render context usage as a visual indicator.
///
/// Example: "Context: ████████░░░░ 67% (86k/128k)"
pub fn context_usage_bar(
    used_tokens: Option<i64>,
    total_tokens: Option<i64>,
    bar_width: usize,
) -> Vec<Span<'static>> {
    let (percent, label) = match (used_tokens, total_tokens) {
        (Some(used), Some(total)) if total > 0 => {
            let pct = ((used as f64 / total as f64) * 100.0).round() as i64;
            let used_k = format_tokens_short(used);
            let total_k = format_tokens_short(total);
            (pct, format!(" ({used_k}/{total_k})"))
        }
        (Some(used), None) => {
            let used_k = format_tokens_short(used);
            (0, format!(" ({used_k} used)"))
        }
        _ => (0, String::new()),
    };

    let mut spans = vec![Span::from("Context: ").dim()];
    spans.extend(progress_bar(percent, bar_width));
    spans.push(Span::from(format!(" {percent}%")).dim());
    if !label.is_empty() {
        spans.push(Span::from(label).dim());
    }
    spans
}

/// Format token count in compact form (e.g., "128k", "1.2M")
fn format_tokens_short(tokens: i64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{}k", tokens / 1_000)
    } else {
        tokens.to_string()
    }
}

// ============================================================================
// Box Border Characters
// ============================================================================

/// Unicode box drawing characters for consistent borders
pub mod border {
    /// Top-left corner
    pub const TL: &str = "╭";
    /// Top-right corner
    pub const TR: &str = "╮";
    /// Bottom-left corner
    pub const BL: &str = "╰";
    /// Bottom-right corner
    pub const BR: &str = "╯";
    /// Horizontal line
    pub const H: &str = "─";
    /// Vertical line
    pub const V: &str = "│";
    /// T-junction pointing down
    pub const T_DOWN: &str = "┬";
    /// T-junction pointing up
    pub const T_UP: &str = "┴";
    /// T-junction pointing right
    pub const T_RIGHT: &str = "├";
    /// T-junction pointing left
    pub const T_LEFT: &str = "┤";
    /// Cross junction
    pub const CROSS: &str = "┼";
}

// ============================================================================
// Tool Call Display Helpers
// ============================================================================

/// Create a tool header span with icon
pub fn tool_header(name: &str) -> Vec<Span<'static>> {
    let icon = match name.to_lowercase().as_str() {
        "read" | "readfile" => "📄",
        "search" | "grep" | "find" => "🔍",
        "write" | "edit" | "patch" => "✏️",
        "run" | "exec" | "bash" | "shell" => "▶",
        "web" | "fetch" | "http" => "🌐",
        _ => "⚙",
    };
    vec![
        Span::from(format!("{icon} ")).dim(),
        Span::styled(name.to_string(), tool_name_style()),
    ]
}

/// Create a collapsible indicator
pub fn collapse_indicator(expanded: bool) -> Span<'static> {
    if expanded { "▾".dim() } else { "▸".dim() }
}

/// Status indicator for tool calls
pub fn tool_status(completed: bool, success: bool) -> Span<'static> {
    if !completed {
        "●".cyan().bold() // Running
    } else if success {
        "✓".green().bold() // Success
    } else {
        "✗".red().bold() // Failed
    }
}

// ============================================================================
// Message Container Helpers
// ============================================================================

/// Create a role label for message containers
pub fn role_label(role: &str) -> Span<'static> {
    match role.to_lowercase().as_str() {
        "user" | "you" => Span::styled(" You ".to_string(), Style::default().bold()),
        "assistant" | "codex" | "azure codex" => Span::styled(
            " Azure Codex ".to_string(),
            Style::default().fg(COLOR_PRIMARY).bold(),
        ),
        "system" => Span::styled(" System ".to_string(), Style::default().dim()),
        _ => Span::styled(format!(" {role} "), Style::default().dim()),
    }
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
