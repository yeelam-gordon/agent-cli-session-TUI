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
    /// Launch method shortcut: "wt" | "pwsh" | "cmd". Ignored if launch_cmd is set.
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
}

fn default_launch_method() -> String {
    "wt".into()
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
            },
        );

        Self {
            data_dir: default_data_dir(),
            poll_interval_ms: default_poll_interval_ms(),
            tick_rate_ms: default_tick_rate_ms(),
            semantic_index_min_interval_ms: default_semantic_index_min_interval_ms(),
            log_max_lines: default_log_lines(),
            providers,
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


