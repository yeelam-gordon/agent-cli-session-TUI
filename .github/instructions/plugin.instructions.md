---
description: 'How to write a new provider plugin for agent-session-tui'
applyTo: 'src/provider/**/*.rs'
---

# Writing a Provider Plugin

A provider plugin teaches the TUI how to discover, monitor, and launch sessions for a specific agent CLI (e.g., Copilot CLI, Claude Code, Codex CLI, Qwen CLI, Gemini CLI).

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

    fn discover_sessions_paged(&self, offset: usize, limit: usize) -> Result<PagedSessions>;
    // Paginated variant — returns PagedSessions { sessions, total_count, has_more }.
    // The supervisor calls this for providers with 100+ sessions to avoid
    // loading everything into memory at once. Default impl delegates to
    // discover_sessions() and slices in-memory.

    fn match_processes(&self, sessions: &mut [Session]) -> Result<()>;
    // Find live OS processes, match to sessions, set pid + state

    // Detail (optional — have defaults)
    fn session_detail(&self, session: &Session) -> Result<SessionDetail>;
    fn activity_sources(&self, session: &Session) -> Result<Vec<ActivitySource>>;
    fn infer_state(&self, signals: &StateSignals) -> SessionState; // has default impl

    // Tab focus (optional — default returns None)
    fn tab_title(&self, session: &Session) -> Option<String>;
    // Extract the current terminal tab title from the session's log files.
    // Many CLIs dynamically set the terminal tab title via ANSI OSC escape
    // sequences (e.g., Copilot CLI's `report_intent` tool calls).
    // Return the **latest** title so the TUI can focus the correct WT tab
    // when the user presses Enter on a running session.
    // If your CLI does not set the tab title, leave the default (None).
    // When None, Enter on a running session is a no-op.
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
| `HealthState` | Clean, Crashed, Orphaned | Is it healthy? (all display as Resumable to user) |

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

## Tab Title Extraction (Optional)

Some agent CLIs dynamically set the terminal tab title to reflect their current activity (e.g., Copilot CLI emits `report_intent` tool calls). Implementing `tab_title()` enables the **Tab Focus** feature: when a user presses Enter on a Running/Waiting session, the TUI switches to the correct Windows Terminal tab.

**If your CLI sets the tab title:**
1. Override `fn tab_title(&self, session: &Session) -> Option<String>`
2. Parse the session's log/event files for the **latest** title-setting event
3. Return the title string (the TUI searches all WT tabs via UI Automation)

**If your CLI does NOT set the tab title:**
- Leave the default (`None`). Enter on a running session will show "Tab focus not available" instead of searching and failing.

**Tab title by provider:**
| Provider | Returns |
|----------|---------|
| Copilot CLI | `report_intent` value from last tool call in `events.jsonl` |
| Claude Code | `✳` static marker (Claude sets its own OSC title) |
| Gemini CLI | CWD folder name (no dynamic title) |
| Codex CLI | CWD folder name |
| Qwen CLI | CWD folder name |

**Example** (from the Copilot provider — tail-reads `events.jsonl`):
```rust
fn tab_title(&self, session: &Session) -> Option<String> {
    let dir = session.state_dir.as_ref()?;
    let path = dir.join("events.jsonl");
    // Tail-read: seek to last 32KB, not read_to_string on multi-MB file
    let file = std::fs::File::open(&path).ok()?;
    let len = file.metadata().ok()?.len();
    let tail_start = len.saturating_sub(32 * 1024);
    let mut reader = std::io::BufReader::new(file);
    std::io::Seek::seek(&mut reader, std::io::SeekFrom::Start(tail_start)).ok()?;
    let mut latest_intent: Option<String> = None;
    for line in std::io::BufRead::lines(reader).flatten() {
        if !line.contains("report_intent") { continue; }
        // Parse JSON: data.toolRequests[].arguments.intent
        // Update latest_intent with each match
    }
    latest_intent
}
```

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
| `kill` | After kill: process not Running, session is Resumable |

## Existing Providers as Reference

| Provider | File | Data Sources |
|----------|------|-------------|
| Copilot CLI | `src/provider/copilot/mod.rs` | `workspace.yaml`, `events.jsonl`, `inuse.<pid>.lock` files, `plan.md` |
| Claude Code | `src/provider/claude/mod.rs` | `~/.claude/projects/<encoded-path>/<session-id>.jsonl` |
| Codex CLI | `src/provider/codex/mod.rs` | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` |
| Qwen CLI | `src/provider/qwen/mod.rs` | `~/.qwen/projects/<encoded-path>/chats/<session-id>.jsonl` |
| Gemini CLI | `src/provider/gemini/mod.rs` | `~/.gemini/tmp/<project>/chats/session-*.jsonl` + subdirs |

Study these for patterns on summary extraction, state inference, and edge case handling.

## Response Extraction

Each CLI stores its last meaningful assistant response differently. Use these patterns when extracting summaries or task completion status:

| Provider | Where | Format |
|----------|-------|--------|
| Copilot CLI | `events.jsonl` | `assistant.message` content field; check `toolRequests` array for `task_complete` entries with `result.summary` |
| Claude Code | `<session>.jsonl` | `message.content[]` — array of text blocks, concatenate `.text` fields |
| Codex CLI | `rollout-*.jsonl` | `payload.content[]` — filter for `type: "output_text"`, read `.text` |
| Gemini CLI | `session-*.jsonl` | `content` field directly on the response object |
| Qwen CLI | `<session>.jsonl` | `message.parts[].text` — array of text parts |

**Copilot `task_complete` pattern:** The Copilot CLI signals task completion via a `task_complete` tool call in the `toolRequests` array of an assistant turn. Extract `arguments.result.summary` for a one-line task summary. This is more reliable than parsing the full assistant message.
