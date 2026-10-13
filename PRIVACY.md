# Privacy and local data

This notice describes builds containing the **privacy-defaults change**, not
older published releases. Those releases may automatically use remote grouping.
This is implementation documentation, not a compliance certification or external
approval. Read the notice in your browser or extracted archive without running
the binary. **Only builds known to contain this change** support `--help` and
`--privacy` as early exits before config creation, session discovery or model
loading. Do not use these flags to inspect an older or unknown build: it may
ignore them and start the application, including automatic remote grouping.

## Defaults and choices

Local discovery and search read enabled providers' saved sessions and inspect
local process metadata (including command lines/PIDs, and Windows window/tab
information). Search is not just a metadata list: it builds a local index of
conversation content. Provider history is not deleted by archiving a session.

Grouping is **manual by default**, with non-network `wordmatch` as the default
engine. `s` in the Grouped view runs the selected engine on demand; assigning
groups yourself remains available. Configure before launching:

```toml
[grouping]
engine = "wordmatch" # default; "local" remains an alias
auto_suggest = false # no automatic grouping; manual s remains available
```

Setting `auto_suggest = true` runs suggestions once after full initial discovery.
Choosing `engine = "remote"` or `"acp"` does not itself enable automatic runs.
An explicit remote choice still sends data when you press `s`.
Existing engine choices are preserved on upgrade. If `auto_suggest` is omitted,
only an explicitly selected ACP engine with legacy `[acp] auto_suggest = true`
retains automatic runs; an explicit `[grouping] auto_suggest` overrides that
legacy setting. ACP also requires an `[acp]` section and a prompt template.
Missing/old configs without automatic opt-in no longer auto-group.

## Where data is saved

`D` below is the platform local-data directory plus `agent-session-tui`:

| OS | Default `D` | OS config fallback |
|---|---|---|
| Windows | `%LOCALAPPDATA%\agent-session-tui` | `%APPDATA%\agent-session-tui\config.toml` |
| macOS | `$HOME/Library/Application Support/agent-session-tui` | `$HOME/Library/Application Support/agent-session-tui/config.toml` |
| Linux | `$XDG_DATA_HOME/agent-session-tui`, or `$HOME/.local/share/agent-session-tui` | `$XDG_CONFIG_HOME/agent-session-tui/config.toml`, or `$HOME/.config/agent-session-tui/config.toml` |

Config search order is `config.toml` beside the executable, then the OS config
fallback, then built-in defaults. If absent, a default config is written beside
the executable. Config contains commands, paths and feature choices.

Top-level `data_dir` overrides `D` **for groups, archive and the full-text index
only**. It is a literal path: use an absolute path; `~`, `%VAR%` and `$VAR` are
not expanded for this setting. Relative paths resolve from the working directory.
Provider `state_dir` supports its own path expansion and selects provider history,
not the TUI cache. If the OS data directory cannot be resolved, the default is
`./agent-session-tui`.

| Location | Content |
|---|---|
| `<data_dir>/groups.json` | Provider/session keys, group assignments and their metadata, dismissed suggestions, group names/descriptions. |
| `<data_dir>/archived.json` | Provider/session keys hidden from the active view. This is not deletion of provider history. |
| `<data_dir>/log_index/` | Tantivy full-text index: session IDs and indexed terms from session activity/log content, potentially sensitive transcript excerpts, messages, summaries and tool output—not merely metadata. The raw content field is indexed, not stored as a verbatim transcript field. |
| `<data_dir>/log_index_fingerprints.json` | Session IDs and hashes derived from activity-file modification times/sizes, used to skip unchanged files. |
| `D/models/embeddings_cache_v3.json` | Session IDs, content hashes and numeric embedding vectors of title/summary/transcript chunks. These are derived sensitive data, not anonymized data; no raw text field is stored in this JSON. |
| `D/models/` | Model/tokenizer assets in Hugging Face cache layout: the bundled plugin selects `Qdrant/all-MiniLM-L6-v2-onnx`, cached as `models--Qdrant--all-MiniLM-L6-v2-onnx` (blobs/snapshots/references). `HF_HOME` overrides the model-asset cache directory, but not `embeddings_cache_v3.json`. **Neither is relocated by `data_dir`.** Older assets/cache versions may remain after upgrades. The host's Nomic directory probe and download message are not the bundled plugin's model selection. |
| OS temporary directory + `agent-session-tui-scan-<provider_name>.json` | Persistent discovery cache: parsed Session objects (IDs, titles, summaries, cwd/state paths, timestamps, status and possible PID/tab title), source modification times and provider fingerprint. Summary text can include conversation content. **Not relocated by `data_dir`.** |
| OS temporary directory + `agent-session-tui.log` | Append-only diagnostics: timestamps, paths, session IDs, operations/errors and possible command, prompt or response details. Windows normally `%TEMP%\agent-session-tui.log`; Unix uses `$TMPDIR` when set, otherwise typically `/tmp/agent-session-tui.log`. |

