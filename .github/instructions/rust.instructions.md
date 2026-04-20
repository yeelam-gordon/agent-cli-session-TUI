---
description: 'Rust coding conventions localized for agent-session-tui'
applyTo: '**/*.rs'
---

# Rust Conventions for agent-session-tui

Based on [The Rust Book](https://doc.rust-lang.org/book/) and [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/).

## Project-Specific Rules

### Error Handling
- Use `anyhow::Result` for application-level errors (main, supervisor, providers).
- Use `Option<T>` when data may be absent (session metadata, lock files).
- **Never panic** on user data — session files may be corrupt, truncated, or have unexpected encoding. Always handle gracefully.
- Use `crate::log::error()` / `crate::log::warn()` for diagnostics (log file at `%TEMP%/agent-session-tui.log`).

### String Handling (Critical)
- **Never slice strings by byte index** (`&s[..200]`). Always use `is_char_boundary()` or the `util::truncate_str_safe()` helper. Session data contains multi-byte characters (Chinese, emoji, etc.).
- When displaying session titles/summaries in the TUI, always truncate through `truncate_str_safe()`.

### Process Detection
- Use `src/process_info.rs` for all process discovery. It uses WMI on Windows (reliable command-line reading) with sysinfo fallback.
- **Do not call sysinfo directly** in provider code — sysinfo can't read command-line args for some processes on Windows.
- Use `process_info::extract_flag_value()` to parse flags from command lines.

### Provider Plugin Pattern
- Each provider lives in `src/provider/<name>/mod.rs` and implements the `Provider` trait from `src/provider/mod.rs`.
- Providers are registered in `main.rs::create_provider()` — one match arm per provider.
- Providers read data **read-only** from the agent CLI's own state directory. We never write to another tool's state.
- Provider config comes from `config.toml` via `ProviderConfig` struct.

### TUI (ratatui + crossterm)
- Only handle `KeyEventKind::Press` — Windows fires Press+Release+Repeat events.
- Persist `ListState` across frames to avoid scroll jumps.
- All draw methods that need stateful widgets take `&mut self`.
- The panic hook must restore terminal state (LeaveAlternateScreen + cursor::Show).

### Testing
- Integration tests live in `tests/` (one per provider).
- Shared test framework is in `src/testing/` (TestRunner + provider-agnostic scenarios).
- Test binaries are thin wrappers: create provider → call shared scenarios.
- Tests run with: `cargo test --test <name> -- --nocapture`

## General Rust Style

- Use `cargo fmt` before committing.
- Use `cargo clippy` to catch common mistakes.
- Prefer `?` over `unwrap()`. The only acceptable `unwrap()` is on values guaranteed by construction.
- Use iterators over index-based loops.
- Prefer `&str` parameters over `String` when ownership isn't needed.
- Keep `main.rs` minimal — logic goes into modules.
- All public items in `lib.rs` modules should have doc comments (`///`).

## Commit Messages
- Include `Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>` trailer when Copilot generates the code.
