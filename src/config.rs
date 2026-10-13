use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Directory for app data (archived.json, etc.)
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
    #[serde(default = "default_log_lines")]
    pub log_max_lines: usize,
    /// UI redraw/event-poll tick in milliseconds. Higher = lower idle CPU,
    /// less responsive spinner animations. Default 1000 (low CPU, smooth-ish spinners).
    /// Drop to 250 for snappy spinners at the cost of ~5% idle CPU.
    /// Keypresses are always instant regardless of this value.
    #[serde(default = "default_tick_rate_ms")]
    pub tick_rate_ms: u64,
    /// Minimum interval (ms) between semantic-indexer runs. Even if sessions
    /// change, the indexer won't fire more often than this. Default 60000 (1 min).
    /// Lower = fresher embeddings of in-progress sessions, higher CPU.
    #[serde(default = "default_semantic_index_min_interval_ms")]
    pub semantic_index_min_interval_ms: u64,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    /// Configuration for the legacy ACP grouping engine.
    ///
    /// Only `Some(_)` when the user explicitly writes an `[acp]` section in
    /// `config.toml`. When the ACP engine is selected, grouping requires this
    /// opt-in and a resolvable prompt template; otherwise its `s` shortcut and
    /// automatic suggestions are unavailable and no ACP subprocess is spawned.
    /// Remote and wordmatch grouping remain available without this section;
    /// wordmatch is the default and does not use the network.
    #[serde(default)]
    pub acp: Option<AcpConfig>,
    /// Grouping defaults to manual, non-network word matching.
    #[serde(default)]
    pub grouping: GroupingConfig,
    /// Shared template for `--remote <box>` mode. When you run the TUI with
    /// `--remote <box>`, it streams that box's whole session list over the
    /// tunnel and wraps resume so it opens on the box. Because every box has
    /// the same layout, this ONE template (with a `{box}` placeholder) covers
    /// every box — a new box needs no config at all.
    #[serde(default)]
    pub remote_defaults: Option<RemoteConfig>,
    /// Per-box overrides. Only add a `[remotes.<box>]` section when a specific
    /// box needs different settings than `remote_defaults`. Any field left out
    /// falls back to the template.
    #[serde(default)]
    pub remotes: HashMap<String, RemoteConfig>,
}

/// How to reach a remote box in `--remote` mode. Strings may contain a `{box}`
/// placeholder (replaced with the box name) and, for `launch_args`, `{command}`
/// (replaced with the built resume command).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RemoteConfig {
    /// Command that returns the box's session list as JSON (an array of
    /// sessions) — re-run each refresh. Typically a `devbox exec <box>
    /// "am --dump-json 50"` invocation. NOTE: a one-shot command that EXITS is
    /// required (not a persistent stream) — the VS Code tunnel only flushes a
    /// spawned command's output when it exits.
    #[serde(default, alias = "stream_cmd")]
    pub list_cmd: Vec<String>,
    /// Launcher used to resume a session on the box (opens a host terminal tab
    /// that dials the box). `{command}` = the built resume command.
    #[serde(default)]
    pub launch_cmd: Option<String>,
    #[serde(default)]
    pub launch_args: Option<Vec<String>>,
}

