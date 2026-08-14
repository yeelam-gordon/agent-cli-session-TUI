# AGENTS.md — Instructions for AI Agents Working on This Project

> Read this file first. Then read the instruction files it references.

## What This Project Is

A Rust TUI that manages agent CLI sessions (**Copilot CLI**, **Claude Code**, **Codex CLI**, **Qwen CLI**, **Gemini CLI**, **Kimi**, and extensible to others). It discovers sessions from each CLI's state directory, monitors running processes, and provides a unified view with search, resume, archive, thematic grouping, and optional AI-assisted group suggestions.

## Instruction Files

Read these before making changes. They are in `.github/instructions/`:

| File | Applies To | What It Covers |
|------|-----------|----------------|
| [`rust.instructions.md`](.github/instructions/rust.instructions.md) | `**/*.rs` | Rust conventions: error handling, string safety, process detection, TUI patterns |
| [`plugin.instructions.md`](.github/instructions/plugin.instructions.md) | `src/provider/**/*.rs` | How to write a new provider plugin |
| [`security.instructions.md`](.github/instructions/security.instructions.md) | `**/*.rs`, `**/*.toml` | Command injection, path traversal, sensitive data, dependency auditing |
| [`performance.instructions.md`](.github/instructions/performance.instructions.md) | `src/provider/**`, `src/supervisor/**`, `src/ui/**` | JSONL scanning, caching, render loop, release profile |
| [`testing.instructions.md`](.github/instructions/testing.instructions.md) | `**/*.rs`, `tests/**` | Test categories, required provider tests, CI quality gates |
| [`cross-platform.instructions.md`](.github/instructions/cross-platform.instructions.md) | `**/*.rs` | Path handling, process detection, terminal launch, Unix support |
| [`tab-focus.instructions.md`](.github/instructions/tab-focus.instructions.md) | `src/focus/**/*.rs` | Windows UI Automation for finding & focusing terminal tabs via COM |

## Project Structure

```
agent-session-tui/
├── .github/
│   ├── instructions/       # Copilot/agent instruction files (READ THESE)
│   └── workflows/          # CI (rust.yml) + Release (release.yml)
├── src/
│   ├── main.rs             # Entry point — config, provider registration, supervisor + TUI startup
│   ├── lib.rs              # Library re-exports (all pub mod) for use by tests
│   ├── config.rs           # TOML config loading (AppConfig, ProviderConfig, AcpConfig)
│   ├── models.rs           # Core types: Session, SessionState (4-axis), StateSignals
│   ├── archive.rs          # JSON-based archive store
│   ├── groups.rs           # GroupManager: assign sessions to thematic groups (groups.json)
│   ├── acp.rs              # Legacy grouping engine: spawns configured CLI one-shot (kills on timeout)
│   ├── grouping/
│   │   ├── mod.rs          # Grouping engine dispatch (remote | local | acp) + cluster expansion
│   │   ├── local.rs        # Zero-egress word-overlap clustering + heuristic names
│   │   └── remote.rs       # Client for the hosted tab auto-grouping service (default engine)
│   ├── log.rs              # File-based logging (%TEMP%/agent-session-tui.log)
│   ├── log_search.rs       # Tantivy full-text index over session content (head/tail/compaction/user)
│   ├── process_info.rs     # Shared process discovery (sysinfo + WMI fallback with cooldown)
│   ├── search.rs           # Tiered search: exact → fuzzy → semantic; multi-vector embedding cache
│   ├── util.rs             # UTF-8 safe string truncation
│   ├── focus/
│   │   ├── mod.rs          # Tab focus API (platform-gated)
│   │   └── win.rs          # Windows UI Automation: find & focus WT tabs via COM
│   ├── provider/
│   │   ├── mod.rs          # Provider trait + ProviderRegistry + default state inference
│   │   ├── config_driven/  # YAML-backed provider engine — ALL local providers use this
│   │   └── remote_json.rs  # Remote providers: one-shot (--dump-json) and streaming
│   ├── supervisor/mod.rs   # Background tokio task: parallel scan, reconcile, launch, archive
│   ├── testing/
│   │   ├── mod.rs          # TestRunner (shared by all provider tests)
│   │   └── scenarios.rs    # Provider-agnostic test scenarios (discover, graceful, launch, kill)
│   └── ui/mod.rs           # ratatui TUI: session list, detail, log viewer, search, groups, AI suggest
├── providers/              # YAML provider definitions (one per provider; config_driven engine reads these)
├── prompts/
│   └── group-suggest.md    # AI grouping prompt template (shipped next to exe in release zip)
├── semantic-plugin/        # Optional semantic search DLL (separate crate, ~26 MB with ONNX model)
├── tests/                  # One *_lifecycle_test.rs per provider (integration tests)
├── config.toml.template    # Source template with placeholders; release build substitutes per-OS defaults into dist/config.toml.example
├── scripts/
│   ├── generate_config.py  # Per-OS substitution (invoked by release workflow)
│   └── validate_configs.py # CI check: per-OS configs have the expected provider set
├── Cargo.toml              # Dependencies and build profile
└── rust-toolchain.toml     # Pins stable MSVC toolchain
```

