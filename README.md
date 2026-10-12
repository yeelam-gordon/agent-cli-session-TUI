# Agent CLI Session TUI — Find and resume coding-agent conversations

<a id="languages"></a>
<details>
<summary>Languages (16)</summary>

[English](README.md) · [简体中文](docs/i18n/README.zh-CN.md) · [繁體中文](docs/i18n/README.zh-TW.md) · [日本語](docs/i18n/README.ja.md) · [한국어](docs/i18n/README.ko.md) · [Español](docs/i18n/README.es.md) · [Português (Brasil)](docs/i18n/README.pt-BR.md) · [Français](docs/i18n/README.fr.md)

[Deutsch](docs/i18n/README.de.md) · [Italiano](docs/i18n/README.it.md) · [Русский](docs/i18n/README.ru.md) · [Türkçe](docs/i18n/README.tr.md) · [Tiếng Việt](docs/i18n/README.vi.md) · [Bahasa Indonesia](docs/i18n/README.id.md) · [हिन्दी](docs/i18n/README.hi.md) · [العربية](docs/i18n/README.ar.md)

</details>

Remember the work, but not which CLI held it? Search saved conversations across Copilot CLI, Claude Code, Codex CLI, Qwen CLI, Gemini CLI and Kimi. If you already know the provider and session, its native history may be enough.

<img src="docs/agent-session-benefits.png" alt="Concept: keyword search finds token expires in retained conversation context, not just its title, then resumes the saved conversation through its original CLI. Copilot CLI, Claude Code, Codex and other providers are supported." width="700">

Concept illustration with fictional rows, not an app screenshot. English labels.

- **Find lost work:** one search across retained CLI histories, including conversations started outside this TUI.
- **Continue the original conversation:** reopen its saved context through the installed provider CLI.
- **Find its running Windows tab:** attempt matching Windows Terminal tab focus; Linux/macOS users switch manually.

## First search: Core + wordmatch

