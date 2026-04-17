use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_db_path")]
    pub db_path: PathBuf,
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
    #[serde(default = "default_log_lines")]
    pub log_max_lines: usize,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub enabled: bool,
    /// The CLI command to invoke (e.g., "copilot", "claude").
    /// Can be a full path like "C:\\Program Files\\GitHub\\copilot.exe".
    pub command: String,
    /// Extra args always passed to the CLI.
    #[serde(default)]
    pub default_args: Vec<String>,
    /// Where the CLI stores its session state (provider discovers from here).
    pub state_dir: Option<PathBuf>,
    /// Override the resume flag (e.g., "--resume" for copilot).
    pub resume_flag: Option<String>,
    /// Default working directory when starting new sessions.
    /// If not set, uses the current working directory.
    #[serde(default)]
    pub startup_dir: Option<PathBuf>,
    /// How to launch the session. Options: "wt" (Windows Terminal tab),
    /// "cmd" (new cmd window), "pwsh" (new PowerShell window).
    /// Defaults to "wt" on Windows.
    #[serde(default = "default_launch_method")]
    pub launch_method: String,
    /// Optional: Windows Terminal profile name to use (e.g., "PowerShell", "Command Prompt").
    #[serde(default)]
    pub wt_profile: Option<String>,
}

fn default_launch_method() -> String {
    "wt".into()
}

fn default_db_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("agent-session-tui")
        .join("sessions.db")
}

fn default_poll_interval_ms() -> u64 {
    2000
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
                command: "copilot".into(),
                default_args: vec![],
                state_dir: dirs::home_dir().map(|h| h.join(".copilot").join("session-state")),
                resume_flag: Some("--resume".into()),
                startup_dir: None,
                launch_method: "wt".into(),
                wt_profile: None,
            },
        );

        // Claude Code
        providers.insert(
            "claude".into(),
            ProviderConfig {
                enabled: true,
                command: "claude".into(),
                default_args: vec![],
                state_dir: dirs::home_dir().map(|h| h.join(".claude").join("projects")),
                resume_flag: Some("--resume".into()),
                startup_dir: None,
                launch_method: "wt".into(),
                wt_profile: None,
            },
        );

        Self {
            db_path: default_db_path(),
            poll_interval_ms: default_poll_interval_ms(),
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
            let beside_exe = exe.parent().unwrap_or(std::path::Path::new(".")).join("config.toml");
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
