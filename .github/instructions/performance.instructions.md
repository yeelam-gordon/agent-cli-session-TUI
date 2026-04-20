---
name: Performance
description: Performance guidelines for scanning and rendering
globs: ["src/provider/**/*.rs", "src/supervisor/**/*.rs", "src/ui/**/*.rs"]
---

# Performance Instructions

## JSONL Scanning

Providers read JSONL session files on every scan cycle (default 2s). With 500+ sessions:

- **Avoid `read_to_string` for large files** — Copilot `events.jsonl` can be multi-MB. For state detection, only the last few lines matter. Use `BufReader` + seek to end for large files, or read only tail.
- **Don't scan the same file twice** — some providers re-scan in both `discover_sessions` and `match_processes`. Cache scan results within a single refresh cycle.
- **Skip unchanged files** — compare file mtime before re-reading. If mtime hasn't changed since last scan, reuse cached result.

## Supervisor Scan Loop

- All providers scan in parallel via `std::thread::scope` — keep it that way
- The scan runs every `poll_interval_ms` (default 2000ms). Don't reduce below 1000ms.
- Consider incremental scanning: only re-scan sessions whose state directory mtime changed

## TUI Rendering

- The render loop runs at 100ms (10 FPS). Ratatui's diff engine only sends changed cells.
- **Don't do expensive work in draw functions** — line padding, unicode width calculation etc. should be as cheap as possible
- If nothing changed (no supervisor events, no key presses), the diff will be empty — but we still call `terminal.draw()`. This is fine; ratatui handles it efficiently.
- **apply_filter()** lowercases every field on every keystroke — acceptable for <1000 sessions, but could cache lowercase versions if it becomes a bottleneck

## Release Profile

The current profile optimizes for binary size (`opt-level = "z"`). This is intentional — the TUI is I/O bound, not CPU bound. Don't change to `opt-level = 3` unless profiling shows CPU bottlenecks.
