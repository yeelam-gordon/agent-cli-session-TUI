# Agent CLI Session TUI

A terminal UI for managing agent CLI sessions — **Copilot CLI**, **Claude Code**, **Codex CLI**, **Qwen CLI**, **Gemini CLI**, and extensible to others.
<img width="2818" height="1608" alt="image" src="https://github.com/user-attachments/assets/28922190-474b-4019-be01-45d291954fe9" />

## Pain Points Solved

- **Where is my running agent?** — press `Enter` on any 🟡 Waiting or 🟢 Running session to instantly focus its terminal tab
- **Too many tabs** — see all sessions in one view with clear status badges
- **Which needs my input?** — 🟡 Waiting vs 🟢 Running vs 💤 Resumable at a glance
- **Finding that one session** — `/` to search with tiered ranking: exact match → fuzzy word match → ✨ semantic similarity (optional). Results ranked by relevance, not just recency
- **Close without worry** — shut down any session anytime; all sessions are discoverable and resumable later
- **Resume after reboot** — session summaries, last activity, full last response help you decide what to pick up
- **One place for all agents** — manage Copilot, Claude, Codex, Qwen, Gemini sessions from a single TUI

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│ TUI (ratatui + crossterm)                                   │
│  Session List  │  Session Detail  │  Activity Log           │
├─────────────────────────────────────────────────────────────┤
│ SessionViewModel (incremental merge, phased loading)        │
│ Supervisor (tokio — parallel provider scans, AtomicBool)    │
│  Discovery · Process matching · Launch/Resume (config-driven)│
├─────────────────────────────────────────────────────────────┤
│ Provider plugins (data-only — read from each CLI's state)   │
│  Copilot │ Claude │ Codex │ Qwen │ Gemini │ (extensible)   │
├─────────────────────────────────────────────────────────────┤
│ Search (exact → fuzzy → semantic via optional DLL plugin)   │
│ Tab Focus (Windows UI Automation — focus running WT tabs)   │
│ Process detection (WMI on Windows, sysinfo on Linux/macOS)  │
│ archived.json — simple list of hidden session IDs           │
└─────────────────────────────────────────────────────────────┘
```

No internal database. Providers read directly from each CLI's own state directory (read-only). All providers scan in parallel for fast refresh (non-blocking — `AtomicBool` scan guard prevents overlapping scans). The `SessionViewModel` merges results incrementally per-provider for progressive loading. The only file we write is `archived.json` for tracking hidden sessions.

### Multi-Axis State Model

Sessions are tracked across four independent axes:

| Axis | Values |
|------|--------|
| **Process** | Running · Exited · Missing · StaleLock |
| **Interaction** | Busy · WaitingInput · Idle · Unknown |
| **Persistence** | Resumable · Ephemeral · Archived |
| **Health** | Clean · Crashed · Orphaned |

Internally tracked for diagnostics; user-facing states are simplified to just: 🟢 Running, 🟡 Waiting, 💤 Resumable.

## Keybindings

| Key | Action |
|-----|--------|
| `↑`/`↓` or `j`/`k` | Navigate sessions |
| `Enter` (⏎) | Resume selected session — focuses the WT tab if Running, launches otherwise |
| `n` | New session (launches default provider) |
| `a` | Archive session (instantly hidden) |
| `/` | Search (type to filter, `↑`/`↓` to browse, `Enter` to resume, `Esc` to cancel) |
| `Shift+Tab` | Toggle between active and archived view |
| `Tab` | Switch panel focus (works for all 5 providers) |
| `PgUp`/`PgDn` | Scroll detail panel |
| `Esc` | Cancel search |
| `q` / `Ctrl+C` | Quit |

Native mouse text selection works (click-drag to highlight and copy).

## Supported Providers

| Provider | State Dir | Session Format |
|----------|-----------|----------------|
| **Copilot CLI** | `~/.copilot/session-state/` | `workspace.yaml` + `events.jsonl` + lock files |
| **Claude Code** | `~/.claude/projects/` | `<encoded-cwd>/<session-id>.jsonl` |
| **Codex CLI** | `~/.codex/sessions/` | Session directories with state files |
| **Qwen CLI** | `~/.qwen/projects/` | `<encoded-cwd>/chats/<session-id>.jsonl` |
| **Gemini CLI** | `~/.gemini/tmp/` | `<project>/chats/session-*.jsonl` + subdirs |

## Configuration

Copy `config.toml.example` next to the binary and rename to `config.toml`:

```toml
data_dir = '~/.local/share/agent-session-tui'
poll_interval_ms = 2000
log_max_lines = 500

[providers.copilot]
enabled = true
default = true          # 'n' launches this provider
command = "copilot"
default_args = []
state_dir = '~/.copilot/session-state'
resume_flag = "--resume"
launch_method = "wt"    # "wt" | "wtai" | "pwsh" | "cmd"
launch_fallback = "cmd" # optional — fallback if primary not found

[providers.claude]
enabled = true
command = "claude"
default_args = []
state_dir = '~/.claude/projects'
resume_flag = "--resume"
launch_method = "wt"
```

For full control over launch commands, use custom launcher fields:

```toml
launch_cmd = "wtai"
launch_args = ["-w", "0", "new-tab", "--startingDirectory", "{cwd}", "cmd", "/k", "{command}"]
launch_fallback_cmd = "wt"
launch_fallback_args = ["-w", "0", "new-tab", "--startingDirectory", "{cwd}", "cmd", "/k", "{command}"]
```

Placeholders: `{cwd}` → working directory, `{command}` → the agent CLI command.

Config search order: next to exe → `%APPDATA%/agent-session-tui/config.toml` → built-in defaults.

## Semantic Search

Search uses a three-tier ranking system: **exact substring** → **fuzzy word** → **semantic similarity**. The semantic tier is an optional DLL plugin (`semantic_search.dll` / `.so` / `.dylib`) that adds meaning-aware matching using cached embeddings.

- Results with a semantic boost show a ✨ indicator in the search list
- Embeddings are pre-computed and cached per session — no embedding during search
- Status bar shows 🧠 when the semantic plugin is loaded and ready
- If the DLL is missing, search falls back gracefully to exact + fuzzy only

The plugin lives in `semantic-plugin/` and is built separately (see [Release Packages](#release-packages)).

## Tab Focus

When you press `Enter` on a **Running** session, the TUI focuses the existing Windows Terminal tab instead of launching a new one. This uses native Windows UI Automation (COM-based, via the `windows` crate):

1. Finds all `CASCADIA_HOSTING_WINDOW_CLASS` windows (WT + Agentic Terminal)
2. Searches descendant `TabItem` elements for a name match
3. Selects the tab via `SelectionItemPattern` and brings the window to foreground

Tab names are extracted by each provider's `tab_title()` method (e.g., Copilot uses `report_intent` tool calls). On non-Windows platforms, `focus_wt_tab()` is a no-op.

## Release Packages

| Package | Size | Contents |
|---------|------|----------|
| **Core** | ~1.1 MB | `agent-session-tui` binary only |
| **Semantic** | ~26 MB | Core + `semantic_search` DLL + ONNX model |

Built for **x64** and **arm64** across all three platforms (Windows, Linux, macOS).

## Adding a Provider

See [`.github/instructions/plugin.instructions.md`](.github/instructions/plugin.instructions.md) for the full guide.

Implement the `Provider` trait (data-only — no launch/resume logic needed):

```rust
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn key(&self) -> &str;
    fn capabilities(&self) -> ProviderCapabilities;
    fn discover_sessions(&self) -> Result<Vec<Session>>;
    fn match_processes(&self, sessions: &mut [Session]) -> Result<()>;
    // Optional: discover_sessions_paged(), session_detail(), activity_sources(),
    //           infer_state(), tab_title()
}
```

Launch/resume/kill are handled by the framework from `config.toml`. Register your provider in `main.rs::create_provider()`.

## Building

Requires the **MSVC toolchain** on Windows (for the `windows` crate used by tab focus):

```bash
cargo build --release
# Binary: target/release/agent-session-tui(.exe)
```

The `rust-toolchain.toml` pins `stable-x86_64-pc-windows-msvc` automatically.

## Testing

```bash
# Unit tests only (runs on CI)
cargo test --lib

# All tests including provider integration tests (needs real session data)
cargo test -- --nocapture

# Specific provider
cargo test --test copilot_lifecycle_test -- --nocapture
cargo test --test claude_lifecycle_test -- --nocapture
cargo test --test qwen_lifecycle_test -- --nocapture
cargo test --test gemini_lifecycle_test -- --nocapture
cargo test --test codex_lifecycle_test -- --nocapture
```

## For Contributors & AI Agents

Read [`AGENTS.md`](AGENTS.md) first — it covers project structure, how to build, how to add providers, key design decisions, and common pitfalls.

## License

MIT
