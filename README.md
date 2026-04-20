# Agent CLI Session TUI

A terminal UI for managing agent CLI sessions — **Copilot CLI**, **Claude Code**, **Codex CLI**, **Qwen CLI**, **Gemini CLI**, and extensible to others.
<img width="2818" height="1608" alt="image" src="https://github.com/user-attachments/assets/28922190-474b-4019-be01-45d291954fe9" />

## Pain Points Solved

- **Too many tabs** — see all sessions in one view with clear status badges
- **Which needs my input?** — 🟡 Waiting vs 🟢 Running vs 💤 Resumable at a glance
- **Close without worry** — shut down any session anytime; all sessions are discoverable and resumable later
- **Resume after reboot** — session summaries, last activity, work state help you decide what to pick up
- **Fast search + resume** — `/` to search across title, summary, CWD, provider, then `Enter` to resume
- **One place for all agents** — manage Copilot, Claude, Codex, Qwen, Gemini sessions from a single TUI

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│ TUI (ratatui + crossterm)                                   │
│  Session List  │  Session Detail  │  Activity Log           │
├─────────────────────────────────────────────────────────────┤
│ Supervisor (tokio — parallel provider scans)                │
│  Discovery · Process matching · Launch/Resume (config-driven)│
├─────────────────────────────────────────────────────────────┤
│ Provider plugins (data-only — read from each CLI's state)   │
│  Copilot │ Claude │ Codex │ Qwen │ Gemini │ (extensible)   │
├─────────────────────────────────────────────────────────────┤
│ Process detection (WMI on Windows, sysinfo on Linux/macOS)  │
│ archived.json — simple list of hidden session IDs           │
└─────────────────────────────────────────────────────────────┘
```

No internal database. Providers read directly from each CLI's own state directory (read-only). All providers scan in parallel for fast refresh. The only file we write is `archived.json` for tracking hidden sessions.

### Multi-Axis State Model

Sessions are tracked across four independent axes:

| Axis | Values |
|------|--------|
| **Process** | Running · Exited · Missing · StaleLock |
| **Interaction** | Busy · WaitingInput · Idle · Unknown |
| **Persistence** | Resumable · Ephemeral · Archived |
| **Health** | Clean · Crashed · Orphaned |

User-facing states are simplified: 🟢 Running, 🟡 Waiting, 💤 Resumable.

## Keybindings

| Key | Action |
|-----|--------|
| `↑`/`↓` or `j`/`k` | Navigate sessions |
| `Enter` (⏎) | Open/resume selected session (in original CWD) |
| `n` | New session (launches default provider) |
| `a` | Archive session (instantly hidden) |
| `/` | Search (type to filter, `↑`/`↓` to browse, `Enter` to resume, `Esc` to cancel) |
| `Shift+Tab` | Toggle between active and archived view |
| `Tab` | Switch panel focus |
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
    // Optional: session_detail(), activity_sources(), infer_state()
}
```

Launch/resume/kill are handled by the framework from `config.toml`. Register your provider in `main.rs::create_provider()`.

## Building

```bash
cargo build --release
# Binary: target/release/agent-session-tui(.exe)
```

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
