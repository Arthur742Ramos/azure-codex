Goal (incl. success criteria):
- Maintain a compaction-safe session briefing in this repo via this ledger; success = entries stay current and the assistant uses it each turn.
- Identify and implement perf/UI/UX improvements; success = concrete improvements merged with basic validation (fmt/build/tests where practical).
- Make terminal native scrollbar/scrollback available by default; success = inline TUI default (no alternate screen) with documented opt-in behavior.
- Remove in-app transcript scrolling; success = transcript is emitted into terminal scrollback and the main UI no longer has internal scrolling/scrollbars.
- Ensure transcript is lossless; success = no user/assistant messages are dropped/hidden after scrollback/flush changes.
- Keep the fork current with upstream (`openai/codex`); success = `upstream/main` merged into `main` and pushed.
- Keep CI green; success = all GitHub Actions checks pass on `origin/main`.

Constraints/Assumptions:
- Follow `AGENTS.md` instructions; for `codex-rs/` changes, obey Rust formatting/lint/test rules and avoid `CODEX_SANDBOX_*` env var code changes.

Key decisions:
- Use `CONTINUITY.md` as the canonical session briefing; begin assistant replies with a brief Ledger Snapshot.
- Default UX: run TUI inline (avoid alternate screen) unless users opt in.
- Default `tui.disable_mouse_capture` to `true` so mouse wheel scroll uses the terminal's native scrollback by default.
- When mouse capture is disabled (inline mode), emit transcript into terminal scrollback and render only the bottom pane.
- Avoid a fixed bottom pane height; auto-size the bottom pane viewport when possible (terminal scrollback transcript mode).

State:
- Done:
  - Added Continuity Ledger instructions to `AGENTS.md`.
  - Created initial `CONTINUITY.md`.
  - Improved `codex-rs/tui2` status indicator redraw scheduling (lower idle CPU when animations are off; no redraw loop while paused).
  - Improved onboarding trust screen UX for non-git folders (added a Git tip) and removed the related TODO.
  - Improved `/diff` performance and resilience (bounded output size, capped untracked diffs, limited git process concurrency) in `codex-rs/tui2/src/get_git_diff.rs`.
  - Improved `/diff` UX (shows transient "Computing diff..." status, avoids markdown-in-overlay message, caps overlay lines) in `codex-rs/tui2/src/chatwidget.rs` and `codex-rs/tui2/src/app.rs`.
  - Improved overlay UX/perf (Esc closes static overlays; removed per-render string allocation in key hints) in `codex-rs/tui2/src/pager_overlay.rs`.
  - Updated TUI2 snapshots for static overlay footer hints.
  - Fixed failing `codex-core` tests in `codex-rs/core/tests/suite/otel.rs` and `codex-rs/core/tests/suite/compact_resume_fork.rs`.
  - Validation: `just fmt`, `just fix -p codex-tui2`, `just fix -p codex-core`, `cargo test -p codex-tui2`, `cargo test -p codex-core --test all --all-features`, `cargo test --all-features` (all passed; `just fmt` warns about `imports_granularity=Item` needing nightly).
  - Committed and pushed to `origin/main`: `6ea071550` ("tui2: perf/ux improvements").
  - Merged `upstream/main` into `main`: `3f92e7179` (resolved delete/modify conflicts by keeping upstream `codex-rs/tui` files).
  - Post-merge checks: `cargo test -p codex-core --test all --all-features` and `cargo test -p codex-tui2` (passed).
  - Pushed upstream merge to `origin/main`: `4e5278a1c`.
  - Fixed `rust-ci` clippy failure on non-Windows (gate `mouse_capture_enabled` capture behind `#[cfg(windows)]`) and validated `cargo test -p codex-tui2` (pass).
  - Committed and pushed CI fixes to `origin/main`: `185931b5f` ("ci: fix tui2 clippy and release workflows").
  - Fixed release workflow validation issues and pushed: `eaa9387aa` ("ci: fix release workflow validation").
  - Fixed `rust-ci` install-action version pinning warnings and pushed: `bacc9e5ce` ("ci: fix install-action version pinning").
  - CI status (latest): `rust-ci` run success on `origin/main`; `rust-release-prepare` workflow_dispatch run success (no-ops when `CODEX_OPENAI_API_KEY` is unset).
  - Fixed `Release NPM Package` release asset collisions and pushed: `6134b6d3d` ("ci: fix release asset name collisions").
  - Re-ran `Release NPM Package` for `v0.2.1` (workflow_dispatch) and confirmed GitHub release now includes per-target assets (e.g., `codex-x86_64-unknown-linux-musl`, `codex-x86_64-pc-windows-msvc.exe`).
  - Added an in-app vertical scrollbar for the main transcript viewport (and aligned selection/copy with the reserved column) in `codex-rs/tui2/src/app.rs`; validated with `just fmt` and `cargo test -p codex-tui2`.
  - Added `tui.use_alternate_screen` config (defaults to `false` / inline) and wired `codex-rs/tui2` to skip entering the alternate screen for the main session; updated docs and ran `just fmt` + `cargo check -p codex-core -p codex-tui2`.
  - Built a debug CLI binary for local testing: `cargo build -p codex-cli` (outputs `codex-rs/target/debug/codex.exe` on Windows).
  - Implemented terminal-scrollback transcript mode when running inline with mouse capture disabled (flushes history lines into terminal scrollback; main UI renders only bottom pane; disables in-app scrolling inputs); rebuilt debug binary.
  - Removed the fixed 12-line bottom viewport in scrollback mode (auto-sizes to `ChatWidget::desired_height`); ran `just fmt` and rebuilt `codex-cli` debug binary.
  - Defaulted `tui.disable_mouse_capture` to `true` (all terminals) so mouse wheel scrolling uses terminal scrollback by default; updated docs and rebuilt `codex-cli` debug binary.
  - Changed scrollback transcript mode to write transcript lines into the terminal's real scrollback (full-screen scroll) instead of internal viewport scrolling; rebuilt `codex-cli` debug binary.
  - Checked upstream (`openai/codex`): `codex-rs/tui2/src/lib.rs` still enters alternate screen unconditionally for the main session (no config toggle upstream), so native scrollbar/scrollback behavior depends on terminal settings (e.g., "scrollback in alternate screen" support).