Atomic saves can also leave adjacent `*.json.tmp` files after interruption.
The full-text reader can index whole activity files up to 2 MiB, or head/tail
slices and selected structured messages/summaries for larger files. These are
read budgets, **not retention or total disk-size limits**.

## Network boundaries

* **Wordmatch and core search:** local computation, no grouping/search service.
  Disabling automatic grouping does not disable local indexing.
* **Remote grouping (explicit engine choice):** sends session **titles and working
  directories as `file:///` URIs**, one representative per local cluster, plus
  language and request bookkeeping, to
  `https://edge.microsoft.com/taggrouptitlegeneration/api/AutoGrouping/groupingacstreaming`.
  Existing group **names** are additionally sent only with
  `reuse_existing_groups = true` (default false). This path does not send summary
  fields, file contents or transcripts; titles/paths themselves can be sensitive.
  A local fallback after a remote failure does not undo transmission.
  The Microsoft-hosted endpoint appears in the Edge firewall allowlist, not a
  published third-party API contract. This project cannot promise the service's
  retention, processing terms or availability; review its suitability before use.
* **ACP grouping (explicit engine/CLI configuration):** passes session keys,
  titles, cwd (normally its last path component), short summary snippets and
  existing group names/counts/descriptions in a
  prompt to the configured CLI (default command `copilot`). Its model/service
  may receive these data, incur quota charges and save its own history. Custom
  prompts/commands can change this behavior. If semantic embeddings are available,
  the prompt can also include session-pair IDs and similarity scores (not vectors).
  The remote-engine “no summaries”
  statement does **not** apply to ACP.
* **Optional semantic plugin:** uses local ONNX inference for query/document
  embeddings, but initializes before the TUI and can download MiniLM-L6-v2
  assets through fastembed/Hugging Face (`https://huggingface.co`, or the
  configured `HF_ENDPOINT`). Download servers receive ordinary
  network request metadata (for example IP address); this is distinct from
  sending session text to a hosted embedding API. Use Core without the plugin
  library to avoid that initialization/download. Automatic-grouping settings do
  not control model loading.
* **Providers and remote machines:** launching/resuming a CLI uses that vendor's
  authentication, history and network behavior, outside these grouping controls.
  Explicit `--remote`, remote list/stream commands and custom launchers can move
  session information over their configured transport. JSON dump/serve modes
  output session information to stdout; protect any capture or forwarding.

## Retention, deletion and protection

No fixed expiry or storage cap is promised. Index refresh removes entries absent
from the active/hidden session set, but archiving alone is not erasure and neither
index maintenance nor file deletion is secure erasure. Semantic caches, groups
and diagnostics can persist until removed. `log_max_lines` (default 500) limits
the TUI's displayed in-memory log list, **not** the append-only diagnostic file;
there is currently no built-in rotation or size limit for that file.

Quit the TUI before cleaning its files. Back up groups/archive state if wanted.
Remove the TUI's `agent-session-tui-scan-*.json` discovery caches (and interrupted
`.json.tmp` saves) from the OS temporary directory. Remove `log_index/` together
with `log_index_fingerprints.json` to clear full-text
search data; remove embedding caches (or `D/models/` for model assets too)
separately, especially with a custom `data_dir` or `HF_HOME`. Only remove TUI
model assets from a shared `HF_HOME`, not other applications' caches. Remove the diagnostic log
separately. Removing config resets settings, not caches. Removing the executable
alone does not remove any of these data. On restart, enabled providers can be
re-indexed and the installed semantic plugin can download models again.
Disable unused providers or stop using the TUI to prevent further indexing;
there is no dedicated persistent “disable local index” switch.

Do not delete provider directories just to remove the TUI. Manage provider
histories and credentials through their own tools. Files use normal filesystem
permissions; the TUI does not add encryption or guarantee owner-only permissions.
Protect directories, backups and diagnostic output with OS access controls.

For questions, use the repository's
[issue tracker](https://github.com/yeelam-gordon/agent-cli-session-TUI/issues)
without attaching private histories, logs, paths or credentials. For security
reporting, follow [SECURITY.md](https://github.com/yeelam-gordon/agent-cli-session-TUI/blob/main/SECURITY.md).
