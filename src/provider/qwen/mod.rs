use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::config::ProviderConfig;
use crate::models::*;
use crate::provider::{Provider, ProviderCapabilities};
use uuid::Uuid;

/// Qwen CLI provider.
///
/// Reads session state from files only (example placeholder).
/// Adjust paths and logic as needed for the actual qwen CLI.
pub struct QwenProvider {
    config: ProviderConfig,
    state_dir: PathBuf,
}

impl QwenProvider {
    pub fn new(config: &ProviderConfig) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let qwen_dir = home.join(".qwen");
        let state_dir = config
            .state_dir
            .clone()
            .unwrap_or_else(|| qwen_dir.join("session-state"));

        Self {
            config: config.clone(),
            state_dir,
        }
    }

    /// Example: read session directories
    fn read_sessions(&self) -> Result<Vec<PathBuf>> {
        let mut sessions = Vec::new();
        if self.state_dir.exists() {
            for entry in std::fs::read_dir(&self.state_dir)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    sessions.push(entry.path());
                }
            }
        }
        Ok(sessions)
    }

    /// Example: read session summary from a file
    fn read_summary(&self, session_dir: &Path) -> Option<String> {
        let summary_path = session_dir.join("summary.txt");
        if summary_path.exists() {
            std::fs::read_to_string(summary_path).ok()
        } else {
            None
        }
    }
}

impl Provider for QwenProvider {
    fn name(&self) -> &str {
        "Qwen CLI"
    }

    fn key(&self) -> &str {
        "qwen"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::default()
    }

    fn discover_sessions(&self) -> Result<Vec<Session>> {
        let mut sessions = Vec::new();
        let session_dirs = self.read_sessions()?;
        for dir in session_dirs {
            let summary = self.read_summary(&dir).unwrap_or_else(|| "No summary".to_string());
            let session = Session {
                id: uuid::Uuid::new_v4().to_string(),
                provider_session_id: dir.file_name().unwrap().to_string_lossy().to_string(),
                provider_name: self.key().to_string(),
                title: format!("Qwen Session {}", dir.file_name().unwrap().to_string_lossy()),
                summary,
                cwd: dir.clone(),
                created_at: "".to_string(),
                updated_at: "".to_string(),
                state: SessionState::default(),
                pid: None,
                state_dir: Some(dir),
            };
            sessions.push(session);
        }
        Ok(sessions)
    }

    fn match_processes(&self, sessions: &mut [Session]) -> Result<()> {
        // TODO: Implement process matching logic
        Ok(())
    }
}
