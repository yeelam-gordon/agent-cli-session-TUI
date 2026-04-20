---
name: Testing
description: Testing standards and practices
globs: ["**/*.rs", "tests/**/*.rs"]
---

# Testing Instructions

## Test Categories

| Category | Location | Runs on CI | Purpose |
|----------|----------|-----------|---------|
| Unit tests | `src/**/*.rs` `#[cfg(test)]` | ✅ `cargo test --lib` | State detection, data parsing, utilities |
| Integration tests | `tests/*_lifecycle_test.rs` | ❌ needs real data | Provider discovery with real session dirs |
| UI invariant tests | `src/ui/mod.rs` | ✅ | Source-level checks preventing regressions |

## Required Tests for Every Provider

Each provider MUST have unit tests in its `mod.rs` under `#[cfg(test)]`:

1. **`scan_detects_waiting_for_user`** — fixture JSONL where assistant responded last → `waiting_for_user == true`
2. **`scan_detects_assistant_working`** — fixture JSONL where user sent last message → `assistant_working == true`
3. **Provider-specific edge cases** — e.g., Gemini's "Request cancelled" info events, Claude's array content format

Currently missing: **Copilot** and **Codex** have no scanner unit tests. Add them.

## Fixture Data

- Create test JSONL data inline in tests using helper functions (see `write_jsonl` pattern in claude/qwen/gemini tests)
- Use `std::env::temp_dir()` with unique names per test to avoid conflicts
- Always clean up: `let _ = fs::remove_dir_all(&dir);` at end of test
- Consider using `tempfile::TempDir` for automatic RAII cleanup (not yet a dependency)

## CI Quality Gates

Current CI runs `cargo build` + `cargo test --lib`. Consider adding:

- `cargo clippy -- -D warnings` — catches common mistakes
- `cargo audit` — checks for known dependency vulnerabilities
- `cargo fmt --check` — enforces consistent formatting

## What NOT to Test

- Don't test ratatui rendering output (too brittle)
- Don't test process detection with real processes (environment-dependent)
- Integration tests that need real CLI session data should NOT run on CI