impl AppConfig {
    /// Resolve the effective [`RemoteConfig`] for a box: start from
    /// `remote_defaults`, overlay any `[remotes.<box>]` overrides, then
    /// substitute the `{box}` placeholder. Returns `None` if neither a template
    /// nor an override exists.
    pub fn resolve_remote(&self, box_name: &str) -> Option<RemoteConfig> {
        let has_override = self.remotes.contains_key(box_name);
        if self.remote_defaults.is_none() && !has_override {
            return None;
        }
        let mut rc = self.remote_defaults.clone().unwrap_or_default();
        if let Some(over) = self.remotes.get(box_name) {
            if !over.list_cmd.is_empty() {
                rc.list_cmd = over.list_cmd.clone();
            }
            if over.launch_cmd.is_some() {
                rc.launch_cmd = over.launch_cmd.clone();
            }
            if over.launch_args.is_some() {
                rc.launch_args = over.launch_args.clone();
            }
        }
        let sub = |s: &String| s.replace("{box}", box_name);
        rc.list_cmd = rc.list_cmd.iter().map(sub).collect();
        rc.launch_cmd = rc.launch_cmd.as_ref().map(sub);
        rc.launch_args = rc.launch_args.as_ref().map(|v| v.iter().map(sub).collect());
        Some(rc)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub enabled: bool,
    #[serde(default)]
    pub default: bool,
    /// The CLI command to invoke (e.g., "copilot", "claude").
    pub command: String,
    #[serde(default)]
    pub default_args: Vec<String>,
    pub state_dir: Option<PathBuf>,
    pub resume_flag: Option<String>,
    #[serde(default)]
    pub startup_dir: Option<PathBuf>,
    /// Launch method shortcut: "wt" | "wtai" | "pwsh" | "cmd".
    /// Ignored if `launch_cmd` is set.
    #[serde(default = "default_launch_method")]
    pub launch_method: String,
    /// Custom launcher program (e.g., "wtai", "wt", "tmux"). Overrides launch_method.
    #[serde(default)]
    pub launch_cmd: Option<String>,
    /// Custom launcher args template. Use {cwd} and {command} as placeholders.
    /// Example: ["-w", "0", "new-tab", "--startingDirectory", "{cwd}", "cmd", "/k", "{command}"]
    #[serde(default)]
    pub launch_args: Option<Vec<String>>,
    /// Fallback launcher program if primary fails.
    #[serde(default)]
    pub launch_fallback_cmd: Option<String>,
    /// Fallback launcher args template. Same placeholders as launch_args.
    #[serde(default)]
    pub launch_fallback_args: Option<Vec<String>>,
    /// Legacy fallback shortcut: "wt" | "pwsh" | "cmd". Ignored if launch_fallback_cmd is set.
    #[serde(default)]
    pub launch_fallback: Option<String>,
    #[serde(default)]
    pub wt_profile: Option<String>,
    /// OPT-IN remote mode. When set, this provider does NOT read a local
    /// `state_dir`; instead it runs this command, captures stdout, and parses it
    /// as the JSON produced by `agent-session-tui --dump-json` (an array of
    /// Session objects) to obtain the session list from another machine.
    ///
    /// Example (resume Copilot sessions living on a remote box via devbox):
    ///   remote_list_cmd = ["devbox", "exec", "gordon-devbox1",
    ///                      "agent-session-tui --dump-json 50"]
    ///
    /// When `None` (the default), the provider behaves exactly as before:
    /// local filesystem discovery via the YAML `ConfigDrivenProvider`.
    #[serde(default)]
    pub remote_list_cmd: Option<Vec<String>>,
    /// OPT-IN streaming remote mode (preferred over `remote_list_cmd`). When
    /// set, this command is spawned ONCE and kept running; it must emit a
    /// persistent NDJSON stream — one compact JSON line per snapshot, each an
    /// array of Session objects (exactly what `agent-session-tui --serve-json`
    /// prints every few seconds). The host caches the latest line and reconnects
    /// if the stream drops. This gives a live, low-overhead remote view over a
    /// single connection instead of re-spawning a one-shot each refresh.
    ///
    /// Example (stream Copilot sessions from a remote box via devbox):
    ///   remote_stream_cmd = ["devbox", "exec", "gordon-devbox1",
    ///                        "agent-session-tui --serve-json 50 --interval 5"]
    ///
    /// Selection order: `remote_stream_cmd` > `remote_list_cmd` > local disk.
    #[serde(default)]
    pub remote_stream_cmd: Option<Vec<String>>,
}

fn default_launch_method() -> String {
    "wt".into()
}

/// Which engine produces group suggestions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum GroupingEngine {
    /// Opt-in hosted tab auto-grouping service. Sends session
    /// titles and working directories and returns suggested group names;
    /// see `src/grouping/remote.rs` for transmission and stability caveats.
    Remote,
    /// Word-overlap matching on titles plus short summary snippets — **not a
    /// local AI model**. It counts how many words the inputs share and groups
    /// them if more than half overlap, naming each group after its most common words (e.g.
    /// `benchmark-prompt-files`). No network or model. Also the automatic
    /// fallback whenever `Remote` fails.
    ///
    /// Accepts `"local"` as an alias — that was the original name, and it
    /// repeatedly read as "runs a local model", which it does not.
    #[serde(alias = "local")]
    #[default]
    WordMatch,
    /// Legacy: spawn the configured CLI (`[acp]`) as a one-shot subprocess.
    Acp,
}

