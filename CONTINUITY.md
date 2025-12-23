Goal (incl. success criteria):
- Maintain a compaction-safe session briefing in this repo via this ledger; success = entries stay current and the assistant uses it each turn.
- Identify and implement perf/UI/UX improvements; success = concrete improvements merged with basic validation (fmt/build/tests where practical).
- Keep the fork current with upstream (`openai/codex`); success = `upstream/main` merged into `main` and pushed.
- Keep CI green; success = all GitHub Actions checks pass on `origin/main`.

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
- Now:
  - CI is green on `origin/main`; ready to take the next perf/UI/UX improvement target.
- Next:
  - Decide next improvement surface (TUI, core, or JS CLI).
  - (Optional) Investigate/flakiness-reduce non-blocking CI test failures on Linux/macOS.
  - After CI is green, pick next perf/UI/UX target surface.

Open questions (UNCONFIRMED if needed):
- Which surface should be prioritized: Rust TUI (`codex-rs/tui`), Rust core, or JS CLI (`codex-cli`)?

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
- `codex-rs/tui2/src/pager_overlay.rs`
- `codex-rs/tui2/src/snapshots/codex_tui2__pager_overlay__tests__static_overlay_snapshot_basic.snap`
- `codex-rs/tui2/src/snapshots/codex_tui2__pager_overlay__tests__static_overlay_wraps_long_lines.snap`
- `codex-rs/core/tests/suite/otel.rs`
- `codex-rs/core/tests/suite/compact_resume_fork.rs`
- `codex-rs/justfile`
- Commands: `just fmt` / `just fix -p codex-tui2` / `just fix -p codex-core` / `cargo test --all-features` (in `codex-rs`)
  - Merge: `git fetch upstream`, `git merge upstream/main`
- CI: `gh run list`, `gh run view --log-failed`
