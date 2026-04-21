---
description: 'How the Tab Focus feature works — Windows UI Automation for switching to running agent tabs'
applyTo: 'src/focus/**/*.rs'
---

# Tab Focus — Implementation Guide

When the user presses `Enter` on a Running or Waiting session, the TUI focuses the existing Windows Terminal tab instead of launching a new one.

## How It Works

Uses native Windows UI Automation (COM-based, via the `windows` crate in `src/focus/win.rs`):

1. **Find WT windows** — searches for all `CASCADIA_HOSTING_WINDOW_CLASS` windows (covers Windows Terminal, Agentic Terminal, and any WT-based host)
2. **Find tabs** — enumerates descendant `TabItem` control type elements
3. **Match by name** — case-insensitive substring match against the tab's `Name` property
4. **Select** — calls `SelectionItemPattern::Select()` to switch to the tab
5. **Foreground** — calls `SetForegroundWindow`. Only calls `ShowWindow(SW_RESTORE)` if the window is minimized (`IsIconic` check), preserving maximized state

## Search Priority

The supervisor tries multiple search terms in order:

1. `tab_title` — extracted from the provider's logs (most accurate, matches what the tab actually shows)
2. Session title — from workspace.yaml or first user message
3. Short session ID — first 8 characters of the provider session ID

## Provider `tab_title()` Method

Each provider implements `tab_title(&self, session: &Session) -> Option<String>` to extract the current tab title:

| Provider | Source | Example |
|----------|--------|---------|
| Copilot | `report_intent` tool call in events.jsonl (tail-read) | `"Building interop tests"` |
| Claude | Static `✳` prefix | `"✳"` |
| Codex | CWD folder name | `"agent-session-tui"` |
| Qwen | CWD folder name | `"agent-session-tui"` |
| Gemini | CWD folder name | `"agent-session-tui"` |

If `tab_title()` returns `None`, Enter on a running session is a no-op (shows "Tab focus not available").

## Platform Support

- **Windows** — full support via `windows` crate COM UI Automation
- **Linux/macOS** — `focus_wt_tab()` is a no-op (returns `false`). Future: could support iTerm2, tmux, etc.

## Key Files

- `src/focus/mod.rs` — platform dispatch (`#[cfg(windows)]`)
- `src/focus/win.rs` — Windows implementation using `windows` crate
- `src/supervisor/mod.rs` — `handle_focus()` orchestrates search terms + fallback
- `src/provider/*/mod.rs` — each provider's `tab_title()` implementation

## Dependencies

```toml
# Only on Windows
[target.'cfg(windows)'.dependencies]
windows = { version = "0.62", features = [
    "Win32_UI_Accessibility",
    "Win32_UI_WindowsAndMessaging",
    "Win32_Foundation",
    "Win32_System_Com",
    "Win32_System_Ole",
    "Win32_System_Variant",
] }
```

Requires MSVC toolchain (`rustup override set stable-x86_64-pc-windows-msvc`).
