# Contributing to Agent CLI Session TUI

Thanks for your interest in contributing! This project manages agent CLI sessions (Copilot, Claude, Codex, Qwen, Gemini) from a single TUI.

## Getting Started

1. **Fork and clone** the repo
2. **Read** [`AGENTS.md`](AGENTS.md) first — it covers project structure, build, test, and design decisions
3. **Build**: `cargo build`
4. **Test**: `cargo test --lib` (unit tests, no real session data needed)

## Development Workflow

1. Create a branch from `main`
2. Make your changes
3. Run `cargo build` — **zero warnings required**
4. Run `cargo test --lib` — all tests must pass
5. Open a PR against `main`

CI runs automatically on PRs: build + unit tests on both Ubuntu and Windows.

## Adding a Provider Plugin

See [`.github/instructions/plugin.instructions.md`](.github/instructions/plugin.instructions.md) for the full guide. Quick summary:

1. Create `src/provider/<name>/mod.rs` implementing the `Provider` trait
   - Required: `discover_sessions()`, `detect_state()`, `parse_log()`
   - Paginated: `discover_sessions_paged(offset, limit)` — default impl calls `discover_sessions()` with slicing; override for lazy discovery
   - Optional: `tab_title(session)` — return a custom tab label (e.g. branch name); defaults to `None`
2. Add match arm in `main.rs::create_provider()`
3. Add `pub mod <name>;` in `src/provider/mod.rs`
4. Add unit tests for state detection (waiting vs busy) with fixture JSONL data
5. Create `tests/<name>_lifecycle_test.rs` using the shared test framework

## Semantic Search Plugin

The optional semantic search plugin lives in `semantic-plugin/` (a separate Cargo crate that builds a cdylib DLL).

1. **Build**: `cd semantic-plugin && cargo build --release` — produces `semantic_search_plugin.dll` (Windows) / `libsemantic_search_plugin.so` (Linux) / `libsemantic_search_plugin.dylib` (macOS) in `semantic-plugin/target/release/`
2. **Install**: copy the DLL next to the TUI binary (same directory as `agent-session-tui(.exe)`); the TUI loads it at startup if found. Example:
   ```powershell
   Copy-Item semantic-plugin\target\release\semantic_search_plugin.dll target\release\
   ```
3. **Test**: `cd semantic-plugin && cargo test` — runs the plugin's own unit tests
4. **MSVC toolchain**: Windows builds require the MSVC toolchain (`rustup default stable-x86_64-pc-windows-msvc`); MinGW is not supported

## Search Module

The `src/search.rs` module handles fuzzy and semantic search across sessions. It contains 22 unit tests covering tokenisation, ranking, embedding cache I/O, and query parsing. Run them with:

```
cargo test --lib search
```

## Code Standards

- **Zero warnings** — `cargo build` must produce no warnings
- **Unit tests for state detection** — every provider must have tests verifying waiting/busy/idle states with fixture data
- **No mouse capture** — native terminal text selection must work
- **No `terminal.clear()` for redraw** — causes flicker
- **Unicode-safe** — use `unicode-width` for display width, never byte-index strings
- **UTF-8 safe** — use `truncate_str_safe()` for any string truncation

## What Makes a Good PR

- **One concern per PR** — don't mix bug fixes with features
- **Tests included** — unit tests for logic changes, especially state detection
- **No personal data** — config.toml is gitignored; don't commit paths or credentials
- **Docs updated** — if you change behavior, update README.md and AGENTS.md

## Reporting Issues

- **Bug reports**: include the session state you expected vs what you saw, and the provider name
- **Feature requests**: describe the use case, not just the solution
- **Security issues**: see [SECURITY.md](SECURITY.md) — do NOT open a public issue

## License

By contributing, you agree that your contributions will be licensed under the [MIT License](LICENSE).
