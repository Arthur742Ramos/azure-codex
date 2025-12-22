Goal (incl. success criteria):
- Maintain a compaction-safe session briefing in this repo via this ledger; success = entries stay current and the assistant uses it each turn.
- Identify and implement perf/UI/UX improvements; success = concrete improvements merged with basic validation (fmt/build/tests where practical).

Constraints/Assumptions:
- Follow `AGENTS.md` instructions; for `codex-rs/` changes, obey Rust formatting/lint/test rules and avoid `CODEX_SANDBOX_*` env var code changes.

Key decisions:
- Use `CONTINUITY.md` as the canonical session briefing; begin assistant replies with a brief Ledger Snapshot.

State:
- Done:
  - Added Continuity Ledger instructions to `AGENTS.md`.
  - Created initial `CONTINUITY.md`.
  - Improved `codex-rs/tui2` status indicator redraw scheduling (lower idle CPU when animations are off; no redraw loop while paused).
  - Improved onboarding trust screen UX for non-git folders (added a Git tip) and removed the related TODO.
  - Ran `just fmt` (warnings about `imports_granularity=Item` needing nightly, but succeeded).
  - Ran `cargo test -p codex-tui2` (passed).
  - Improved `/diff` performance and resilience (bounded output size, capped untracked diffs, limited git process concurrency) in `codex-rs/tui2/src/get_git_diff.rs`.
  - Improved `/diff` UX (shows transient “Computing diff...” status, avoids markdown-in-overlay message, caps overlay lines) in `codex-rs/tui2/src/chatwidget.rs` and `codex-rs/tui2/src/app.rs`.
  - Improved overlay UX/perf (Esc closes static overlays; removed per-render string allocation in key hints) in `codex-rs/tui2/src/pager_overlay.rs`.
  - Updated TUI2 snapshots for static overlay footer hints.
  - Ran `just fmt` and `cargo test -p codex-tui2` (passed).
- Now:
  - Running final checks (lint/tests) before committing and pushing changes.
- Next:
  - Run `just fix -p codex-tui2` (lint autofix), then re-run relevant tests.
  - Commit changes with an appropriate message.
  - Push to the current branch's remote.

Open questions (UNCONFIRMED if needed):
- Which surface should be prioritized: Rust TUI (`codex-rs/tui`), Rust core, or JS CLI (`codex-cli`)?
- What commit message style is preferred (single combined commit vs split commits)?

Working set (files/ids/commands):
- `AGENTS.md`
- `CONTINUITY.md`
- `codex-rs/`
- `codex-cli/`
- `codex-rs/tui2/src/status_indicator_widget.rs`
- `codex-rs/tui2/src/onboarding/trust_directory.rs`
- `codex-rs/tui2/src/onboarding/onboarding_screen.rs`
- `codex-rs/tui2/src/get_git_diff.rs`
- `codex-rs/tui2/src/chatwidget.rs`
- `codex-rs/tui2/src/chatwidget/tests.rs`
- `codex-rs/tui2/src/app.rs`
- `codex-rs/tui2/src/pager_overlay.rs`
- `codex-rs/tui2/src/snapshots/codex_tui2__pager_overlay__tests__static_overlay_snapshot_basic.snap`
- `codex-rs/tui2/src/snapshots/codex_tui2__pager_overlay__tests__static_overlay_wraps_long_lines.snap`
- `codex-rs/justfile`
- Commands: `just fmt` (in `codex-rs`), `cargo test -p codex-tui2` (in `codex-rs`)