- Now:
  - Investigate report that scrollback-mode transcript is "eating" recent messages (user can't see their last question).
- Next:
  - User validates that the default inline TUI restores terminal native scrollbar/scrollback.
  - User validates that transcript history is scrollable via terminal scrollback (no in-app scrolling).
  - Fix any transcript loss/visibility bugs in scrollback transcript mode; add focused coverage if feasible; re-validate.
  - (Optional) Decide whether overlays (diff/transcript) should also respect `tui.use_alternate_screen`.
  - Ask user before running `cargo test -p codex-tui2` / `cargo test -p codex-core` for additional validation if desired.
  - Consider whether overlays should also default inline (or stay alternate-screen only for full-screen views).

Open questions (UNCONFIRMED if needed):
- Which surface should be prioritized: Rust TUI (`codex-rs/tui`), Rust core, or JS CLI (`codex-cli`)?
- Should overlays (diff/transcript) also respect `tui.use_alternate_screen`, or just the main session?
- Should overlays also default to inline (no alternate screen), or keep alternate screen for full-screen views?
- Should scrollback-mode bottom viewport be capped (min/max), or fully auto-size to `desired_height`?

Working set (files/ids/commands):
- `AGENTS.md`
- `CONTINUITY.md`
- `codex-rs/tui2/src/tui.rs`
- `.github/workflows/rust-release-prepare.yml`
- `.github/workflows/release-npm.yml`
- `codex-rs/`
- `codex-cli/`
- `codex-rs/tui2/src/status_indicator_widget.rs`
- `codex-rs/tui2/src/onboarding/trust_directory.rs`
- `codex-rs/tui2/src/onboarding/onboarding_screen.rs`
- `codex-rs/tui2/src/get_git_diff.rs`
- `codex-rs/tui2/src/chatwidget.rs`
- `codex-rs/tui2/src/chatwidget/tests.rs`
- `codex-rs/tui2/src/app.rs`
- `codex-rs/tui2/src/app_backtrack.rs`
- `codex-rs/tui2/src/lib.rs`
- `codex-rs/tui2/src/tui.rs`
- `codex-rs/tui2/docs/tui_viewport_and_history.md`
- `codex-rs/core/src/config/types.rs`
- `codex-rs/core/src/config/mod.rs`
- `docs/config.md`
- `docs/example-config.md`
- Commands: `just fmt`, `cargo test -p codex-tui2` (in `codex-rs`)
- Commands: `cargo build` (debug; in `codex-rs`)
- `codex-rs/tui2/src/pager_overlay.rs`
- `codex-rs/tui2/src/snapshots/codex_tui2__pager_overlay__tests__static_overlay_snapshot_basic.snap`
- `codex-rs/tui2/src/snapshots/codex_tui2__pager_overlay__tests__static_overlay_wraps_long_lines.snap`
- `codex-rs/core/tests/suite/otel.rs`
- `codex-rs/core/tests/suite/compact_resume_fork.rs`
- `codex-rs/justfile`
- Commands: `just fmt` / `just fix -p codex-tui2` / `just fix -p codex-core` / `cargo test --all-features` (in `codex-rs`)
  - Merge: `git fetch upstream`, `git merge upstream/main`
- CI: `gh run list`, `gh run view --log-failed`
