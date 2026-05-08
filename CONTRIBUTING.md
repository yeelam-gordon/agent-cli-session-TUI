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

See [`.github/instructions/plugin.instructions.md`](.github/instructions/plugin.instructions.md) for the full guide. Most providers are now **YAML-backed** via the `ConfigDrivenProvider` engine — you usually don't need to write Rust:

1. Create `providers/<name>.yaml` describing discovery, fields, and liveness rules
2. Add a `[providers.<name>]` section to `config.toml.template`
3. Add the provider name to `EXPECTED_PROVIDERS` in `scripts/validate_configs.py`
4. Create `tests/<name>_lifecycle_test.rs` using the shared test framework + `ConfigDrivenProvider::load_from_yaml`
5. Add scanner unit tests for state detection (waiting vs busy) — fixture JSONL inline

Only fall back to a dedicated `src/provider/<name>/mod.rs` when the YAML schema can't express the layout.

## Semantic Search Plugin

The optional semantic search plugin lives in `semantic-plugin/` (a separate Cargo crate that builds a cdylib DLL).

1. **Build**: `cd semantic-plugin && cargo build --release` — produces `semantic_search_plugin.dll` (Windows) / `libsemantic_search_plugin.so` (Linux) / `libsemantic_search_plugin.dylib` (macOS) in `semantic-plugin/target/release/`
2. **Install**: copy the DLL next to the TUI binary (same directory as `agent-session-tui(.exe)`); the TUI loads it at startup if found.
3. **Test**: `cd semantic-plugin && cargo test`
4. **MSVC toolchain**: Windows builds require the MSVC toolchain (`rustup default stable-x86_64-pc-windows-msvc`); MinGW is not supported.

## Search Module

`src/search.rs` handles fuzzy and semantic search across sessions. Embeddings are now **multi-vector per session** (base + compaction summaries + task-complete summaries + user messages), so a name buried in a long compacted conversation still matches. Full-text search lives in `src/log_search.rs` and indexes the same enriched content via Tantivy.

Run search-related tests with:

```
cargo test --lib search
```

## Code Standards

- **Zero clippy warnings** — `cargo clippy --release -- -D warnings` must be clean (CI runs this with `-D warnings`)
- **Unit tests for state detection** — every provider must have tests verifying waiting/busy/idle states with fixture data
- **Regression test for every bug fix** — see [`testing.instructions.md`](.github/instructions/testing.instructions.md) § Regression Test Policy
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
