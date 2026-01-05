use strum::IntoEnumIterator;
use strum_macros::AsRefStr;
use strum_macros::EnumIter;
use strum_macros::EnumString;
use strum_macros::IntoStaticStr;

/// Commands that can be invoked by starting a message with a leading slash.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, EnumString, EnumIter, AsRefStr, IntoStaticStr,
)]
#[strum(serialize_all = "kebab-case")]
pub enum SlashCommand {
    // DO NOT ALPHA-SORT! Enum order is presentation order in the popup, so
    // more frequently used commands should be listed first.
    Model,
    Endpoint,
    Approvals,
    Skills,
    Review,
    ReviewFix,
    Loop,
    CancelLoop,
    New,
    Resume,
    Init,
    Compact,
    // Undo,
    Diff,
    Mention,
    Status,
    Ps,
    Mcp,
    Theme,
    ToggleMouseMode,
    Logout,
    Quit,
    Exit,
    Feedback,
    Rollout,
    TestApproval,
}

impl SlashCommand {
    /// User-visible description shown in the popup.
    pub fn description(self) -> &'static str {
        match self {
            SlashCommand::Feedback => "send logs to maintainers",
            SlashCommand::New => "start a new chat during a conversation",
            SlashCommand::Init => "create an AGENTS.md file with instructions for Codex",
            SlashCommand::Compact => "summarize conversation to prevent hitting the context limit",
            SlashCommand::Review => "review my current changes and find issues",
            SlashCommand::ReviewFix => "review my changes, fix issues, and re-check until clean",
            SlashCommand::Loop => "run a task in an autonomous loop until completion",
            SlashCommand::CancelLoop => "stop the current autonomous loop",
            SlashCommand::Resume => "resume a saved chat",
            // SlashCommand::Undo => "ask Codex to undo a turn",
            SlashCommand::Quit | SlashCommand::Exit => "exit Codex",
            SlashCommand::Diff => "show git diff (including untracked files)",
            SlashCommand::Mention => "mention a file",
            SlashCommand::Skills => "use skills to improve how Codex performs specific tasks",
            SlashCommand::Status => "show current session configuration and token usage",
            SlashCommand::Ps => "list active background terminals",
            SlashCommand::Model => "choose what model and reasoning effort to use",
            SlashCommand::Endpoint => "show or change the Azure OpenAI endpoint",
            SlashCommand::Approvals => "choose what Codex can do without approval",
            SlashCommand::Mcp => "list configured MCP tools",
            SlashCommand::Theme => "change the color theme",
            SlashCommand::ToggleMouseMode => "toggle mouse capture for native text selection",
            SlashCommand::Logout => "log out of Codex",
            SlashCommand::Rollout => "print the rollout file path",
            SlashCommand::TestApproval => "test approval request",
        }
    }

    /// Command string without the leading '/'. Provided for compatibility with
    /// existing code that expects a method named `command()`.
    pub fn command(self) -> &'static str {
        self.into()
    }

    /// Whether this command can be run while a task is in progress.
    pub fn available_during_task(self) -> bool {
        match self {
            SlashCommand::New
            | SlashCommand::Resume
            | SlashCommand::Init
            | SlashCommand::Compact
            // | SlashCommand::Undo
            | SlashCommand::Model
            | SlashCommand::Endpoint
            | SlashCommand::Approvals
            | SlashCommand::Review
            | SlashCommand::ReviewFix
            | SlashCommand::Loop
            | SlashCommand::Logout => false,
            SlashCommand::Diff
            | SlashCommand::Mention
            | SlashCommand::Skills
            | SlashCommand::Status
            | SlashCommand::Ps
            | SlashCommand::Mcp
            | SlashCommand::Theme
            | SlashCommand::ToggleMouseMode
            | SlashCommand::Feedback
            | SlashCommand::CancelLoop
            | SlashCommand::Quit
            | SlashCommand::Exit => true,
            SlashCommand::Rollout => true,
            SlashCommand::TestApproval => true,
        }
    }

    fn is_visible(self) -> bool {
        match self {
            SlashCommand::Rollout | SlashCommand::TestApproval => cfg!(debug_assertions),
            _ => true,
        }
    }
}

/// Return all built-in commands in a Vec paired with their command string.     
pub fn built_in_slash_commands() -> Vec<(&'static str, SlashCommand)> {
    SlashCommand::iter()
        .filter(|command| command.is_visible())
        .map(|c| (c.command(), c))
        .collect()
}

/// Return all recognized built-in command names, including non-displayed
/// aliases (useful for parsing / validation).
pub fn built_in_slash_commands_for_matching() -> Vec<(&'static str, SlashCommand)> {
    let mut commands = built_in_slash_commands();
    commands.extend([("review-and-fix", SlashCommand::ReviewFix)]);
    commands
}
