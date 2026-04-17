---
description: 'How to write a new provider plugin for agent-session-tui'
applyTo: 'src/provider/**/*.rs'
---

# Writing a Provider Plugin

A provider plugin teaches the TUI how to discover, monitor, and launch sessions for a specific agent CLI (e.g., Copilot CLI, Claude Code, Codex CLI).

## Quick Start

1. **Create** `src/provider/<yourname>/mod.rs`
2. **Implement** the `Provider` trait (see below)
3. **Register** in `src/main.rs::create_provider()` — add one match arm
4. **Add config** section in `config.toml` under `[providers.<yourname>]`
5. **Write test** in `tests/<yourname>_lifecycle_test.rs` using the shared framework
6. **Build & test**: `cargo build --release && cargo test --test <yourname>_lifecycle_test -- --nocapture`

## The Provider Trait

Defined in `src/provider/mod.rs`. Every method you must implement:

```rust
pub trait Provider: Send + Sync {
    // Identity (required)
    fn name(&self) -> &str;                    // "My CLI"
    fn key(&self) -> &str;                     // "mycli" (matches config key)
    fn capabilities(&self) -> ProviderCapabilities;

    // Discovery (required) — DATA ONLY
    fn discover_sessions(&self) -> Result<Vec<Session>>;
    // Scan CLI state dir → return sessions with title, summary, timestamps

    fn match_processes(&self, sessions: &mut [Session]) -> Result<()>;
    // Find live OS processes, match to sessions, set pid + state

    // Detail (optional — have defaults)
    fn session_detail(&self, session: &Session) -> Result<SessionDetail>;
    fn activity_sources(&self, session: &Session) -> Result<Vec<ActivitySource>>;
    fn infer_state(&self, signals: &StateSignals) -> SessionState; // has default impl
}
// NOTE: No build_resume_command, build_new_command, or collect_signals.
// Launch/resume/kill are config-driven — owned by the supervisor framework.
```

## Session State Model

Sessions have four independent state axes:

| Axis | Values | Meaning |
|------|--------|---------|
| `ProcessState` | Running, Exited, Missing, StaleLock | Is the OS process alive? |
| `InteractionState` | Busy, WaitingInput, Idle, Unknown | What is the session doing? |
| `PersistenceState` | Resumable, Ephemeral, Archived | Can it be resumed? |
| `HealthState` | Clean, Crashed, Orphaned | Is it healthy? |

Plus `Confidence` (Low, Medium, High) and a `reason` string for diagnostics.

## Process Detection

**Use `src/process_info.rs`** — do NOT call sysinfo directly.

```rust
use crate::process_info::{discover_processes, extract_flag_value};

fn match_processes(&self, sessions: &mut [Session]) -> Result<()> {
    let procs = discover_processes("mycli"); // matches process name or command line
    let mut results = Vec::new();
    for (pid, entry) in &procs {
        let session_id = extract_flag_value(&entry.command_line, "--session-id");
        results.push((*pid, session_id));
    }
    // Match results to sessions by session ID, then set session.pid + session.state
    Ok(())
}
```

This uses WMI on Windows (reliable) with sysinfo fallback.

## Key Rules for Providers

1. **Data only** — providers discover and interpret sessions. Launch/resume/kill are handled by the framework from `config.toml`.
2. **Read-only** — never write to the agent CLI's state directory.
2. **UTF-8 safe** — always use `util::truncate_str_safe()` for string truncation. Session data can contain any Unicode.
3. **Skip empty sessions** — filter out sessions with no user interaction during discovery.
4. **File mtime for "last activity"** — don't rely on timestamps inside files. Use the file's modification time as the real-time activity indicator.
5. **Log diagnostics** — use `crate::log::info/warn/error()` for troubleshooting. Log file is next to the exe.
6. **Graceful degradation** — if a session file is corrupt or unreadable, skip it and continue. Never crash the TUI.

## Config Structure

Each provider gets a `[providers.<key>]` section in `config.toml`:

```toml
[providers.mycli]
enabled = true
command = "mycli"                    # bare name or full path
default_args = ["--some-flag"]       # always passed on new + resume
state_dir = 'C:\Users\me\.mycli'    # where sessions live on disk
resume_flag = "--resume"             # how to resume: <command> <args> <resume_flag> <session_id>
startup_dir = 'D:\'                  # default CWD for new sessions
launch_method = "wt"                 # "wt" | "cmd" | "pwsh"
wt_profile = "PowerShell"           # optional WT profile name
```

## Testing Your Plugin

### 1. Create test file: `tests/<yourname>_lifecycle_test.rs`

```rust
use agent_session_tui::config::AppConfig;
use agent_session_tui::provider::mycli::MyCliProvider;
use agent_session_tui::testing::TestRunner;
use agent_session_tui::testing::scenarios;

#[test]
fn mycli_lifecycle() {
    let config = AppConfig::load().expect("config");
    let pc = config.providers.get("mycli").expect("'mycli' not in config");
    let provider = MyCliProvider::new(pc);
    let mut runner = TestRunner::new("MyCLI");

    // Common scenarios work with any Provider
    scenarios::discover(&mut runner, &provider);
    scenarios::graceful(&mut runner, &provider);

    assert!(runner.summary(), "Tests failed");
}
```

### 2. Run tests

```bash
# Non-interactive (discover + graceful)
cargo test --test mycli_lifecycle_test -- --nocapture

# Interactive (launches a real session)
cargo test --test mycli_lifecycle_test -- --nocapture --scenario launch

# Kill test (kills a running session)
cargo test --test mycli_lifecycle_test -- --nocapture --scenario kill
```

### 3. What the shared scenarios validate

| Scenario | Tests |
|----------|-------|
| `discover` | Session count > 0, live processes found, reconcile produces correct state, running sessions have PIDs, waiting sessions have ≥Medium confidence |
| `graceful` | Clean-exited sessions are Resumable+Clean, orphaned sessions have no PID, resumable sessions have summaries |
| `launch` | Detects Running, Busy, WaitingInput transitions over 60s |
| `kill` | After kill: process not Running, session is Resumable or Orphaned |

## Existing Providers as Reference

| Provider | File | Data Sources |
|----------|------|-------------|
| Copilot CLI | `src/provider/copilot/mod.rs` | `workspace.yaml`, `events.jsonl`, `inuse.<pid>.lock` files, `plan.md` |
| Claude Code | `src/provider/claude/mod.rs` | `~/.claude/projects/<encoded-path>/<session-id>.jsonl` |

Study these for patterns on summary extraction, state inference, and edge case handling.
