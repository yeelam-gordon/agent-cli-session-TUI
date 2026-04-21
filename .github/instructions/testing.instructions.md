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

## Search & Semantic Plugin Tests

- **22 search unit tests** in `src/search/mod.rs` cover the tiered ranking system: exact matches, prefix matches, fuzzy matches, keyword scoring, and combined ranking. These run on CI.
- **5 semantic plugin unit tests** in `src/search/semantic_plugin.rs` cover cosine similarity, embedding normalization, score thresholds, cache hit/miss, and graceful fallback when the model is unavailable.
- **Future:** `tab_title()` extraction and `discover_sessions_paged()` pagination need dedicated unit tests per provider. Track these as test debt.

## Regression Test Policy

**Every bug fix MUST include a test that would have caught the bug.**

When a regression or bug is discovered and fixed:

1. **Write a unit test first** that reproduces the bug (red), then verify the fix makes it pass (green).
2. **Name the test descriptively** — e.g., `test_search_query_not_in_title_bar` rather than `test_fix_123`.
3. **If unit testing is impractical** (e.g., a UI layout issue), add a functional or integration test instead — but always add *something*.
4. **CI must run the test** — if the test can't run on CI (needs real data, hardware, etc.), document why and add the closest possible approximation that *can* run on CI.
5. **Reference the fix** — add a comment in the test linking to the commit or issue, e.g., `// Regression: ed3f64b — search query was rendering in top title bar instead of list block title`.

This is non-negotiable. A bug without a regression test is a bug that will come back.

## What NOT to Test

- Don't test ratatui rendering output (too brittle)
- Don't test process detection with real processes (environment-dependent)
- Integration tests that need real CLI session data should NOT run on CI
