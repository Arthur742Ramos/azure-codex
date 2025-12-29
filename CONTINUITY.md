Goal (incl. success criteria):
- Improve TUI `/loop` behavior; success = (1) loop stops on task errors by default, (2) queued user input cancels loop instead of interleaving, (3) completion phrase detection is more reliable, (4) README documents `/loop` + `/cancel-loop`.
- Evaluate (and improve if needed) the TUI slash command the user calls “review-and-fix”; success = command name/UX/documentation match expectations and behavior is correct.
- Ensure changes are CI-ready before commit/push; success = local checks match CI expectations, and there are no obvious workflow failures pending.
- Maintain a compaction-safe session briefing in this repo via this ledger; success = entries stay current and the assistant uses it each turn.

Constraints/Assumptions:
- Follow `AGENTS.md` instructions; for `codex-rs/` changes, obey Rust formatting/lint/test rules and avoid `CODEX_SANDBOX_*` env var code changes.
- Sandbox is danger-full-access; approval_policy is never.

Key decisions:
- Use `CONTINUITY.md` as the canonical session briefing; begin assistant replies with a brief Ledger Snapshot.

State:
  - Done:
  - (Prior work) Implemented Anthropic tool_use/tool_result adjacency handling + regression tests.
  - (Prior work) Added signed-thinking support + tests/docs; adjusted request builder to avoid invalid signed thinking blocks; ran `just fmt`; built debug `codex-cli`.
  - Implemented TUI `/loop` improvements (cancel on error, cancel on queued input, completion phrase detection uses streamed output buffer).
  - Fixed slash command tab-complete ranking so `/c` favors `/compact` over `/cancel-loop`.
  - Updated the affected chatwidget snapshot and ran `cargo test -p codex-tui2` (PASS).
  - Ran `codex-exec` smoke tests for GPT + Claude; MCP `cloudbuild` startup failed with AADSTS90009 but the prompts still completed.
  - Now:
  - Ensure `/review-and-fix` works as an alias for `/review-fix` without impacting popup display.
  - Fix user-visible edge case: dispatching a bare slash command should not accidentally carry image attachments into a later message (removed unsafe fast-path).
  - Next:
  - If user wants: run `just fix -p codex-tui2` (requires explicit OK per `AGENTS.md`).
  - After commit/push: monitor GitHub Actions `rust-ci`/`codespell`/`cargo-deny` outcomes (UNCONFIRMED: exact required checks).

Open questions (UNCONFIRMED if needed):
- Confirmed: user is asking about the TUI slash command `/loop`.
- Confirmed: `/review-and-fix` is now supported as an alias for `/review-fix`.
- Do you expect loop to stop on any `EventMsg::Error`, or keep trying until max iterations?

Working set (files/ids/commands):
- `AGENTS.md`
- `CONTINUITY.md`
- `codex-rs/tui2/src/chatwidget.rs`
- `codex-rs/tui2/src/bottom_pane/command_popup.rs`
- `codex-rs/tui2/src/bottom_pane/chat_composer.rs`
- `codex-rs/tui2/src/bottom_pane/mod.rs`
- `codex-rs/tui2/src/slash_command.rs`
- `codex-rs/tui2/src/chatwidget/tests.rs`
- `codex-rs/tui2/src/chatwidget/snapshots/codex_tui2__chatwidget__tests__deltas_then_same_final_message_are_rendered_snapshot.snap`
- `README.md`