/// Configuration for group suggestions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupingConfig {
    /// Engine to use for manual and automatic suggestions. Defaults to wordmatch.
    #[serde(default)]
    pub engine: GroupingEngine,
    /// Run once after discovery when true; false disables automatic grouping.
    /// When omitted, only the legacy ACP engine's explicit auto_suggest opt-in
    /// is honored. Manual suggestions remain available independently.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_suggest: Option<bool>,
    /// Language tag passed to a remote engine; controls the language generated
    /// group names are written in.
    #[serde(default = "default_grouping_language")]
    pub language: String,
    /// Whether to offer your existing group names to a remote engine so new
    /// sessions get folded into a group you already have.
    ///
    /// **Off by default**: the service accepts pre-assigned groups only
    /// erratically. Measured against the live endpoint with 11 real candidates,
    /// even 3 group names returned an empty body while 1 succeeded; with
    /// synthetic data 6 worked. Since the failure is payload-dependent and not
    /// a tunable limit, enabling this mostly costs a wasted round-trip before
    /// the automatic retry. Group reuse still happens locally — a session that
    /// clusters with an already-grouped one inherits that group.
    #[serde(default = "default_reuse_existing_groups")]
    pub reuse_existing_groups: bool,
    /// Maximum number of existing group names offered when
    /// [`Self::reuse_existing_groups`] is enabled.
    #[serde(default = "default_max_group_anchors")]
    pub max_group_anchors: usize,
    /// Maximum seconds to wait for a remote grouping call. This is a configured
    /// waiting policy, not evidence that a slower response is faulty.
    /// Actual latency depends on the service, network and workload.
    #[serde(default = "default_grouping_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_grouping_language() -> String {
    "en-US".into()
}

fn default_reuse_existing_groups() -> bool {
    false
}

fn default_max_group_anchors() -> usize {
    6
}

fn default_grouping_timeout_secs() -> u64 {
    20
}

impl Default for GroupingConfig {
    fn default() -> Self {
        Self {
            engine: GroupingEngine::default(),
            auto_suggest: None,
            language: default_grouping_language(),
            reuse_existing_groups: default_reuse_existing_groups(),
            max_group_anchors: default_max_group_anchors(),
            timeout_secs: default_grouping_timeout_secs(),
        }
    }
}

impl GroupingConfig {
    /// Resolve the automatic-run opt-in, preserving legacy ACP configuration.
    pub fn auto_suggest_enabled(&self, acp: &AcpConfig) -> bool {
        self.auto_suggest
            .unwrap_or(self.engine == GroupingEngine::Acp && acp.auto_suggest)
    }
}

/// Configuration for ACP-based AI group suggestions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpConfig {    /// The CLI command to use for AI suggestions (e.g., "codex", "qwen", "copilot").
    #[serde(default = "default_acp_command")]
    pub command: String,
    /// Extra args appended to the command (e.g., ["--model", "gpt-4o-mini"] for cost control).
    #[serde(default = "default_acp_extra_args")]
    pub extra_args: Vec<String>,
    /// Path to the prompt template file. Defaults to `<exe-dir>/prompts/group-suggest.md`.
    #[serde(default)]
    pub prompt_template: Option<PathBuf>,
    /// If true, automatically run AI grouping for the top 30 ungrouped sessions
    /// once initial discovery completes. Suggestions render inline in the
    /// Active view and can be accepted with `y`, dismissed with `n`, or edited
    /// with `e` while the cursor is on the session.
    #[serde(default = "default_acp_auto_suggest")]
    pub auto_suggest: bool,
    /// Maximum seconds to wait for an ACP run to complete. If exceeded, the
    /// run is cancelled and an error is logged. Default 180s — enough for a
    /// 30-session prompt against a slow model. Bump higher if your model is
    /// consistently slow; drop lower to detect hangs faster.
    #[serde(default = "default_acp_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_acp_command() -> String {
    "copilot".into()
}

