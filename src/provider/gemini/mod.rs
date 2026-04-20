use std::path::{Path, PathBuf};
use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::config::ProviderConfig;
use crate::models::*;
use crate::process_info;
use crate::provider::{Provider, ProviderCapabilities};
use crate::util::truncate_str_safe;

/// Gemini CLI provider.
///
/// Reads session state from:
///   `~/.gemini/tmp/<project-name>/chats/session-*.jsonl`
///
/// Projects map is in:
///   `~/.gemini/projects.json`
pub struct GeminiProvider {
    config: ProviderConfig,
    state_dir: PathBuf,
}

#[derive(Debug, Deserialize)]
struct ProjectsConfig {
    projects: HashMap<String, String>,
}

impl GeminiProvider {
    pub fn new(config: &ProviderConfig) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let state_dir = config
            .state_dir
            .clone()
            .unwrap_or_else(|| home.join(".gemini"));

        Self {
            config: config.clone(),
            state_dir,
        }
    }

    /// Load the projects map (CWD -> Project Name)
    fn load_projects(&self) -> HashMap<String, String> {
        let projects_json = self.state_dir.join("projects.json");
        if let Ok(content) = std::fs::read_to_string(projects_json) {
            if let Ok(cfg) = serde_json::from_str::<ProjectsConfig>(&content) {
                return cfg.projects;
            }
        }
        HashMap::new()
    }

    /// Single-pass scan of a JSONL session file.
    fn scan_jsonl(&self, jsonl_path: &Path) -> JsonlScanResult {
        let file_mtime = std::fs::metadata(jsonl_path)
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|t| {
                let dt: chrono::DateTime<chrono::Local> = t.into();
                dt.to_rfc3339()
            });

        let text = match std::fs::read_to_string(jsonl_path) {
            Ok(t) => t,
            Err(_) => return JsonlScanResult { file_mtime, ..Default::default() },
        };

        let lines: Vec<&str> = text.lines().collect();
        let mut first_user_msg: Option<String> = None;
        let mut last_gemini_msg: Option<String> = None;
        let mut first_timestamp: Option<String> = None;
        let mut last_timestamp: Option<chrono::DateTime<chrono::Utc>> = None;
        let mut last_type = String::new();
        let mut session_id_full: Option<String> = None;

        for line in &lines {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                // Header line has sessionId
                if session_id_full.is_none() {
                    if let Some(sid) = val.get("sessionId").and_then(|v| v.as_str()) {
                        session_id_full = Some(sid.to_string());
                    }
                }

                if let Some(ts) = val.get("timestamp").and_then(|v| v.as_str()) {
                    if first_timestamp.is_none() {
                        first_timestamp = Some(ts.to_string());
                    }
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
                        last_timestamp = Some(dt.with_timezone(&chrono::Utc));
                    }
                }

                let event_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if !event_type.is_empty() {
                    last_type = event_type.to_string();
                }

                if event_type == "user" {
                    if first_user_msg.is_none() {
                        if let Some(content) = Self::extract_user_text(&val) {
                            first_user_msg = Some(truncate_str_safe(&content, 300));
                        }
                    }
                }

                if event_type == "gemini" {
                    if let Some(content) = val.get("content").and_then(|v| v.as_str()) {
                        last_gemini_msg = Some(truncate_str_safe(content, 500));
                    }
                }
            }
        }

        let last_event_age_secs = last_timestamp.map(|ts| {
            chrono::Utc::now().signed_duration_since(ts).num_seconds().max(0) as u64
        });

        let waiting_for_user = last_type == "gemini";
        let has_user = first_user_msg.is_some();

        JsonlScanResult {
            first_user_msg,
            last_gemini_msg,
            has_user,
            event_count: lines.len(),
            first_timestamp,
            last_event_age_secs,
            waiting_for_user,
            file_mtime,
            session_id_full,
        }
    }

    fn extract_user_text(val: &serde_json::Value) -> Option<String> {
        let content = val.get("content")?.as_array()?;
        let mut texts = Vec::new();
        for item in content {
            if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                if !text.trim().is_empty() {
                    texts.push(text.trim().to_string());
                }
            }
        }
        if texts.is_empty() { None } else { Some(texts.join("\n")) }
    }
}

#[derive(Debug, Default)]
struct JsonlScanResult {
    first_user_msg: Option<String>,
    last_gemini_msg: Option<String>,
    has_user: bool,
    event_count: usize,
    first_timestamp: Option<String>,
    last_event_age_secs: Option<u64>,
    waiting_for_user: bool,
    file_mtime: Option<String>,
    session_id_full: Option<String>,
}

