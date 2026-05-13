# Agent CLI Session TUI

**Agent view for every CLI** — manage background agent sessions across **Copilot CLI**, **Claude Code**, **Codex CLI**, **Qwen CLI**, **Gemini CLI**, **Kimi**, and more, in a single terminal screen.

![Agent CLI Session TUI demo](docs/agentTUI.gif)

> **Similar to `claude agents`, but cross-CLI.** Claude Code's [Agent view](https://code.claude.com/docs/en/agent-view) gives you one screen for parallel background Claude sessions — what's working, what needs input, what's done. This project gives you the same view across **all** your agent CLIs at once: Copilot CLI, Claude Code, Codex CLI, Qwen, Gemini, Kimi. Same idea — *parallel agents, one supervisor process, one screen* — applied beyond a single vendor.

## Pain Points Solved

- **Where is my running agent?** — press `Enter` on any 🟡 *Needs input* or 🟢 *Working* session to attach / focus its terminal tab
- **Too many tabs** — see every background session in one view with clear status badges
- **Which needs my input?** — 🟡 *Needs input* vs 🟢 *Working* vs 💤 *Resumable* at a glance
- **Finding that one session** — `/` to search with tiered ranking: exact match → fuzzy word match → ✨ semantic similarity (optional). Now indexes head, tail, compaction summaries, and your own messages — names buried inside long conversations show up in results
- **Hundreds of sessions piling up** — assign sessions to thematic groups with `g`, view by group via `Shift+Tab`. Optional [AI auto-suggest](#ai-auto-grouping) proposes groups for you
- **Close without worry** — shut down any session anytime; all sessions are discoverable and resumable later
- **Resume after reboot** — session summaries, last activity, full last response help you decide what to pick up
- **One place for all agents** — manage Copilot, Claude, Codex, Qwen, Gemini, Kimi sessions from a single agent-view-style TUI

## Agent view, multiplied

[Claude Code's Agent view](https://code.claude.com/docs/en/agent-view) (`claude agents`) introduced a clean idea: one screen for every background Claude session you've dispatched — grouped by *Working*, *Needs input*, *Completed* — with peek-and-reply and attach. This project applies that same idea **across every agent CLI you use**, and fills gaps that Claude's view doesn't cover.

| Capability                                       | `claude agents` (verified against [docs](https://code.claude.com/docs/en/agent-view)) | This TUI |
|--------------------------------------------------|------------------------------|----------|
| Single screen for parallel agent sessions        | ✅ Claude only               | ✅ Copilot · Claude · Codex · Qwen · Gemini · Kimi |
| Group by state (Working / Needs input / Completed) | ✅                         | ✅ (🟢 / 🟡 / 💤) |
| Attach to a running session                      | ✅ inline transcript         | ✅ focuses the existing Windows Terminal tab |
| Dispatch new sessions                            | ✅ from view                 | ✅ `n` per provider |
| Background supervisor                            | ✅ Claude's supervisor       | ✅ tokio-based supervisor, parallel provider scans |
| Pin / reorder / rename sessions                  | ✅                           | — (use thematic groups instead) |
| Pull-request status dots                         | ✅                           | — |
| Auto-generated row summaries                     | ✅ Haiku-class model         | ✅ extracted from each CLI's own metadata + last response (no extra model calls) |
| **🔍 Content search across transcripts**         | ❌ filter only — by agent name (`a:`), state (`s:`), or PR number (`#`); no content search across transcripts | ✅ exact + fuzzy + optional semantic — searches titles, summaries, compaction summaries, AND your own messages buried in long transcripts |
| **🏷️ Thematic groups (user-defined names)**     | ❌ groups only by state or by directory (`Ctrl+S`); within a group you can pin / reorder / rename | ✅ press `g` to assign any session to a named group (e.g., "auth-rewrite", "perf-investigation"); browse by group via `Shift+Tab` |
| **🤖 AI-suggested thematic groups**              | ❌                           | ✅ optional ACP-driven auto-grouping — analyzes your sessions and proposes thematic clusters you can accept / edit / dismiss |
| No vendor lock-in                                | —                            | ✅ data-only providers, read each CLI's own state |

The bottom three rows matter most when you have hundreds of accumulated sessions across multiple CLIs. Claude's Agent view assumes you remember what you dispatched recently or know the PR number; this TUI assumes you have a year of sessions and need to find one by something you said inside it.

If you've enjoyed `claude agents` and wished it covered your other agent CLIs too — Copilot CLI, Codex CLI, Qwen, Gemini, Kimi — and if you've ever needed to *find* a session by something you said three weeks ago rather than scrolling, this tool fills both gaps.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│ TUI (ratatui + crossterm)                                   │
│  Session List  │  Session Detail  │  Activity Log           │
│  Search (exact → fuzzy → semantic)  │  Tab Focus            │
├─────────────────────────────────────────────────────────────┤
│ SessionViewModel (incremental merge, phased loading)        │
│ Supervisor (tokio — parallel provider scans, non-blocking)  │
│  Discovery · Process matching · Launch/Resume (config-driven)│
├─────────────────────────────────────────────────────────────┤
│ Provider plugins (data-only — read from each CLI's state)   │
│  Copilot │ Claude │ Codex │ Qwen │ Gemini │ Kimi │ (more…) │
├─────────────────────────────────────────────────────────────┤
│ Shared infrastructure                                       │
│  Process detection │ Semantic DLL (optional) │ Archive store │
└─────────────────────────────────────────────────────────────┘
```

No internal database. Providers read directly from each CLI's own state directory (read-only). All providers scan in parallel for fast refresh. The `SessionViewModel` merges results incrementally per-provider for progressive loading.

### Session States

At a glance, every session shows one of three states. The mapping mirrors Claude Code's [Agent view](https://code.claude.com/docs/en/agent-view) terminology so users familiar with `claude agents` feel at home:

| Badge | This TUI    | Equivalent in `claude agents` | Meaning |
|-------|-------------|-------------------------------|---------|
| 🟢    | **Running** | *Working*                     | Agent is actively running tools or generating a response |
| 🟡    | **Waiting** | *Needs input*                 | Agent finished — waiting for your reply / permission |
| 💤    | **Resumable** | *Completed / Stopped*       | Session is not currently running — can be attached / resumed anytime |

Press `Enter` on *Running*/*Waiting* to attach (focuses the existing terminal tab). Press `Enter` on *Resumable* to relaunch the session.

## Keybindings

| Key | Action |
|-----|--------|
| `↑`/`↓` or `j`/`k` | Navigate sessions |
| `Enter` (⏎) | Resume selected session — focuses the WT tab if Running, launches otherwise |
| `n` | New session (launches default provider) |
| `a` | Archive session (instantly hidden) |
| `g` | Assign current session to a group (← → pick from existing, type to add new) |
| `s` | Run AI grouping on the top ungrouped sessions (Grouped view) — see [AI Auto-Grouping](#ai-auto-grouping) |
| `y` / `n` / `e` | Accept / dismiss / edit pending AI suggestion (cursor must be on a session with a 🤖 shadow) |
| `/` | Search (type to filter, `↑`/`↓` to browse, `Enter` to resume, `Esc` to cancel) |
| `Shift+Tab` | Cycle Active → Grouped → Hidden views |
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
| **Kimi** | `~/.kimi/sessions/` | Session JSONL files |

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
launch_method = "wt"    # "wt" | "pwsh" | "cmd"
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
# Windows — open in a new Windows Terminal tab
launch_cmd = "wt"
launch_args = ["-w", "0", "new-tab", "--startingDirectory", "{cwd}", "cmd", "/k", "{command}"]

# Linux/macOS — open in a new tmux window
# launch_cmd = "tmux"
# launch_args = ["new-window", "-c", "{cwd}", "{command}"]
```

Placeholders: `{cwd}` → working directory, `{command}` → the agent CLI command.

Config search order: next to exe → `%APPDATA%/agent-session-tui/config.toml` → built-in defaults.

## Semantic Search

Search uses a three-tier ranking system: **exact substring** → **fuzzy word** → **semantic similarity**. The semantic tier is an optional DLL plugin (`semantic_search.dll` / `.so` / `.dylib`) that adds meaning-aware matching using cached embeddings.

- Results with a semantic boost show a ✨ indicator in the search list
- Embeddings are pre-computed and cached per session — no embedding during search
- Status bar shows 🧠 when the semantic plugin is loaded and ready
- If the DLL is missing, search falls back gracefully to exact + fuzzy only

The plugin lives in `semantic-plugin/` and is built separately. See [`CONTRIBUTING.md` § Semantic Search Plugin](CONTRIBUTING.md#semantic-search-plugin) for the exact `cargo build` and copy-DLL-next-to-exe steps.

## AI Auto-Grouping

Optional. Asks an AI agent (GitHub Copilot CLI by default) to suggest thematic groups for your ungrouped sessions. **Off by default** — opt in via `config.toml`.

### What it does

- Sends each ungrouped session's **title, summary, and updated_at timestamp** (never file contents) to the AI in batches of 30.
- The AI proposes a group name + score for each session, or skips it.
- Suggestions render inline as a dim `· ⟨group⟩` shadow under the session row in the Active view.
- Press `y` to accept, `n` to dismiss, or `e` to edit the group name before saving.

### Two modes

| Mode | How to trigger | What happens |
|------|----------------|--------------|
| **Manual** | `s` in Grouped view | One batch, results open in a popup so you can step through y/n/e |
| **Auto** | `auto_suggest = true` in config | Runs in the background after the initial session load. Chains batches automatically until every ungrouped session has been analyzed. No popup — suggestions appear inline as shadows. |

### Requirements

- `copilot` CLI installed on PATH and authenticated (`copilot login`)
- `prompts/group-suggest.md` template next to the binary (shipped in the release zip)

### Configuration

```toml
[acp]
command = "copilot"
extra_args = ["--effort", "low"]   # ~30% faster than default; quality unchanged for this task
auto_suggest = false               # set to true for background auto-suggest
timeout_secs = 180                 # bump if your model is slow
# prompt_template = '~/.config/agent-session-tui/group-suggest.md'
```

### Cost & performance

- ~25–45s per 30-session batch with `--effort low`
- Each batch counts against your Copilot CLI usage quota
- Auto-suggest chains batches: ~1 minute per 30 sessions until all are processed

### Other group-view keys

- `g` — assign the selected session to an existing group (← → to pick) or type a new one

Groups are sorted by most-recent member activity, frozen on entry to the Grouped view to avoid jitter on the 2-second scan refresh. To refresh the order, leave and re-enter the view via `Shift+Tab`.

## Release Packages

| Package | Size | Contents |
|---------|------|----------|
| **Core** | ~1.1 MB | `agent-session-tui` binary only |
| **Semantic** | ~26 MB | Core + `semantic_search_plugin` DLL |

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
rustup override set stable-x86_64-pc-windows-msvc  # Windows only
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

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for how to get started — adding providers, building the semantic plugin, and code standards.

For project internals, design decisions, and AI agent context, see [`AGENTS.md`](AGENTS.md).

## License

[MIT](LICENSE)