## How to Build

Requires the **MSVC toolchain** on Windows — the `windows` crate (used for tab focus via UI Automation) needs MSVC link libraries. The `rust-toolchain.toml` pins `stable-x86_64-pc-windows-msvc` so `cargo` selects it automatically.

```bash
# Debug build (fast, for development)
cargo build

# Release build (optimized, ~1.1 MB binary)
cargo build --release

# On Windows with MSVC toolchain explicitly
cargo +stable-x86_64-pc-windows-msvc build --release
```

Output: `target/release/agent-session-tui.exe`

Config search order: next to exe → `%APPDATA%\agent-session-tui\config.toml` → built-in defaults.

## Pre-Push Check — MANDATORY

CI (`.github/workflows/rust.yml`) runs `cargo clippy -- -D warnings` on **BOTH** crates and treats any warning as an error. `cargo build` alone does NOT run clippy, so a local green build can still break CI. Rust toolchain updates (e.g. 1.95 added `not_unsafe_ptr_arg_deref`, `redundant_closure` tightening) routinely surface new lints.

**Update your toolchain first — a stale local `stable` silently defeats this whole check.** CI installs the *latest* stable, so an older local compiler will pass clippy on code CI rejects. This has already happened once: local 1.95.0 passed while CI on 1.97.1 failed on `question_mark` and `useless_borrows_in_formatting` in long-standing code.

```bash
rustup update stable
rustc --version   # must match (or exceed) what CI reports in its logs
```

**Then, before every `git push`, run BOTH commands and fix any findings:**

```bash
# Core crate — DO NOT pass --lib; CI checks lib + bin + tests
cargo clippy --release -- -D warnings

# Semantic plugin crate (separate workspace member)
cargo clippy --release --manifest-path semantic-plugin/Cargo.toml -- -D warnings

# Tests — CI runs `cargo test --lib`. Linux CI runner exposes Windows-path
# hardcoding in fixtures. Gate Windows-only tests with `#[cfg(windows)]`.
cargo test --lib
```

Both must exit 0. **Do not shortcut with `--lib`** — the bin target surfaces its own lints (notably `dead_code` on library-public API that `main.rs` doesn't call). CI runs plain `cargo clippy`, which defaults to all targets.

Note that CI runs clippy in **debug** (`cargo clippy`), not `--release`. The two normally agree, but if you are chasing a CI-only failure, reproduce with the exact CI command.

If clippy complains about a lint that is intentionally allowed for the situation (e.g. FFI raw-pointer args on `pub extern "C" fn`, or library-public API preserved for future use), scope an `#[allow(clippy::<lint>)]` or `#[allow(dead_code)]` to that item — do not globally silence it.

## How to Run Tests

```bash
# Unit tests only (100 tests — runs on CI)
cargo test --lib

# All tests including provider integration tests (needs real session data)
cargo test -- --nocapture

# Specific lifecycle integration test file
cargo test --test copilot_lifecycle_test -- --nocapture
cargo test --test claude_lifecycle_test -- --nocapture
cargo test --test codex_lifecycle_test -- --nocapture
cargo test --test qwen_lifecycle_test -- --nocapture
cargo test --test gemini_lifecycle_test -- --nocapture

# Single unit test by fully-qualified path (fastest iteration when fixing one test)
cargo test --lib provider::config_driven::tests::claude_yaml_end_to_end
cargo test --lib ui::ui_invariant_tests::empty_providers_marks_initial_load_complete

# All tests in a module (substring match on test path)
cargo test --lib provider::config_driven::tests       # every config_driven unit test
cargo test --lib ui_invariant_tests                   # every UI invariant test
cargo test --lib claude                               # anything with "claude" in its path

# Show test output (stdout) — helpful when debugging assertion failures
cargo test --lib <filter> -- --nocapture
```

Tests use the shared framework in `src/testing/`. Each `tests/*_lifecycle_test.rs` is a thin wrapper that builds its provider via `testing::load_provider("<key>", pc)` (which loads `providers/<key>.yaml`) and calls shared scenarios. They are compiled but **not run** on CI — they need real session data. Provider scanner tests (state detection with fixture JSONL) live in `src/provider/config_driven/mod.rs` under `#[cfg(test)]`.

## How to Add a New Provider

**Detailed guide**: [`.github/instructions/plugin.instructions.md`](.github/instructions/plugin.instructions.md)

