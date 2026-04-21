---
name: Performance
description: Performance guidelines for scanning and rendering
globs: ["src/provider/**/*.rs", "src/supervisor/**/*.rs", "src/ui/**/*.rs"]
---

# Performance Instructions

## JSONL Scanning

Providers read JSONL session files on every scan cycle (default 2s). With 500+ sessions:

- **Tail-read for large JSONL files** — Copilot `events.jsonl` can be multi-MB. For state detection only the last few lines matter. Seek to `file_len - 32KB`, skip the first partial line, then iterate forward. Never `read_to_string` a file that can exceed ~64KB. `read_to_string` is fine for small, bounded files (e.g., `workspace.yaml`, `plan.md`).
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

## Non-Blocking Background Scans

- Use an `AtomicBool` guard to prevent overlapping scans. The supervisor sets the flag before spawning a scan thread and clears it on completion. If the flag is already set, skip the cycle.
- Background scan threads must never hold a lock that the UI thread needs. Scan into a local `Vec<Session>`, then swap into shared state under a brief `Mutex` lock.
- Semantic indexing (embedding generation) uses `try_lock` on the shared model handle — if the UI thread holds the lock, the indexer skips that cycle rather than blocking.

## SessionViewModel Change Detection

- `SessionViewModel` tracks a content hash of the rendered session list. On each scan cycle, compute the new hash and compare — if unchanged, skip the `apply_filter()` / sort / re-render pipeline entirely.
- This avoids needless re-rendering when 500+ sessions are loaded but nothing changed.

## Progressive Provider Loading

- Providers load in parallel. Each provider's results are merged into the shared `SessionViewModel` as they arrive, not after all providers complete.
- The UI shows partial results immediately — the user sees Copilot sessions while Claude/Gemini are still scanning.

## Release Profile

The current profile optimizes for binary size (`opt-level = "z"`). This is intentional — the TUI is I/O bound, not CPU bound. Don't change to `opt-level = 3` unless profiling shows CPU bottlenecks.
