//! Syntax highlighting for code using syntect.
//!
//! Provides syntax-highlighted styled spans that can be combined with
//! diff highlighting (background colors for additions/deletions).

use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Span;
use std::path::Path;
use std::sync::OnceLock;
use syntect::highlighting::FontStyle;
use syntect::highlighting::HighlightState;
use syntect::highlighting::Highlighter;
use syntect::highlighting::RangedHighlightIterator;
use syntect::highlighting::Theme;
use syntect::highlighting::ThemeSet;
use syntect::parsing::ParseState;
use syntect::parsing::ScopeStack;
use syntect::parsing::SyntaxReference;
use syntect::parsing::SyntaxSet;

use crate::terminal_palette::best_color;
use crate::terminal_palette::is_light_background;

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME_DARK: OnceLock<Theme> = OnceLock::new();
static THEME_LIGHT: OnceLock<Theme> = OnceLock::new();

fn syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme_dark() -> &'static Theme {
    THEME_DARK.get_or_init(|| {
        let ts = ThemeSet::load_defaults();
        // Use "base16-ocean.dark" for a nice dark theme with good contrast
        ts.themes
            .get("base16-ocean.dark")
            .cloned()
            .unwrap_or_else(|| ts.themes.values().next().cloned().unwrap_or_default())
    })
}

fn theme_light() -> &'static Theme {
    THEME_LIGHT.get_or_init(|| {
        let ts = ThemeSet::load_defaults();
        // Use "base16-ocean.light" for light theme
        ts.themes
            .get("base16-ocean.light")
            .cloned()
            .unwrap_or_else(|| ts.themes.values().next().cloned().unwrap_or_default())
    })
}

fn current_theme() -> &'static Theme {
    if is_light_background() {
        theme_light()
    } else {
        theme_dark()
    }
}

/// Convert a syntect FontStyle to ratatui Modifier
fn font_style_to_modifier(style: FontStyle) -> Modifier {
    let mut modifier = Modifier::empty();
    if style.contains(FontStyle::BOLD) {
        modifier |= Modifier::BOLD;
    }
    if style.contains(FontStyle::ITALIC) {
        modifier |= Modifier::ITALIC;
    }
    if style.contains(FontStyle::UNDERLINE) {
        modifier |= Modifier::UNDERLINED;
    }
    modifier
}

/// Convert a syntect color to ratatui Color using best_color for terminal compatibility
fn syntect_color_to_ratatui(color: syntect::highlighting::Color) -> Color {
    best_color((color.r, color.g, color.b))
}

/// Find the syntax definition for a file based on its path
pub fn find_syntax_for_file(path: &Path) -> Option<&'static SyntaxReference> {
    let ss = syntax_set();

    // Try by extension first
    if let Some(ext) = path.extension().and_then(|e| e.to_str())
        && let Some(syntax) = ss.find_syntax_by_extension(ext)
    {
        return Some(syntax);
    }

    // Try by filename
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        // Handle common dotfiles
        match name {
            "Makefile" | "makefile" | "GNUmakefile" => {
                return ss.find_syntax_by_extension("make");
            }
            "Dockerfile" => {
                return ss.find_syntax_by_extension("dockerfile");
            }
            "Cargo.toml" | "Cargo.lock" => {
                return ss.find_syntax_by_extension("toml");
            }
            ".gitignore" | ".dockerignore" => {
                return ss.find_syntax_by_extension("gitignore");
            }
            _ => {}
        }
    }

    None
}

/// Highlight a single line of code with syntax highlighting.
///
/// Returns a vector of styled spans that can be combined with diff styling.
/// The returned spans will have foreground colors from syntax highlighting
/// but no background colors (caller should apply diff background).
pub fn highlight_line(
    line: &str,
    _syntax: &SyntaxReference,
    parse_state: &mut ParseState,
    highlight_state: &mut HighlightState,
) -> Vec<Span<'static>> {
    let ss = syntax_set();
    let theme = current_theme();
    let highlighter = Highlighter::new(theme);

    let ops = match parse_state.parse_line(line, ss) {
        Ok(ops) => ops,
        Err(_) => return vec![Span::raw(line.to_string())],
    };

    let iter = RangedHighlightIterator::new(highlight_state, &ops, line, &highlighter);

    let mut spans = Vec::new();
    for (style, text, _range) in iter {
        let fg = syntect_color_to_ratatui(style.foreground);
        let modifier = font_style_to_modifier(style.font_style);
        let ratatui_style = Style::default().fg(fg).add_modifier(modifier);
        spans.push(Span::styled(text.to_string(), ratatui_style));
    }

    if spans.is_empty() {
        vec![Span::raw(line.to_string())]
    } else {
        spans
    }
}

/// A stateful syntax highlighter for processing multiple lines of the same file.
pub struct LineHighlighter {
    syntax: &'static SyntaxReference,
    parse_state: ParseState,
    highlight_state: HighlightState,
}

impl LineHighlighter {
    /// Create a new highlighter for the given syntax
    pub fn new(syntax: &'static SyntaxReference) -> Self {
        let theme = current_theme();
        let highlighter = Highlighter::new(theme);
        Self {
            syntax,
            parse_state: ParseState::new(syntax),
            highlight_state: HighlightState::new(&highlighter, ScopeStack::new()),
        }
    }

    /// Highlight a single line, maintaining state for multi-line constructs
    pub fn highlight_line(&mut self, line: &str) -> Vec<Span<'static>> {
        highlight_line(
            line,
            self.syntax,
            &mut self.parse_state,
            &mut self.highlight_state,
        )
    }
}

/// Highlight a line without maintaining state (for single-line contexts)
#[allow(dead_code)]
pub fn highlight_line_standalone(line: &str, path: &Path) -> Vec<Span<'static>> {
    let Some(syntax) = find_syntax_for_file(path) else {
        return vec![Span::raw(line.to_string())];
    };

    let theme = current_theme();
    let highlighter = Highlighter::new(theme);
    let mut parse_state = ParseState::new(syntax);
    let mut highlight_state = HighlightState::new(&highlighter, ScopeStack::new());

    highlight_line(line, syntax, &mut parse_state, &mut highlight_state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn finds_rust_syntax() {
        let path = PathBuf::from("test.rs");
        let syntax = find_syntax_for_file(&path);
        assert!(syntax.is_some());
        assert_eq!(syntax.map(|s| s.name.as_str()), Some("Rust"));
    }

    #[test]
    fn finds_python_syntax() {
        let path = PathBuf::from("script.py");
        let syntax = find_syntax_for_file(&path);
        assert!(syntax.is_some());
        assert_eq!(syntax.map(|s| s.name.as_str()), Some("Python"));
    }

    #[test]
    fn finds_javascript_syntax() {
        let path = PathBuf::from("app.js");
        let syntax = find_syntax_for_file(&path);
        assert!(syntax.is_some());
        assert_eq!(syntax.map(|s| s.name.as_str()), Some("JavaScript"));
    }

    #[test]
    fn finds_makefile_syntax() {
        let path = PathBuf::from("Makefile");
        let syntax = find_syntax_for_file(&path);
        assert!(syntax.is_some());
    }

    #[test]
    fn highlight_rust_line() {
        let path = PathBuf::from("test.rs");
        let spans = highlight_line_standalone("fn main() { }", &path);
        // Should have multiple spans for keywords, identifiers, etc.
        assert!(!spans.is_empty());
    }

    #[test]
    fn highlight_unknown_extension_returns_plain() {
        let path = PathBuf::from("test.xyz123unknown");
        let spans = highlight_line_standalone("some random text", &path);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content.as_ref(), "some random text");
    }
}