/// Default extra args for the ACP run.
///
/// Uses `--effort low` for assigning sessions to thematic groups.
/// Runtime and suggestion quality depend on the CLI, model and workload;
/// no speedup or unchanged-quality guarantee is implied. Users can override
/// `extra_args` in config.toml.
fn default_acp_extra_args() -> Vec<String> {
    vec!["--effort".into(), "low".into()]
}

/// Default for `auto_suggest`. Off by default so:
/// (1) the TUI never spawns a `copilot` CLI without the user's awareness;
/// (2) no API/quota cost is incurred for users who don't want AI grouping;
/// (3) the feature requires `copilot` CLI auth, which not every user has set up.
/// Users who want it opt in via `[acp] auto_suggest = true` in config.toml.
fn default_acp_auto_suggest() -> bool {
    false
}

fn default_acp_timeout_secs() -> u64 {
    180
}

impl Default for AcpConfig {
    fn default() -> Self {
        Self {
            command: default_acp_command(),
            extra_args: default_acp_extra_args(),
            prompt_template: None,
            auto_suggest: default_acp_auto_suggest(),
            timeout_secs: default_acp_timeout_secs(),
        }
    }
}

fn default_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("agent-session-tui")
}

fn default_poll_interval_ms() -> u64 {
    5000
}

fn default_tick_rate_ms() -> u64 {
    1000
}

fn default_semantic_index_min_interval_ms() -> u64 {
    10_000
}

fn default_log_lines() -> usize {
    500
}

impl Default for AppConfig {
    fn default() -> Self {
        let mut providers = HashMap::new();

        // Copilot CLI — default provider
        providers.insert(
            "copilot".into(),
            ProviderConfig {
                enabled: true,
                default: true,
                command: "copilot".into(),
                default_args: vec![],
                state_dir: dirs::home_dir().map(|h| h.join(".copilot").join("session-state")),
                resume_flag: Some("--resume".into()),
                startup_dir: None,
                launch_method: "wt".into(),
                launch_cmd: None,
                launch_args: None,
                launch_fallback_cmd: None,
                launch_fallback_args: None,
                launch_fallback: Some("cmd".into()),
                wt_profile: None,
                remote_list_cmd: None,
                remote_stream_cmd: None,
            },
        );

        // Claude Code
        providers.insert(
            "claude".into(),
            ProviderConfig {
                enabled: true,
                default: false,
                command: "claude".into(),
                default_args: vec![],
                state_dir: dirs::home_dir().map(|h| h.join(".claude").join("projects")),
                resume_flag: Some("--resume".into()),
                startup_dir: None,
                launch_method: "wt".into(),
                launch_cmd: None,
                launch_args: None,
                launch_fallback_cmd: None,
                launch_fallback_args: None,
                launch_fallback: Some("cmd".into()),
                wt_profile: None,
                remote_list_cmd: None,
                remote_stream_cmd: None,
            },
        );

        // Codex CLI
        providers.insert(
            "codex".into(),
            ProviderConfig {
                enabled: true,
                default: false,
                command: "codex".into(),
                default_args: vec![],
                state_dir: dirs::home_dir().map(|h| h.join(".codex").join("sessions")),
                resume_flag: Some("resume".into()),
                startup_dir: None,
                launch_method: "wt".into(),
                launch_cmd: None,
                launch_args: None,
                launch_fallback_cmd: None,
                launch_fallback_args: None,
                launch_fallback: Some("cmd".into()),
                wt_profile: None,
                remote_list_cmd: None,
                remote_stream_cmd: None,
            },
        );

        // Gemini CLI
        providers.insert(
            "gemini".into(),
            ProviderConfig {
                enabled: true,
                default: false,
                command: "gemini".into(),
                default_args: vec![],
                state_dir: dirs::home_dir().map(|h| h.join(".gemini")),
                resume_flag: Some("--resume".into()),
                startup_dir: None,
                launch_method: "wt".into(),
                launch_cmd: None,
                launch_args: None,
                launch_fallback_cmd: None,
                launch_fallback_args: None,
                launch_fallback: Some("cmd".into()),
                wt_profile: None,
                remote_list_cmd: None,
                remote_stream_cmd: None,
            },
        );

        // Kimi
        providers.insert(
            "kimi".into(),
            ProviderConfig {
                enabled: true,
                default: false,
                command: "kimi".into(),
                default_args: vec![],
                state_dir: dirs::home_dir().map(|h| h.join(".kimi").join("sessions")),
                resume_flag: Some("--resume".into()),
                startup_dir: None,
                launch_method: "wt".into(),
                launch_cmd: None,
                launch_args: None,
                launch_fallback_cmd: None,
                launch_fallback_args: None,
                launch_fallback: Some("cmd".into()),
                wt_profile: None,
                remote_list_cmd: None,
                remote_stream_cmd: None,
            },
        );

        Self {
            data_dir: default_data_dir(),
            poll_interval_ms: default_poll_interval_ms(),
            tick_rate_ms: default_tick_rate_ms(),
            semantic_index_min_interval_ms: default_semantic_index_min_interval_ms(),
            log_max_lines: default_log_lines(),
            providers,
            acp: None,
            grouping: GroupingConfig::default(),
            remote_defaults: None,
            remotes: HashMap::new(),
        }
    }
}