Quick summary — providers are **YAML, not Rust**. You should not need to write
any Rust to add one:
1. Create `providers/<name>.yaml` describing discovery, extraction and state rules
2. Add a `[providers.<name>]` section in `config.toml` (command, resume flag, state_dir)
3. Add an end-to-end unit test in `src/provider/config_driven/mod.rs` (see
   `copilot_yaml_end_to_end` and friends) covering discovery + state detection
4. Optionally add `tests/<name>_lifecycle_test.rs` — copy an existing one; they
   all go through `testing::load_provider("<name>", pc)`
5. Run: `cargo test --lib provider::config_driven::tests` and, for the
   integration test, `cargo test --test <name>_lifecycle_test -- --nocapture`

`src/main.rs::create_provider()` resolves `providers/<key>.yaml` automatically —
there is no match arm to edit.

## Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| **Multi-axis state model** | Process, Interaction, Persistence, Health are independent axes — avoids ambiguous flat enums. User-facing display simplified to Running/Waiting/Resumable. |
| **SessionViewModel** | Merges scan results incrementally per-provider. Enables progressive loading — first page renders instantly, remaining pages load in background. |
| **AtomicBool scan guard** | Prevents overlapping scan cycles. Non-blocking — if a scan is already in progress the next poll is skipped rather than queued. |
| **Phased selection** | First page of all providers loads synchronously (fast initial render). Remaining pages load asynchronously and merge into the viewmodel without flicker. |
| **WMI for process detection** | sysinfo can't read command-line args for some Windows processes; WMI is reliable |
| **No internal DB** | We read from each CLI's own state (read-only). Only `archived.json` for hide/show. No sync issues. |
| **Parallel provider scans** | All providers scan concurrently via `std::thread::scope` for fast refresh |
| **Provider trait** | Each CLI is a plugin. Discovery, state inference, and launch are provider-specific. Common test scenarios validate any provider. |
| **File-based logging** | `%TEMP%/agent-session-tui.log`. Panics are logged with file:line before terminal restore. |
| **Remote-first grouping** | `[grouping] engine` defaults to `remote`. It sends session titles + cwd (as `file:///` URIs) for one representative per local cluster (measured: 40 sessions → 8 entries), never summaries or file contents, and falls back to `wordmatch` on any failure. `wordmatch` is word-overlap matching, **not** a model of any kind (it was called `local`, which misread as "local model"; that name still parses as an alias). |
| **Subprocess timeouts must kill** | `tokio::time::timeout` only drops the future; it cannot reap an OS process. `acp::wait_with_timeout` polls `try_wait` and calls `child.kill()` at the limit. Any future subprocess work must do the same. |

## Common Pitfalls

1. **UTF-8 string slicing** — Never use `&s[..N]`. Use `util::truncate_str_safe()`. Sessions contain Chinese, emoji, etc.
2. **crossterm key events** — Only handle `KeyEventKind::Press` on Windows (fires Press+Release+Repeat).
3. **ListState recreation** — Persist `ListState` across frames or scroll position jumps.
4. **Lock files** — Copilot sessions can have MULTIPLE lock files (stale + live). Check all, prefer live.
5. **Empty command lines** — sysinfo returns empty `cmd()` for some processes. Use `process_info.rs` instead.
6. **Regressions without tests** — Every bug fix MUST include a regression test. See [`testing.instructions.md`](.github/instructions/testing.instructions.md) § Regression Test Policy. No exceptions.

7. **Fixing one plugin in isolation** — A bug in one provider likely exists in others. See [`plugin.instructions.md`](.github/instructions/plugin.instructions.md) § Cross-Plugin Consistency Rule. Always audit all providers before closing a fix.

8. **Pushing without running clippy** — `cargo build` does not run clippy, but CI does, with `-D warnings`, on BOTH the core crate and `semantic-plugin/`. Always run both clippy commands (see § Pre-Push Check) before `git push`. Toolchain bumps (e.g. 1.95) frequently add new lints that were clean the day before.

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
| `src/search.rs` or `semantic-plugin/` | `README.md` semantic search section, `CONTRIBUTING.md` § Semantic Search Plugin (build + install steps) |
| `src/focus/` | `README.md` tab focus section |
| `src/testing/` (test framework) | `plugin.instructions.md` testing section, `AGENTS.md` how to test |
| `Cargo.toml` (deps, bin entries) | `AGENTS.md` how to build |
| Any file move or rename | `AGENTS.md` project structure, `lib.rs` exports, `main.rs` mod declarations |
| `src/ui/mod.rs` (keybindings) | `README.md` keybindings table |
| `src/provider/<name>/mod.rs` (bug fix) | All other providers for the same bug — see `plugin.instructions.md` § Cross-Plugin Consistency |

**Run the instruction audit after significant changes:**
Use a code-review agent to read `.github/instructions/*.md` + `AGENTS.md` and diff against the actual code. Fix both directions — code should match docs, and docs should match code.
