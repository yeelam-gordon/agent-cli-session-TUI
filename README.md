# Agent Session TUI

A terminal UI for managing agent CLI sessions — **Copilot CLI**, Claude Code, Codex CLI, and more.

## Pain Points Solved

- **Too many tabs** — see all sessions in one view with clear status badges
- **Which needs my input?** — 🟡 Waiting vs 🟢 Running vs 💤 Resumable at a glance
- **Resume after reboot** — session summaries, last activity, work state help you decide what to pick up
- **One place to start them all** — launch new sessions or resume old ones without memorizing flags

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│ TUI (ratatui + crossterm)                                   │
│  Session List  │  Session Detail  │  Activity Log           │
├─────────────────────────────────────────────────────────────┤
│ Supervisor (tokio background task)                          │
│  Discovery · Reconciliation · Process monitoring · Commands │
├─────────────────────────────────────────────────────────────┤
│ Provider Registry                                           │
│  Copilot CLI │ Claude Code │ Codex CLI │ (extensible)       │
├─────────────────────────────────────────────────────────────┤
│ SQLite (WAL mode) — persistent session tracking             │
└─────────────────────────────────────────────────────────────┘
```

### Multi-Axis State Model

Sessions are tracked across four independent axes, not a flat enum:

| Axis | Values |
|------|--------|
| **Process** | Running · Exited · Missing · StaleLock |
| **Interaction** | Busy · WaitingInput · Idle · Unknown |
| **Persistence** | Resumable · Ephemeral · Archived |
| **Health** | Clean · Crashed · Orphaned |

State is inferred from **multiple signals** (lock files, event streams, process liveness, timestamps) with a confidence rating.

## Keybindings

| Key | Action |
|-----|--------|
| `↑`/`↓` or `j`/`k` | Navigate sessions |
| `Enter` or `r` | Resume selected session |
| `n` | New session |
| `a` | Archive session |
| `R` | Force refresh |
| `Tab` | Switch panel focus |
| `q` / `Ctrl+C` | Quit |

## Configuration

Config lives at `~/.config/agent-session-tui/config.toml` (auto-created on first run):

```toml
db_path = "~/.local/share/agent-session-tui/sessions.db"
poll_interval_ms = 2000
log_max_lines = 500

[providers.copilot]
enabled = true
command = "copilot"
default_args = []
resume_flag = "--resume"

[providers.claude]
enabled = false
command = "claude"
default_args = []
resume_flag = "--continue"
```

## Adding a Provider

Implement the `Provider` trait in `src/provider/`:

```rust
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn key(&self) -> &str;
    fn capabilities(&self) -> ProviderCapabilities;
    fn discover_persisted_sessions(&self) -> Result<Vec<Session>>;
    fn discover_live_processes(&self) -> Result<Vec<(u32, Option<String>)>>;
    fn reconcile(&self, persisted: &mut [Session], live: &[(u32, Option<String>)]) -> Result<()>;
    fn read_session_metadata(&self, session: &Session) -> Result<SessionMetadata>;
    fn activity_sources(&self, session: &Session) -> Result<Vec<ActivitySource>>;
    fn build_resume_command(&self, session: &Session) -> Result<Vec<String>>;
    fn build_new_command(&self, cwd: &PathBuf) -> Result<Vec<String>>;
    fn collect_signals(&self, session: &Session) -> Result<StateSignals>;
    fn infer_state(&self, signals: &StateSignals) -> SessionState; // default impl provided
}
```

Register it in `main.rs` and add config in `config.toml`.

## Building

```bash
cargo build --release
# Binary at target/release/agent-session-tui(.exe)
```

## License

MIT