impl AppConfig {
    /// Load config. Search order:
    /// 1. `config.toml` next to the executable
    /// 2. `%APPDATA%\agent-session-tui\config.toml`
    /// 3. Built-in defaults
    pub fn load() -> Result<Self> {
        let config_path = Self::config_path();
        if config_path.exists() {
            let text = std::fs::read_to_string(&config_path)?;
            let config: AppConfig = toml::from_str(&text)?;
            Ok(config)
        } else {
            Ok(Self::default())
        }
    }

    /// Resolve config path: next to exe first, then %APPDATA%.
    pub fn config_path() -> PathBuf {
        // 1. Next to the executable
        if let Ok(exe) = std::env::current_exe() {
            let beside_exe = exe
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join("config.toml");
            if beside_exe.exists() {
                return beside_exe;
            }
        }
        // 2. %APPDATA%\agent-session-tui\config.toml
        let appdata = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("agent-session-tui")
            .join("config.toml");
        if appdata.exists() {
            return appdata;
        }
        // 3. Default: next to exe (will be created there)
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|p| p.join("config.toml")))
            .unwrap_or_else(|| PathBuf::from("config.toml"))
    }

    /// Write default config to disk if it doesn't exist.
    pub fn write_default_if_missing(&self) -> Result<()> {
        let path = Self::config_path();
        if !path.exists() {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let text = toml::to_string_pretty(self)?;
            std::fs::write(&path, text)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AcpConfig must be `None` when the user did not write an `[acp]`
    /// section in their config.toml. This is the master gate for the
    /// AI grouping feature — without an explicit section, `s` shortcut
    /// stays hidden, auto-suggest is skipped, no copilot process spawns.
    #[test]
    fn acp_absent_yields_none() {
        let toml_str = r#"
            poll_interval_ms = 2000
            log_max_lines = 500
            tick_rate_ms = 1000
            semantic_index_min_interval_ms = 60000
            data_dir = "/tmp/test"
        "#;
        let cfg: AppConfig = toml::from_str(toml_str).expect("parse");
        assert!(cfg.acp.is_none(), "expected acp=None when section omitted");
    }

    /// An empty `[acp]` section is sufficient to opt in: defaults fill
    /// in all fields, and the user has clearly expressed intent.
    #[test]
    fn empty_acp_section_yields_some_with_defaults() {
        let toml_str = r#"
            poll_interval_ms = 2000
            log_max_lines = 500
            tick_rate_ms = 1000
            semantic_index_min_interval_ms = 60000
            data_dir = "/tmp/test"
            [acp]
        "#;
        let cfg: AppConfig = toml::from_str(toml_str).expect("parse");
        let acp = cfg.acp.expect("expected acp=Some when section present");
        assert_eq!(acp.command, "copilot", "default command");
        assert!(!acp.auto_suggest, "default auto_suggest=false");
    }

    /// Populated `[acp]` section round-trips correctly.
    #[test]
    fn populated_acp_section_parses_correctly() {
        let toml_str = r#"
            poll_interval_ms = 2000
            log_max_lines = 500
            tick_rate_ms = 1000
            semantic_index_min_interval_ms = 60000
            data_dir = "/tmp/test"
            [acp]
            command = "claude"
            extra_args = ["--model", "sonnet"]
            auto_suggest = true
            timeout_secs = 240
        "#;
        let cfg: AppConfig = toml::from_str(toml_str).expect("parse");
        let acp = cfg.acp.expect("acp set");
        assert_eq!(acp.command, "claude");
        assert_eq!(acp.extra_args, vec!["--model", "sonnet"]);
        assert!(acp.auto_suggest);
        assert_eq!(acp.timeout_secs, 240);
    }

    /// AppConfig::default() (the in-code fallback when no config.toml is
    /// found) must NOT opt the user in — defaults stay opt-out.
    #[test]
    fn default_app_config_has_no_acp() {
        let cfg = AppConfig::default();
        assert!(cfg.acp.is_none(), "default AppConfig must not opt into AI");
    }

    /// Regression: missing config must not silently select outbound grouping.
    #[test]
    fn grouping_defaults_to_wordmatch_when_section_absent() {
        let toml_str = r#"
            poll_interval_ms = 2000
        "#;
        let cfg: AppConfig = toml::from_str(toml_str).expect("parse");
        assert_eq!(cfg.grouping.engine, GroupingEngine::WordMatch);
        assert!(!cfg.grouping.auto_suggest_enabled(&AcpConfig::default()));
        assert_eq!(cfg.grouping.language, "en-US");
        assert_eq!(cfg.grouping.timeout_secs, 20);
    }

    #[test]
    fn default_app_config_uses_manual_wordmatch_grouping() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.grouping.engine, GroupingEngine::WordMatch);
        assert!(!cfg.grouping.auto_suggest_enabled(&AcpConfig::default()));
    }

    /// An empty `[grouping]` section must behave exactly like an absent one.
    #[test]
    fn empty_grouping_section_matches_the_default() {
        let cfg: AppConfig = toml::from_str("[grouping]\n").expect("parse");
        assert_eq!(cfg.grouping.engine, GroupingEngine::WordMatch);
        assert!(!cfg.grouping.auto_suggest_enabled(&AcpConfig::default()));
    }

    /// Opting out to the offline engine must be possible and must stick.
    #[test]
    fn local_engine_can_be_selected_explicitly() {
        let cfg: AppConfig =
            toml::from_str("[grouping]\nengine = \"local\"\n").expect("parse");
        assert_eq!(cfg.grouping.engine, GroupingEngine::WordMatch);
    }

    #[test]
    fn grouping_engine_parses_each_variant() {
        for (s, want) in [
            ("wordmatch", GroupingEngine::WordMatch),
            ("remote", GroupingEngine::Remote),
            ("acp", GroupingEngine::Acp),
        ] {
            let cfg: AppConfig =
                toml::from_str(&format!("[grouping]\nengine = \"{s}\"\n")).expect("parse");
            assert_eq!(cfg.grouping.engine, want, "engine = {s}");
            assert!(!cfg.grouping.auto_suggest_enabled(&AcpConfig::default()));
        }
    }

    #[test]
    fn automatic_grouping_requires_explicit_opt_in_for_each_engine() {
        for engine in ["wordmatch", "local", "remote", "acp"] {
            for enabled in [false, true] {
                let cfg: AppConfig = toml::from_str(&format!(
                    "[grouping]\nengine = \"{engine}\"\nauto_suggest = {enabled}\n[acp]\n"
                ))
                .unwrap();
                assert_eq!(
                    cfg.grouping.auto_suggest_enabled(cfg.acp.as_ref().unwrap()),
                    enabled,
                    "{engine}"
                );
            }
        }
    }

    #[test]
    fn legacy_acp_opt_in_is_preserved_but_never_enables_other_engines() {
        for engine in ["wordmatch", "remote", "acp"] {
            let text = format!("[grouping]\nengine = \"{engine}\"\n[acp]\nauto_suggest = true\n");
            let cfg: AppConfig = toml::from_str(&text).unwrap();
            assert_eq!(
                cfg.grouping.auto_suggest_enabled(cfg.acp.as_ref().unwrap()),
                engine == "acp"
            );
            let text = text.replace("[grouping]", "[grouping]\nauto_suggest = false");
            let cfg: AppConfig = toml::from_str(&text).unwrap();
            assert!(!cfg.grouping.auto_suggest_enabled(cfg.acp.as_ref().unwrap()));
        }
    }

    #[test]
    fn manual_remote_selection_survives_disabled_automatic_grouping() {
        let cfg: AppConfig =
            toml::from_str("[grouping]\nengine = \"remote\"\nauto_suggest = false\n").unwrap();
        let round_trip: AppConfig = toml::from_str(&toml::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(round_trip.grouping.engine, GroupingEngine::Remote);
        assert!(!round_trip
            .grouping
            .auto_suggest_enabled(&AcpConfig::default()));
    }

    #[test]
    fn privacy_notice_is_packaged_and_available_before_startup() {
        let release = include_str!("../.github/workflows/release.yml");
        assert_eq!(release.matches("cp PRIVACY.md dist/").count(), 2);
        let main = include_str!("main.rs");
        let startup = main.find("let log_path =").unwrap();
        assert!(main.find("arg == \"--privacy\"").unwrap() < startup);
        assert!(main.find("arg == \"--help\"").unwrap() < startup);
        assert!(main.contains("include_str!(\"../PRIVACY.md\")"));
        let policy = include_str!("../PRIVACY.md");
        for path in [
            "log_index_fingerprints.json",
            "embeddings_cache_v3.json",
            "agent-session-tui-scan-",
            "HF_HOME",
        ] {
            assert!(policy.contains(path), "{path}");
        }
    }

    #[test]
    fn localized_grouping_steps_match_current_and_legacy_templates() {
        let template = include_str!("../config.toml.template");
        assert!(template.lines().any(|line| line == "# engine = \"wordmatch\""));
        let translations = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("docs")
            .join("i18n");
        let mut checked = 0;
        for entry in std::fs::read_dir(translations).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy();
            if !name.starts_with("README.") || !name.ends_with(".md") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let step = text.lines().find(|line| line.starts_with("2. ")).unwrap();
            assert!(step.contains("# [grouping]"), "{name}");
            let current = step
                .find("# engine = \"wordmatch\"")
                .expect("current example");
            let legacy = step.find("# engine = \"remote\"").expect("legacy example");
            assert!(current < legacy, "{name}: current example must come first");
            assert_eq!(
                text.matches("# engine = \"wordmatch\"").count(),
                1,
                "{name}"
            );
            assert_eq!(text.matches("# engine = \"remote\"").count(), 1, "{name}");
            let block = text
                .split_once("```toml")
                .unwrap()
                .1
                .split("```")
                .next()
                .unwrap();
            let config: AppConfig = toml::from_str(block).unwrap();
            assert_eq!(config.grouping.engine, GroupingEngine::WordMatch, "{name}");
            assert_eq!(config.grouping.auto_suggest, Some(false), "{name}");
            checked += 1;
        }
        assert_eq!(checked, 15);
    }

    /// `local` was the original name for the word-overlap engine. It read as
    /// "runs a local model" — which it does not — so the canonical name is now
    /// `wordmatch`. Existing configs must keep working.
    #[test]
    fn legacy_local_engine_name_still_parses() {
        let cfg: AppConfig =
            toml::from_str("[grouping]\nengine = \"local\"\n").expect("legacy name must parse");
        assert_eq!(cfg.grouping.engine, GroupingEngine::WordMatch);
    }

    /// The canonical serialized form is `wordmatch`, not `local`.
    #[test]
    fn wordmatch_serializes_under_its_new_name() {
        let cfg = GroupingConfig {
            engine: GroupingEngine::WordMatch,
            ..Default::default()
        };
        let out = toml::to_string(&cfg).expect("serialize");
        assert!(out.contains("wordmatch"), "expected wordmatch in: {out}");
    }

    #[test]
    fn populated_grouping_section_round_trips() {
        let toml_str = r#"
            [grouping]
            engine = "remote"
            language = "zh-CN"
            timeout_secs = 45
        "#;
        let cfg: AppConfig = toml::from_str(toml_str).expect("parse");
        assert_eq!(cfg.grouping.engine, GroupingEngine::Remote);
        assert_eq!(cfg.grouping.language, "zh-CN");
        assert_eq!(cfg.grouping.timeout_secs, 45);
    }
}
