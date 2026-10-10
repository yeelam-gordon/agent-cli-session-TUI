# Agent CLI Session TUI — Find an old coding-agent conversation and resume it

<a id="languages"></a>
<details>
<summary>Languages / 语言 / 言語 / اللغات (16)</summary>

[English](README.md) · [简体中文](docs/i18n/README.zh-CN.md) · [繁體中文](docs/i18n/README.zh-TW.md) · [日本語](docs/i18n/README.ja.md) · [한국어](docs/i18n/README.ko.md) · [Español](docs/i18n/README.es.md) · [Português (Brasil)](docs/i18n/README.pt-BR.md) · [Français](docs/i18n/README.fr.md)

[Deutsch](docs/i18n/README.de.md) · [Italiano](docs/i18n/README.it.md) · [Русский](docs/i18n/README.ru.md) · [Türkçe](docs/i18n/README.tr.md) · [Tiếng Việt](docs/i18n/README.vi.md) · [Bahasa Indonesia](docs/i18n/README.id.md) · [हिन्दी](docs/i18n/README.hi.md) · [العربية](docs/i18n/README.ar.md)

</details>

Find lost work across CLI histories in one search. Resume the original conversation and context through its installed CLI. On Windows, try focusing its matching running Windows Terminal tab instead of opening another window.

