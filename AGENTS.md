# AGENTS.md — Instructions for AI Agents Working on This Project

> Read this file first. Then read the instruction files it references.

## What This Project Is

A Rust TUI that manages agent CLI sessions (Copilot CLI, Claude Code, and extensible to others). It discovers sessions from each CLI's state directory, monitors running processes, and provides a unified view with search, resume, and archive capabilities.

## Instruction Files

Read these before making changes. They are in `.github/instructions/`:

| File | Applies To | What It Covers |
|------|-----------|----------------|
| [`rust.instructions.md`](.github/instructions/rust.instructions.md) | `**/*.rs` | Rust conventions localized for this project: error handling, string safety, process detection, TUI patterns, testing |
| [`plugin.instructions.md`](.github/instructions/plugin.instructions.md) | `src/provider/**/*.rs` | How to write a new provider plugin: trait to implement, process detection, config, testing |

## Project Structure

```
agent-session-tui/
├── .github/instructions/   # Copilot/agent instruction files (READ THESE)
├── src/
│   ├── main.rs             # Entry point — config, provider registration, supervisor + TUI startup
│   ├── lib.rs              # Library re-exports (all pub mod) for use by tests
│   ├── config.rs           # TOML config loading (AppConfig, ProviderConfig)
│   ├── models.rs           # Core types: Session, SessionState (4-axis), StateSignals
│   ├── archive.rs          # JSON-based archive store
│   ├── log.rs              # File-based logging (next to exe)
│   ├── process_info.rs     # Shared process discovery (WMI on Windows + sysinfo fallback)
│   ├── util.rs             # UTF-8 safe string truncation
│   ├── provider/
│   │   ├── mod.rs          # Provider trait + ProviderRegistry + default state inference
│   │   ├── copilot/mod.rs  # Copilot CLI plugin
│   │   └── claude/mod.rs   # Claude Code plugin
│   ├── supervisor/mod.rs   # Background tokio task: scan, reconcile, launch, archive
│   ├── testing/
│   │   ├── mod.rs          # TestRunner (shared by all provider tests)
│   │   └── scenarios.rs    # Provider-agnostic test scenarios (discover, graceful, launch, kill)
│   └── ui/mod.rs           # ratatui TUI: session list, detail, log viewer, search, keybindings
├── tests/
│   ├── copilot_lifecycle_test.rs   # Copilot provider integration test
│   └── claude_lifecycle_test.rs    # Claude provider integration test
├── config.toml             # User config (lives next to exe at runtime)
├── Cargo.toml              # Dependencies and build profile
└── rust-toolchain.toml     # Pins stable MSVC toolchain
```

## How to Build

```bash
# Debug build (fast, for development)
cargo build

# Release build (optimized, ~1.9 MB binary)
cargo build --release

# On Windows with MSVC toolchain explicitly
cargo +stable-x86_64-pc-windows-msvc build --release
```

Output: `target/release/agent-session-tui.exe`

The `config.toml` file must be next to the exe (or at `%APPDATA%\agent-session-tui\config.toml`).

## How to Run Tests

```bash
# Run ALL integration tests (non-interactive scenarios only)
cargo test -- --nocapture

# Run a specific provider's tests
cargo test --test copilot_lifecycle_test -- --nocapture
cargo test --test claude_lifecycle_test -- --nocapture
```

Tests use the shared framework in `src/testing/`. Each test file is a thin wrapper that creates its provider and calls shared scenarios. See `plugin.instructions.md` for details.

## How to Add a New Provider

**Detailed guide**: [`.github/instructions/plugin.instructions.md`](.github/instructions/plugin.instructions.md)

Quick summary:
1. Create `src/provider/<name>/mod.rs` implementing the `Provider` trait
2. Add match arm in `src/main.rs::create_provider()`
3. Add `pub mod <name>;` in `src/provider/mod.rs`
4. Add `[providers.<name>]` section in `config.toml`
5. Create `tests/<name>_lifecycle_test.rs` using the shared test framework
6. Build and run: `cargo test --test <name>_lifecycle_test -- --nocapture`

## Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| **Multi-axis state model** | Process, Interaction, Persistence, Health are independent axes — avoids ambiguous flat enums |
| **WMI for process detection** | sysinfo can't read command-line args for some Windows processes; WMI is reliable |
| **No internal DB** | We read from each CLI's own state (read-only). Only an `archived.json` for hide/show. No sync issues. |
| **Provider trait** | Each CLI is a plugin. Discovery, state inference, and launch are provider-specific. Common test scenarios validate any provider. |
| **File-based logging** | `agent-session-tui.log` next to exe. Panics are logged with file:line before terminal restore. |

## Common Pitfalls

1. **UTF-8 string slicing** — Never use `&s[..N]`. Use `util::truncate_str_safe()`. Sessions contain Chinese, emoji, etc.
2. **crossterm key events** — Only handle `KeyEventKind::Press` on Windows (fires Press+Release+Repeat).
3. **ListState recreation** — Persist `ListState` across frames or scroll position jumps.
4. **Lock files** — Copilot sessions can have MULTIPLE lock files (stale + live). Check all, prefer live.
5. **Empty command lines** — sysinfo returns empty `cmd()` for some processes. Use `process_info.rs` instead.

## Self-Correction Rule

**When you change code, check if any documentation needs updating — and vice versa.**

This project has multiple agents and humans working on it. Stale docs cause real confusion. After any code change, verify:

| If you changed... | Then check... |
|-------------------|---------------|
| `src/provider/mod.rs` (Provider trait) | `plugin.instructions.md` trait reference, `AGENTS.md` structure |
| `src/provider/<name>/mod.rs` (a plugin) | That plugin's README if it exists, `plugin.instructions.md` examples |
| `src/models.rs` (state enums, Session struct) | `plugin.instructions.md` state model table, `AGENTS.md` design decisions |
| `src/config.rs` (ProviderConfig fields) | `plugin.instructions.md` config structure, `config.toml` example |
| `src/process_info.rs` | `rust.instructions.md` process detection section, `plugin.instructions.md` code example |
| `src/testing/` (test framework) | `plugin.instructions.md` testing section, `AGENTS.md` how to test |
| `Cargo.toml` (deps, bin entries) | `AGENTS.md` how to build |
| Any file move or rename | `AGENTS.md` project structure, `lib.rs` exports, `main.rs` mod declarations |
| `src/ui/mod.rs` (keybindings) | `README.md` keybindings table |

**Run the instruction audit after significant changes:**
Use a code-review agent to read `.github/instructions/*.md` + `AGENTS.md` and diff against the actual code. Fix both directions — code should match docs, and docs should match code.