impl Provider for GeminiProvider {
    fn name(&self) -> &str {
        "Gemini CLI"
    }

    fn key(&self) -> &str {
        "gemini"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_resume: true,
            supports_discovery: true,
            supports_logs: true,
            supports_wait_detection: true,
            supports_kill: true,
            supports_archive: false,
            supports_summary_extraction: true,
        }
    }

    fn discover_sessions(&self) -> Result<Vec<Session>> {
        let mut sessions = Vec::new();
        let projects_map = self.load_projects();
        let tmp_dir = self.state_dir.join("tmp");

        if !tmp_dir.exists() {
            return Ok(sessions);
        }

        let entries = std::fs::read_dir(&tmp_dir)
            .with_context(|| format!("Cannot read tmp dir: {:?}", tmp_dir))?;

        // Invert projects_map: Name -> CWD
        let name_to_cwd: HashMap<String, PathBuf> = projects_map
            .into_iter()
            .map(|(cwd, name)| (name, PathBuf::from(cwd)))
            .collect();

        for entry in entries.flatten() {
            if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                continue;
            }

            let proj_name = entry.file_name().to_string_lossy().to_string();
            let cwd = name_to_cwd.get(&proj_name).cloned().unwrap_or_else(|| PathBuf::from("."));

            let chats_dir = entry.path().join("chats");
            if !chats_dir.exists() {
                continue;
            }

            let jsonl_files = std::fs::read_dir(&chats_dir)?;
            for file_entry in jsonl_files.flatten() {
                let fname = file_entry.file_name().to_string_lossy().to_string();
                if !fname.ends_with(".jsonl") || !fname.starts_with("session-") {
                    continue;
                }

                let scan = self.scan_jsonl(&file_entry.path());
                if !scan.has_user || scan.event_count < 2 {
                    continue;
                }

                let session_id = scan.session_id_full.clone().unwrap_or_else(|| {
                    // Fallback to filename parts
                    fname.split('-').last().unwrap_or(&fname).replace(".jsonl", "")
                });

                let file_mtime = scan.file_mtime.clone().unwrap_or_default();
                let created_at = scan.first_timestamp.clone().unwrap_or_else(|| file_mtime.clone());

                let mut summary = String::new();
                if let Some(ref msg) = scan.first_user_msg {
                    summary = format!("--- First message ---\n{}", msg);
                }
                if let Some(ref msg) = scan.last_gemini_msg {
                    summary = format!("{}\n\n--- Last Gemini response ---\n{}", summary, msg);
                }

                let title = scan.first_user_msg.as_ref()
                    .map(|m| truncate_str_safe(m.lines().next().unwrap_or(m), 60))
                    .unwrap_or_else(|| session_id[..8.min(session_id.len())].to_string());

                sessions.push(Session {
                    id: format!("gemini_{}_{}", proj_name, session_id),
                    provider_session_id: session_id,
                    provider_name: "gemini".into(),
                    cwd: cwd.clone(),
                    title,
                    summary,
                    state: SessionState {
                        interaction: if scan.waiting_for_user { InteractionState::WaitingInput } else { InteractionState::Busy },
                        ..SessionState::default()
                    },
                    pid: None,
                    created_at,
                    updated_at: file_mtime,
                    state_dir: Some(chats_dir.clone()),
                });
            }
        }

        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(sessions)
    }

    fn match_processes(&self, sessions: &mut [Session]) -> Result<()> {
        let procs = process_info::discover_processes("gemini");

        for session in sessions.iter_mut() {
            for (pid, proc) in &procs {
                // Gemini CLI usually shows session ID in command line or path
                if proc.command_line.contains(&session.provider_session_id) {
                    session.pid = Some(*pid);
                    session.state = SessionState {
                        process: ProcessState::Running,
                        interaction: session.state.interaction, // Keep from scan
                        persistence: PersistenceState::Resumable,
                        health: HealthState::Clean,
                        confidence: Confidence::High,
                        reason: "process alive".into(),
                    };
                    break;
                }
            }
            
            // Fallback for non-running sessions
            if session.pid.is_none() {
                session.state = SessionState {
                    process: ProcessState::Exited,
                    interaction: InteractionState::Idle,
                    persistence: PersistenceState::Resumable,
                    health: HealthState::Clean,
                    confidence: Confidence::High,
                    reason: "no process found".into(),
                };
            }
        }
        Ok(())
    }
}
