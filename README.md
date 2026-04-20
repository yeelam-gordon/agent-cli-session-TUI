# Agent CLI Session TUI

A terminal UI for managing agent CLI sessions — **Copilot CLI**, **Claude Code**, and extensible to others.
<img width="2818" height="1608" alt="image" src="https://github.com/user-attachments/assets/28922190-474b-4019-be01-45d291954fe9" />

## Pain Points Solved

- **Too many tabs** — see all sessions in one view with clear status badges
- **Which needs my input?** — 🟡 Waiting vs 🟢 Running vs 💤 Resumable at a glance
- **Close without worry** — shut down any session anytime; all sessions are discoverable and resumable later, no need to keep tabs open "just in case"
- **Resume after reboot** — session summaries, last activity, work state help you decide what to pick up
- **Fast search + resume** — `/` to search across all sessions by title, summary, CWD, then `Enter` → `r` to resume instantly
- **One place to start them all** — launch new sessions or resume old ones in their original working directory

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│ TUI (ratatui + crossterm)                                   │
│  Session List  │  Session Detail  │  Activity Log           │
├─────────────────────────────────────────────────────────────┤
│ Supervisor (tokio background task)                          │
│  Discovery · Process matching · Launch/Resume (config-driven)│
├─────────────────────────────────────────────────────────────┤
│ Provider plugins (data-only — read from each CLI's state)   │
│  Copilot CLI │ Claude Code │ (extensible via Provider trait)│
├─────────────────────────────────────────────────────────────┤
│ Process detection (WMI on Windows, sysinfo on Linux/macOS)  │
│ archived.json — simple list of hidden session IDs           │
└─────────────────────────────────────────────────────────────┘
```

No internal database. Providers read directly from each CLI's own state directory (read-only). The only file we write is `archived.json` for tracking hidden sessions.

### Multi-Axis State Model

Sessions are tracked across four independent axes:

| Axis | Values |
|------|--------|
| **Process** | Running · Exited · Missing · StaleLock |
| **Interaction** | Busy · WaitingInput · Idle · Unknown |
| **Persistence** | Resumable · Ephemeral · Archived |
| **Health** | Clean · Crashed · Orphaned |

State is inferred from **multiple signals** (lock files, event streams, process liveness, file timestamps) with a confidence rating.

## Keybindings

| Key | Action |
|-----|--------|
| `↑`/`↓` or `j`/`k` | Navigate sessions |
| `Enter` or `r` | Resume selected session (in original CWD) |
| `n` | New session |
| `a` | Archive session (instantly hidden) |
| `/` | Search (type to filter, `↑`/`↓` to browse, `Enter` to lock, `Esc` to clear) |
| `Shift+Tab` | Toggle between active and archived view |
| `Tab` | Switch panel focus |
| Mouse click | Select session |
| Mouse scroll | Navigate session list |
| `Esc` | Clear search filter |
| `q` / `Ctrl+C` | Quit |

## Configuration

Copy `config.toml.example` next to the binary and rename to `config.toml`:

```toml
poll_interval_ms = 2000
log_max_lines = 500

[providers.copilot]
enabled = true
command = "copilot"
default_args = []
state_dir = '~/.copilot/session-state'
resume_flag = "--resume"
# startup_dir = '/home/user/projects'
launch_method = "wt"    # "wt" (Windows Terminal) | "cmd" | "pwsh"

[providers.claude]
enabled = true
command = "claude"
default_args = []
state_dir = '~/.claude/projects'
resume_flag = "--resume"
launch_method = "wt"
```

Config search order: next to exe → `%APPDATA%/agent-session-tui/config.toml` → built-in defaults.

## Adding a Provider

See [`.github/instructions/plugin.instructions.md`](.github/instructions/plugin.instructions.md) for the full guide.

Implement the `Provider` trait (data-only — no launch/resume logic needed):

```rust
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn key(&self) -> &str;
    fn capabilities(&self) -> ProviderCapabilities;
    fn discover_sessions(&self) -> Result<Vec<Session>>;       // scan CLI state dir
    fn match_processes(&self, sessions: &mut [Session]) -> Result<()>; // match live processes
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
# Run all provider integration tests
cargo test -- --nocapture

# Run a specific provider's tests
cargo test --test copilot_lifecycle_test -- --nocapture
cargo test --test claude_lifecycle_test -- --nocapture
```

## For Contributors & AI Agents

Read [`AGENTS.md`](AGENTS.md) first — it covers project structure, how to build, how to add providers, key design decisions, and common pitfalls.

## License

MIT