[See how it works](#first-search-and-expected-result) · [Install Core safely](#install-the-complete-release-archive) · [Provider coverage and limits](#supported-providers)

Default remote grouping can automatically send titles and working paths; choose Core + wordmatch before launch.

<img src="docs/agent-session-benefits.png" alt="Concept: find a lost coding-agent conversation across CLI histories and resume it" width="780">

Concept illustration, not an actual app screenshot. English labels and mock rows are illustrative, not real private histories.

- **Find the old topic or project:** search a phrase you remember, then verify provider, title and working directory.
- **Continue the right conversation:** `Enter` on **Resumable** reopens through its provider CLI. Retained history, an installed/authenticated CLI and an accessible original directory are required.
- **Find the running Windows tab:** `Enter` on **Running/Waiting** attempts to focus a matching Windows Terminal tab, on Windows only—not universal process attachment. Linux/macOS users switch manually.

You do not need to have started the conversation in this TUI.


<a id="quick-start-find-an-old-conversation-and-resume-it"></a>

## Private first search: Core + wordmatch

### Before you launch: prerequisites and privacy

- Have saved history from at least one [supported provider](#supported-providers). To resume, that provider's CLI must be installed, available through the configured `command`, and authenticated. The original working directory must still be accessible.
- Use a release for your OS and CPU in an interactive terminal. Core targets Windows, Linux and macOS, x64/arm64; Semantic availability differs by target. On Windows, install Windows Terminal (`wt`) for the default launcher and native tab focus; `cmd` is the packaged fallback. Linux defaults to `gnome-terminal`; macOS uses `osascript` with Terminal. Change the [launcher configuration](#configuration) if those commands are unavailable. Linux/macOS do not focus existing running tabs.

### Install the complete release archive

1. Open [Releases](https://github.com/yeelam-gordon/agent-cli-session-TUI/releases), select a release, and download the **Core** archive for your OS/architecture (`.zip` on Windows, `.tar.gz` on Linux/macOS), not GitHub's source-code archive. Semantic is optional.
2. Extract the **whole archive** into a writable folder. Keep the binary, `providers/`, `prompts/`, and generated `config.toml.example` together. Copying just the executable loses provider definitions and prompt assets.

Check the exact release asset digest before launch; a byte match is not a signature or safety guarantee. Respect OS warnings and organizational policy. [SHA-256](#verify-a-release-download).

**Privacy warning before first launch:** default `remote` grouping sends session **titles and working directories** to a hosted service and can run automatically after initial discovery, not just when you press `s`. The service has no published third-party request/response contract. Choose non-network grouping before launching:

3. For a fresh Core install, first copy the **generated** `config.toml.example` to `config.toml` beside the executable. In that copy, find `# [grouping]` and `# engine = "remote"`: remove the leading `#` from **both lines** and change the engine value to `"wordmatch"`, producing the two active lines below. Leave the other commented grouping options unchanged. If you already have an active `[grouping]` table, change its `engine` there instead—do not add a second table. Save the file before launching or scanning any history.

```toml
[grouping]
engine = "wordmatch"
```

`wordmatch` uses word overlap; it is **not local AI**. This setting controls grouping, not the behavior of your provider CLIs. The optional semantic plugin can download model files on initialization; use Core for a first run without that plugin. See [grouping privacy](#what-remote-sends) and [semantic search](#what-you-get-out-of-the-box). Keep configuration, logs and session metadata private.

Check `state_dir`, provider `command` and launcher; disable unused providers with `enabled = false`. Do not overwrite an existing configuration.

4. Open a terminal **in the extracted folder**, then start:

```powershell
# Windows PowerShell
.\agent-session-tui.exe
```

```sh
# Linux/macOS
./agent-session-tui
```

### First search and expected result

Start with a literal title fragment or project phrase. Core provides keyword/fuzzy search; semantic paraphrase ranking needs the optional plugin. Use your own saved history; no sample session is bundled.

Six shipped history definitions and configured resume routes are provided; tested native CLI versions are **unknown** here. This is not live compatibility certification. See [coverage and evidence](#supported-providers).

| UI | `Enter` |
|---|---|
| Resumable | Resume through installed/authenticated CLI; original directory accessible |
| Running / Waiting / Idle | On Windows, attempt matching running Windows Terminal tab focus; otherwise switch manually |
| Archived / Stopped | Resume through installed/authenticated CLI; original directory accessible; [limits](#supported-providers) |



1. Wait for sessions to populate the list. Press `/`, then type a known project name or distinctive phrase (the slash opens search; it is not part of the query).
2. Use `↑`/`↓` to inspect matches and details. Confirm the provider, title and working directory before pressing `Enter`; ranking can miss a session, especially with vague or misspelled queries.
3. Choose **Resumable** to relaunch that provider CLI with its saved session ID in the original working directory. **Expected result:** a terminal opens and the provider displays the selected conversation, ready to continue, assuming history and authentication remain valid.


`Esc` cancels search; `q` or `Ctrl+C` exits without deleting provider histories. If nothing appears or launching fails, see [Troubleshooting, update and removal](#troubleshooting-update-and-removal).

<details>
<summary>Optional fictional illustration</summary>

**Fictional worked recovery—not observed runtime and not a bundled session:** press `/`, type `build failure`, and imagine a matching row: Claude Code · title `Build failure` · project `demo-project`. Use `↑`/`↓` to inspect the provider, title and original directory. Only if this is your intended **Resumable** conversation, press `Enter` to reopen it through the installed, authenticated Claude CLI with that directory accessible. This illustrates the workflow, not a guaranteed search match or compatibility test.

</details>




Native CLI history is enough when you already know the provider and session. Use this **terminal session manager** when histories span CLIs or projects, or you remember the topic rather than the session ID.

Built-in history definitions: **Copilot CLI, Claude Code, Codex CLI, Qwen CLI, Gemini CLI and Kimi**. Discovery depends on retained formats and filters; additional providers require [definitions](#adding-a-provider), not universal CLI support.

Follow the installation privacy steps before launching; default remote grouping can run automatically.





### Secondary workflow: find a running Windows tab

For **Running/Waiting**, `Enter` instead attempts to focus an existing matching **Windows Terminal** tab. **Expected result on a match:** that tab is selected and its window comes to the foreground. This is focus, not universal process attachment; on Linux/macOS, switch to the running terminal yourself.

Running labels can also include **Idle**. A second Enter can force a resume; verify the selected conversation first. **Kill is not implemented.**

## Recorded demo

Browse the saved-history list, search your own conversation, inspect it and select it to resume using the workflow above. This GIF records an English interface from its recording-time version; it is not current-version or all-provider compatibility verification. Translated documentation does not translate the UI or saved conversations. Complete the privacy setup before running. The recording version and date are unknown.

<img src="docs/agentTUI.gif" alt="Recorded English TUI history list, search and conversation selection" width="700">

## Verify a release download

Optional integrity check before executing: on the official release API, identify your **exact tag and asset name**, then compare its `digest` (`sha256:` prefix) with the local archive SHA-256. No `gh` installation is required: view the [v0.3.2 asset metadata](https://api.github.com/repos/yeelam-gordon/agent-cli-session-TUI/releases/tags/v0.3.2) in your browser. For a different release, use its own tag metadata; never reuse a different file’s digest.

If `gh` is already installed, this optional command lists the published tag’s names/digests:

```powershell
gh api repos/yeelam-gordon/agent-cli-session-TUI/releases/tags/v0.3.2 --jq '.assets[] | {name, digest}'
Get-FileHash -Algorithm SHA256 .\agent-session-tui-x86_64-pc-windows-msvc.zip
```

```sh
# Linux: use your exact downloaded asset name
sha256sum agent-session-tui-x86_64-unknown-linux-gnu.tar.gz
# macOS: use your exact downloaded asset name
shasum -a 256 agent-session-tui-aarch64-apple-darwin.tar.gz
```

Expected: local hexadecimal hash matches the exact asset digest, ignoring case and the metadata prefix. If absent/mismatched, do not execute; verify the tag, target and source or stop. A matching byte checksum is **not** a publisher signature, safety guarantee or proof of trustworthy software. Respect OS execution warnings and organizational policy; do not disable controls to run.

Bounded inspection: v0.3.2 Windows x64 Core `agent-session-tui-x86_64-pc-windows-msvc.zip` matched the published SHA-256 `08537753c6226ed2bd1703baaca9ac25ddbf3917792fd5f8cc9ba46a97c030dc`. Only ZIP contents/configuration were read—**no executable was run**. It contained binary, six provider definitions, prompts and generated example; example lines 63/64 were commented `[grouping]`/`engine = "remote"`, consistent with the copy/uncomment recipe. This is pinned archive evidence, not validation of later releases or native CLI resume. v0.3.2 metadata lists 11 target assets; no macOS x64 Semantic asset. Consult your release, not an assumed universal matrix.

## Troubleshooting, update and removal

| Symptom | What to check |
|---------|---------------|
| Empty list or no match | Confirm saved conversations, an enabled provider and the actual CLI `state_dir`. Wait for discovery; try a literal title fragment. Check Hidden with `Shift+Tab` if you archived it. Not every format/version or query is guaranteed to match. |
| Configuration error at startup | Check TOML syntax and duplicate tables. Back up your config, compare the generated release example, and fix the file found by the config search order. Preserve the privacy setting; removing configuration can restore remote defaults. |
| `Failed to resume` / no terminal | Try the provider's own resume command with the selected session ID in its original directory. Check authentication, `command`, resume arguments, directory existence and launcher availability. Fix configuration, not provider history. |
| Missing or wrong running-tab focus | Focus is Windows-only and title-based, not a process attachment guarantee. Shared titles can be ambiguous. Switch manually and verify the tab; Linux/macOS users must switch manually. |
| No 🧠 indicator | Semantic search is optional. Check the trusted matching-platform library beside the executable and model initialization; Core still has keyword/lexical search. |

See [issue guidance](CONTRIBUTING.md#reporting-issues) and [security reporting](SECURITY.md). Logs can contain IDs and paths; redact sensitive content before sharing.

**Update:** quit with `q`/`Ctrl+C`, back up `config.toml` and configured `data_dir` (groups, archive state and caches), and extract the new complete archive into a separate folder. Copy your configuration and keep the same `data_dir`; compare the new example without overwriting your settings. **Restore:** return to the old complete package and backed-up configuration/data. Provider updates and authentication are separate.

**Remove:** quit, then remove only your extracted TUI package. Optionally back up and remove its own configured `data_dir` and OS-level TUI configuration; do not delete `.copilot`, `.claude`, `.codex`, `.qwen`, `.gemini` or `.kimi` histories. Exiting/removing the TUI does not stop running provider CLIs; close those through their own terminal.

## Pain Points Solved

<details>
<summary>Optional alternatives and reference</summary>

- **Where is my running agent?** — `Enter` on a 🟡 *Needs input* / 🟢 *Working* session attempts to focus a matching Windows Terminal tab (Windows only)
- **Too many tabs** — supported retained sessions discovered by the provider definitions in one view with clear status badges (subject to provider formats and discovery filters)
- **Which needs my input?** — 🟡 *Needs input* vs 🟢 *Working* vs 💤 *Resumable* at a glance
- **Finding that one session** — `/` to search. Hybrid ranking fuses keyword (BM25) + semantic embeddings + lexical scoring via Reciprocal Rank Fusion, to help with typos, paraphrases, and partial recall; results are not guaranteed and semantic ranking requires the optional plugin. Indexes titles, summaries, compaction summaries, and your own messages
- **Hundreds of sessions piling up** — `g` to assign to groups; `Shift+Tab` to view by group; optional [AI auto-suggest](#ai-auto-grouping)
- **Close without worry** — shut down anytime; saved histories remain available; resumption depends on the provider CLI and retained history
- **Resume a saved conversation after reboot** — summaries, last activity and the full last response help choose the right retained history. This does not preserve a running process across reboot

</details>

## Keybindings

| Key | Action |
|-----|--------|
| `↑`/`↓` or `j`/`k` | Navigate sessions |
| `Enter` (⏎) | Resume a Resumable session; attempt WT tab focus for Running/Waiting on Windows |
| `n` | New session (launches default provider) |
| `a` | Archive session (instantly hidden) |
| `g` | Assign current session to a group (← → pick from existing, type to add new) |
| `s` | Run AI grouping on the top ungrouped sessions (Grouped view) — see [AI Auto-Grouping](#ai-auto-grouping) |
| `y` / `n` / `e` | Accept / dismiss / edit pending AI suggestion (cursor must be on a session with a 🤖 shadow) |
| `/` | Search (type to filter, `↑`/`↓` to browse, `Enter` to resume, `Esc` to cancel) |
| `Shift+Tab` | Cycle Active → Grouped → Hidden views |
| `Tab` | Switch panel focus (provider-independent) |
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

### Coverage and evidence—not a live compatibility certificate

| Provider | Shipped discovery definition | Content search | Resume | Running-tab focus | Evidence/version scope |
|---|---|---|---|---|---|
| Copilot CLI | [copilot.yaml](providers/copilot.yaml) | Core keyword/fuzzy; Semantic optional | Configured resume, not live certification | Windows matching-tab attempt only | Native CLI versions unknown; [test source](tests/copilot_lifecycle_test.rs) |
| Claude Code | [claude.yaml](providers/claude.yaml) | Core keyword/fuzzy; Semantic optional | Configured resume, not live certification | Windows matching-tab attempt only | Native CLI versions unknown; [test source](tests/claude_lifecycle_test.rs) |
| Codex CLI | [codex.yaml](providers/codex.yaml) | Core keyword/fuzzy; Semantic optional | Configured resume, not live certification | Windows matching-tab attempt only | Native CLI versions unknown; [test source](tests/codex_lifecycle_test.rs) |
| Qwen CLI | [qwen.yaml](providers/qwen.yaml) | Core keyword/fuzzy; Semantic optional | Configured resume, not live certification | Windows matching-tab attempt only | Native CLI versions unknown; [test source](tests/qwen_lifecycle_test.rs) |
| Gemini CLI | [gemini.yaml](providers/gemini.yaml) | Core keyword/fuzzy; Semantic optional | Configured resume, not live certification | Windows matching-tab attempt only | Native CLI versions unknown; [test source](tests/gemini_lifecycle_test.rs) |
| Kimi | [kimi.yaml](providers/kimi.yaml) | Core keyword/fuzzy; Semantic optional | Configured resume, not live certification | Windows matching-tab attempt only | Native CLI versions unknown; [test source](tests/kimi_lifecycle_test.rs) |

Definitions describe retained-history parsing; configured resume flags express intent. Public [synthetic scanner tests](src/provider/config_driven/mod.rs) include copilot/claude/codex/qwen/gemini end-to-end fixture tests; do not infer equivalent Kimi fixture coverage. Lifecycle test files exist for all six but need real provider history and are not proof that these native CLI versions were run. Tested native CLI versions are **unknown** here. Discovery depends on format, enabled definitions and filters; resume requires installed/authenticated configured CLI, retained history and accessible cwd. Core does not supply semantic paraphrase embeddings. Native focus additionally needs Windows Terminal and a matching tab; it is not universal attachment. Archived/Stopped are rendered labels too, not a guarantee of valid resume history.

## Configuration

<details>
<summary>Advanced reference</summary>

Use the generated release example, copied to `config.toml` next to the binary. This excerpt is Windows-oriented; retain your release example's OS-specific settings and the pre-launch privacy choice above:

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

Config search order: next to exe → the OS configuration directory (`%APPDATA%/agent-session-tui/config.toml` on Windows) → built-in defaults. A malformed found configuration is an error, not an instruction to use defaults.

The source checkout has **`config.toml.template`**, not a ready-to-run example: release packaging substitutes OS-specific placeholders using `scripts/generate_config.py` and writes **`dist/config.toml.example`**. Do not simply rename the source template. Developers can use the generator with their target triple before following the copy/edit step; see [Building](#building).

</details>

## Search

<details>
<summary>Advanced reference</summary>

Search is a hybrid ranker. Three independent signals score every session, then **Reciprocal Rank Fusion** (RRF) combines their ranked lists into a single ordering:

| Signal | What it catches |
|--------|-----------------|
| **BM25** (Tantivy full-text index) | Exact keywords, multi-word phrases, weighted titles |
| **Semantic** (cached embeddings, optional DLL) | Paraphrase similarity, e.g. "improve grammar" versus "Polish English Writing"; no guaranteed match |
| **Lexical** (substring + fuzzy) | Typos and partial recall, e.g. "iteation revew" versus "Iteration Review"; no guaranteed match |

On top of fusion: an **exact-title substring lock** prioritizes literal title matches. Multiple matching titles, available content and query choice still affect which conversation you find. State and recency act as soft tie-breakers.

### Measured on real session data

Maintainer-reported historical results from a private 30-query corpus over ~780 sessions, comparing the previous additive scorer with the current RRF default. Actual private queries, histories and run artifacts are not published, so these numbers are not publicly reproducible from the example queries. Results vary with corpus, plugin availability and environment; they do not guarantee accuracy on your histories and have not been rerun for this documentation update:

|                  | Additive | **RRF (default)** |
|------------------|----------|-------------------|
| MRR              | 0.696    | **0.802**         |
| P@1              | 53%      | **73%**           |
| Recall@10        | 93%      | 90%               |

MRR is mean reciprocal rank of the expected conversation; P@1 is the fraction with the expected conversation first; Recall@10 is the fraction retrieved within the first ten. First-result ranking improved, but top-ten coverage fell **93% → 90%**; this is a trade-off, not universal improvement. The optional plugin’s state for that historical run is **unknown**.

Per-category P@1 jumps: exact-title 33% → 100%, person-name 50% → 100%, semantic-only 38% → 62%, typo 0% → 33%. Keyword and partial-recall queries hold their ground.

### What you get out of the box

- No flags, no opt-in — RRF is the default scorer
- The semantic signal needs the optional `semantic_search_plugin.dll` / `libsemantic_search_plugin.so` / `libsemantic_search_plugin.dylib` plugin; without it, the ranker degrades gracefully to BM25 + lexical
- Embeddings are pre-computed and cached per session — search itself never embeds
- The plugin may download model files during initialization; embeddings are computed locally. Only load a trusted matching-platform plugin from a trusted directory
- Status bar shows 🧠 when the semantic plugin is loaded
- Indexes are incremental — only changed sessions are re-indexed on each scan
- **No index migration needed** when upgrading: schema, fingerprints, and embedding cache are unchanged

### Built-in evaluation harness

```bash
agent-session-tui --search-bench "iteration review" --expect <session_id> --top 10
agent-session-tui --search-eval --report eval/runs/today.json
```

Reads a gitignored `eval/search-queries.toml` of (query, expected session) pairs and reports MRR / P@1 / Recall@K, per-category and overall. Useful for catching ranking regressions before they ship.

The semantic plugin lives in `semantic-plugin/`. See [`CONTRIBUTING.md` § Semantic Search Plugin](CONTRIBUTING.md#semantic-search-plugin) for the build + install steps.

</details>

## AI Auto-Grouping

<details>
<summary>Advanced reference</summary>

Suggests thematic groups for your ungrouped sessions. It runs **once**, after
the initial full discovery completes, so the top-30 batch is selected from the
complete session list rather than the fast first page. Press `s` in the Grouped
view to run another batch on demand. Suggestions appear as a dim
`· ⟨group⟩` shadow under the session row — `y` accepts, `n` dismisses, `e` edits
the name first.

Sessions that look like an **already-grouped** session are suggested for that
same existing group instead of a brand-new one.

### Engines

Set with `[grouping] engine` in `config.toml`.

**`remote` — the default.** Sends session titles and working directories to a
hosted tab auto-grouping service, which returns natural group names like
"System Maintenance". A maintainer-reported historical private-data run took about 2 seconds for 30 sessions; no public run artifact/environment is supplied. Latency varies with service, network and workload. No separate service installation or authentication is configured, but titles and paths leave the machine.

**`wordmatch`** — no network, and **not an AI model of any kind**. It counts how many words two session titles share, groups them if more than half overlap, and names each group after its most common words.
That produces blunt names like `benchmark-prompt-files`, so expect to rename
some with `e`. A maintainer-reported historical private-data run took ~0.2s; no public run artifact/environment is supplied. Timing varies with hardware and workload. It is also the automatic fallback whenever `remote` fails.

**`acp`** — legacy. Spawns the CLI configured in `[acp]` (default `copilot`).
The maintainer historically reported useful names and 25–45s per batch on private data, without a public run artifact/environment. Quality and timing vary with model and workload; it consumes the configured CLI's quota. Requires the
`copilot` CLI authenticated and `prompts/group-suggest.md` next to the binary.
A run exceeding `timeout_secs` is **terminated**, not just abandoned.

```toml
[grouping]
engine = "remote"     # "remote" | "wordmatch" | "acp"
language = "en-US"    # language for generated group names
timeout_secs = 20     # remote call timeout
```

### What `remote` sends

- **Sent:** session titles and working directories (as `file:///` URIs), for one
  representative per cluster of near-duplicate sessions. The maintainer reported a historical private-data reduction from
  40 sessions to 8 entries; no public run artifact is supplied and other histories
  can cluster differently. Working directories provide additional context.
- **Never sent:** session summaries, file contents, chat transcripts.
- **On any failure** — non-2xx, timeout, schema change — it falls back to
  `wordmatch` and shows a warning. To retry the online service, use `Shift+Tab` to
  enter the Grouped view, then press `s`. If a manual run fails while its
  suggestion review is open, press `Esc`, then `s`. The fallback is intended to preserve grouping when the service fails.

> **Note:** the grouping service is Microsoft-hosted. Its URL is published in
> Microsoft's [Edge endpoint allowlist](https://learn.microsoft.com/en-us/deployedge/microsoft-edge-security-endpoints)
> (under "Tab groups"), but only as a firewall entry — there is no published
> request/response contract, versioning guarantee, or terms covering
> third-party callers, and the request shape used here was determined by
> probing. Titles and folder paths do leave your machine. If that isn't
> acceptable for your work, set `engine = "wordmatch"`.

### Reusing the groups you already have

New sessions that cluster with an already-grouped session inherit that group,
so you don't accumulate near-duplicates. That matching happens locally; results depend on titles and existing assignments.

There is also an opt-in mode that sends your existing group *names* (never the
sessions in them) to the remote service so it can fold candidates in directly.
It's **off by default**: the service accepts those payloads only erratically —
in a maintainer-reported historical private-data trial with 11 sessions, 3 group names returned
an empty response while 1 succeeded. No public run artifact is supplied; this is not a current reliability measurement. Turning it on mostly costs a wasted
round-trip before the automatic retry.

```toml
[grouping]
reuse_existing_groups = false  # true to try sending existing group names
max_group_anchors = 6          # how many names to offer when enabled
```

### Other group-view keys

- `g` — assign the selected session to an existing group (← → to pick) or type a new one

Groups are sorted by most-recent member activity, frozen on entry to the Grouped view to avoid jitter on the 2-second scan refresh. To refresh the order, leave and re-enter the view via `Shift+Tab`.

</details>

## Release Packages

<details>
<summary>Advanced reference</summary>

| Package | Size | Contents |
|---------|------|----------|
| **Core** | ~1.1 MB binary — historical estimate¹ (archive varies) | Binary + `providers/`, `prompts/`, generated `config.toml.example`, README and license |
| **Semantic** | ~26 MB — historical estimate¹ (varies by target) | Complete Core package + platform-specific plugin; model download may follow |

¹ These are earlier maintainer estimates, not measurements of the current release. Their version, target and measurement method are unknown; the ~26 MB estimate does not establish compressed versus unpacked size. Do not infer a current download or disk-space requirement from either figure. Check the exact selected release asset size and unpacked contents.

Core targets cover **x64** and **arm64** on Windows, Linux and macOS. The Semantic matrix covers Windows/Linux x64 and arm64, and macOS arm64; consult the selected release assets for availability.

</details>

## Beyond `claude agents`

<details>
<summary>Optional alternatives and reference</summary>

This TUI reads supported retained provider histories, including conversations started independently of this TUI. See Claude Code's [`agent view documentation`](https://code.claude.com/docs/en/agent-view) for its current behavior; no future or version-wide comparison is asserted here.

See [provider coverage](#supported-providers), [search](#search) and [grouping privacy](#what-remote-sends) for the same catalog capabilities and limits.

</details>

## Beyond Intelligent Terminal

<details>
<summary>Optional alternatives and reference</summary>

[**Intelligent Terminal**](https://github.com/microsoft/intelligent-terminal) is an experimental fork of Windows Terminal with native agent integration — the *terminal* that hosts your agent CLI. This TUI catalogs supported retained histories discovered by its provider definitions.

See [provider coverage](#supported-providers), [search](#search) and [grouping privacy](#what-remote-sends) for the same catalog capabilities and limits.

Use them together: Intelligent Terminal for the conversation, this TUI for the catalog.

</details>

## Architecture

<details>
<summary>Advanced reference</summary>

```
┌─────────────────────────────────────────────────────────────┐
│ TUI (ratatui + crossterm)                                   │
│  Session List  │  Session Detail  │  Activity Log           │
│  Hybrid RRF Search (BM25 + semantic + lexical) · Tab Focus │
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

The actual rendered source labels include Running, Waiting, Idle, Resumable, Archived and Stopped. The selection table above uses those English labels. The following historical comparison only covers three common cases; other-product names are not TUI labels:

| Badge | This TUI    | `claude agents` | Meaning |
|-------|-------------|-----------------|---------|
| 🟢    | **Running** | *Working*       | Actively running tools or generating a response |
| 🟡    | **Waiting** | *Needs input*   | Finished — waiting for your reply / permission |
| 💤    | **Resumable** | *Completed*   | Not currently running — resume through the installed provider CLI |

`Enter` on *Running* / *Waiting* attempts Windows Terminal tab focus on Windows; Linux/macOS cannot focus existing tabs. `Enter` on *Resumable* relaunches through the configured CLI.

</details>

## Adding a Provider

<details>
<summary>Advanced reference</summary>

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

Launch and resume are handled by the supervisor from `config.toml`; **kill is not implemented**. The six shipped providers use YAML definitions loaded generically by `main.rs::create_provider()`; implement the trait only when the shared YAML engine cannot express your provider.

</details>

## Building

<details>
<summary>Advanced reference</summary>

Requires the **MSVC toolchain** on Windows (for the `windows` crate used by tab focus):

```bash
rustup override set stable-x86_64-pc-windows-msvc  # Windows only
cargo build --release
# Binary: target/release/agent-session-tui(.exe)
```


Python 3 is needed for configuration generation. From the repository root, after building, stage a fresh Windows package (do not overwrite an existing installation):

```powershell
New-Item -ItemType Directory -Path dist
Copy-Item target\release\agent-session-tui.exe dist\
Copy-Item providers,prompts -Destination dist -Recurse
python scripts\generate_config.py x86_64-pc-windows-msvc config.toml.template dist\config.toml.example
Copy-Item dist\config.toml.example dist\config.toml
```

For Windows arm64 use `aarch64-pc-windows-msvc`. For native Linux/macOS builds, copy `target/release/agent-session-tui` rather than the `.exe`, and keep `providers/` and `prompts/` beside it. The generator takes three arguments: **target triple, source template, output file**. From the repository root:

```sh
# Linux x64; use aarch64-unknown-linux-gnu for arm64
python3 scripts/generate_config.py x86_64-unknown-linux-gnu config.toml.template dist/config.toml.example
# macOS arm64; use x86_64-apple-darwin for x64
python3 scripts/generate_config.py aarch64-apple-darwin config.toml.template dist/config.toml.example
```

The generator creates the output directory if needed but does not copy assets. The finished layout is `dist/agent-session-tui(.exe)`, `dist/providers/`, `dist/prompts/`, `dist/config.toml.example` and your edited `dist/config.toml`. Copy the example only for a fresh setup; never rename the placeholder template or overwrite existing configuration. **Before launching the staged binary**, follow [the prerequisites and privacy choice](#before-you-launch-prerequisites-and-privacy), including `[grouping] engine = "wordmatch"` for non-network grouping.

</details>

## Testing

<details>
<summary>Advanced reference</summary>

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

</details>

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for how to get started — adding providers, building the semantic plugin, and code standards.

For project internals, design decisions, and AI agent context, see [`AGENTS.md`](AGENTS.md).

## License

[MIT](LICENSE)