Have saved history, an installed/authenticated provider CLI reachable through its configured `command`, and an accessible original working directory. Use an interactive terminal. Default launchers: Windows `wt` (fallback `cmd`), Linux `gnome-terminal`, macOS `osascript` with Terminal; [configure another launcher](docs/usage.md#configuration) if unavailable.

**Before launch:** default remote grouping can automatically send **session titles and working directories** to a hosted service after discovery. It has no published third-party API contract. Core + `wordmatch` avoids that remote grouping, not provider-CLI networking. The optional semantic plugin may download models at initialization; start with Core without that plugin.

1. Download the complete **Core** archive for your OS/CPU from [Core archive filenames](docs/usage.md#core-archive-filenames), not the source archive. Extract everything together: binary, `providers/`, `prompts/`, generated `config.toml.example`. [Verify the exact asset digest](docs/usage.md#verify-a-release-download); a match is not a signature or safety guarantee. Respect OS/org controls.
   For attestation-enabled releases, [verify GitHub build provenance](docs/usage.md#verify-github-build-provenance) before extracting. This is not Windows code signing; older releases such as v0.3.2 are not retroactively attested.
2. For a fresh install, copy the generated example to `config.toml` beside the binary; **do not overwrite existing settings or rename the source template**. In the copy, uncomment **both** `# [grouping]` and `# engine = "remote"`, then change the value:

```toml
[grouping]
engine = "wordmatch"
```

Keep other grouping options commented. If `[grouping]` already exists, change its `engine` rather than adding a duplicate. Check `state_dir` and `command`; save **before launch**. `wordmatch` is word overlap, not local AI.

3. In the extracted folder, run:

```powershell
# Windows PowerShell
.\agent-session-tui.exe
```

```sh
# Linux/macOS
./agent-session-tui
```

4. Wait for the list. Press `/`, type a literal title/project fragment from your own history, and inspect matches with `↑`/`↓`. Verify provider, title and directory; choose **Resumable**, then press `Enter`. **Expected:** its CLI opens the selected conversation to continue. Search can miss; tested native CLI versions are unknown. [coverage and evidence](docs/usage.md#supported-providers).

For **Running/Waiting/Idle**, `Enter` attempts Windows matching-tab focus, not process attachment. Repeating `Enter` promptly on the same running session can **force resume** instead; verify before doing so. [Windows route](docs/usage.md#secondary-workflow-find-a-running-windows-tab).

**No result?** Check saved history/enabled provider/`state_dir`; for launch failures, check authentication, cwd and launcher. [Troubleshooting](docs/usage.md#troubleshooting-update-and-removal).

**Exit:** `Esc` cancels search; then `q` exits, or `Ctrl+C` in the list or search without deleting histories or stopping other CLIs. Kill is unimplemented. [Update, restore and removal](docs/usage.md#troubleshooting-update-and-removal).

[Operating guide](docs/usage.md) · [Recorded demo](docs/usage.md#recorded-demo) · [Contributing](CONTRIBUTING.md) · [Security](SECURITY.md) · [MIT license](LICENSE)

<details>
<summary>Earlier section links → operating guide</summary>

<a id="agent-cli-session-tui--find-an-old-coding-agent-conversation-and-resume-it"></a>[Agent CLI Session TUI — Find an old coding-agent conversation and resume it](docs/usage.md#quick-start-find-an-old-conversation-and-resume-it)

<a id="private-first-search-core--wordmatch"></a>[Private first search: Core + wordmatch](docs/usage.md#private-first-search-core--wordmatch)

<a id="before-you-launch-prerequisites-and-privacy"></a>[Before you launch: prerequisites and privacy](docs/usage.md#before-you-launch-prerequisites-and-privacy)

<a id="install-the-complete-release-archive"></a>[Install the complete release archive](docs/usage.md#install-the-complete-release-archive)

<a id="first-search-and-expected-result"></a>[First search and expected result](docs/usage.md#first-search-and-expected-result)

<a id="secondary-workflow-find-a-running-windows-tab"></a>[Secondary workflow: find a running Windows tab](docs/usage.md#secondary-workflow-find-a-running-windows-tab)

<a id="recorded-demo"></a>[Recorded demo](docs/usage.md#recorded-demo)

<a id="verify-a-release-download"></a>[Verify a release download](docs/usage.md#verify-a-release-download)

<a id="troubleshooting-update-and-removal"></a>[Troubleshooting, update and removal](docs/usage.md#troubleshooting-update-and-removal)

<a id="pain-points-solved"></a>[Pain Points Solved](docs/usage.md#pain-points-solved)

<a id="keybindings"></a>[Keybindings](docs/usage.md#keybindings)

<a id="supported-providers"></a>[Supported Providers](docs/usage.md#supported-providers)

<a id="coverage-and-evidencenot-a-live-compatibility-certificate"></a>[Coverage and evidence—not a live compatibility certificate](docs/usage.md#coverage-and-evidencenot-a-live-compatibility-certificate)

<a id="configuration"></a>[Configuration](docs/usage.md#configuration)

<a id="search"></a>[Search](docs/usage.md#search)

<a id="measured-on-real-session-data"></a>[Measured on real session data](docs/usage.md#measured-on-real-session-data)

<a id="what-you-get-out-of-the-box"></a>[What you get out of the box](docs/usage.md#what-you-get-out-of-the-box)

<a id="built-in-evaluation-harness"></a>[Built-in evaluation harness](docs/usage.md#built-in-evaluation-harness)

<a id="ai-auto-grouping"></a>[AI Auto-Grouping](docs/usage.md#ai-auto-grouping)

<a id="engines"></a>[Engines](docs/usage.md#engines)

<a id="what-remote-sends"></a>[What `remote` sends](docs/usage.md#what-remote-sends)

<a id="reusing-the-groups-you-already-have"></a>[Reusing the groups you already have](docs/usage.md#reusing-the-groups-you-already-have)

<a id="other-group-view-keys"></a>[Other group-view keys](docs/usage.md#other-group-view-keys)

<a id="release-packages"></a>[Release Packages](docs/usage.md#release-packages)

<a id="beyond-claude-agents"></a>[Beyond `claude agents`](docs/usage.md#beyond-claude-agents)

<a id="beyond-intelligent-terminal"></a>[Beyond Intelligent Terminal](docs/usage.md#beyond-intelligent-terminal)

<a id="architecture"></a>[Architecture](docs/usage.md#architecture)

<a id="session-states"></a>[Session States](docs/usage.md#session-states)

<a id="adding-a-provider"></a>[Adding a Provider](docs/usage.md#adding-a-provider)

<a id="building"></a>[Building](docs/usage.md#building)

<a id="testing"></a>[Testing](docs/usage.md#testing)

<a id="contributing"></a>[Contributing](docs/usage.md#contributing)

<a id="license"></a>[License](docs/usage.md#license)

<a id="quick-start-find-an-old-conversation-and-resume-it"></a>[Original quick start](docs/usage.md#quick-start-find-an-old-conversation-and-resume-it)

</details>
