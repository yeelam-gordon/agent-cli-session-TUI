use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::ProviderConfig;
use crate::models::*;
use crate::process_info;
use crate::provider::{Provider, ProviderCapabilities};
use crate::util::truncate_str_safe;

/// Qwen CLI provider.
///
/// Reads session state from:
///   `~/.qwen/projects/<encoded-cwd>/chats/<session-id>.jsonl`
///
/// Structure is similar to Claude but with a `chats/` subdirectory.
/// JSONL event types: user, assistant, system, tool_result
/// User content in message.parts[].text, role is "user" or "model".
pub struct QwenProvider {
    config: ProviderConfig,
    projects_dir: PathBuf,
}

impl QwenProvider {
    pub fn new(config: &ProviderConfig) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let projects_dir = config
            .state_dir
            .clone()
            .unwrap_or_else(|| home.join(".qwen").join("projects"));

        Self {
            config: config.clone(),
            projects_dir,
        }
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
        let event_count = lines.len();
        let mut first_user_msg: Option<String> = None;
        let mut last_assistant_msg: Option<String> = None;
        let mut first_timestamp: Option<String> = None;
        let mut last_timestamp: Option<chrono::DateTime<chrono::Utc>> = None;
        let mut last_role = String::new();

        for line in &lines {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                if let Some(ts) = val.get("timestamp").and_then(|v| v.as_str()) {
                    if first_timestamp.is_none() {
                        first_timestamp = Some(ts.to_string());
                    }
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
                        last_timestamp = Some(dt.with_timezone(&chrono::Utc));
                    }
                }

                let event_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");

                if event_type == "user" {
                    last_role = "user".into();
                    if first_user_msg.is_none() {
                        if let Some(content) = Self::extract_text_parts(&val) {
                            first_user_msg = Some(truncate_str_safe(&content, 300));
                        }
                    }
                }

                if event_type == "assistant" {
                    last_role = "assistant".into();
                    if let Some(content) = Self::extract_text_parts(&val) {
                        last_assistant_msg = Some(truncate_str_safe(&content, 500));
                    }
                }
            }
        }

        let last_event_age_secs = last_timestamp.map(|ts| {
            chrono::Utc::now().signed_duration_since(ts).num_seconds().max(0) as u64
        });

        let waiting_for_user = last_role == "assistant";
        let has_user = first_user_msg.is_some();

        JsonlScanResult {
            first_user_msg,
            last_assistant_msg,
            has_user,
            event_count,
            first_timestamp,
            last_event_age_secs,
            waiting_for_user,
            file_mtime,
        }
    }

    /// Extract text from message.parts[].text
    fn extract_text_parts(val: &serde_json::Value) -> Option<String> {
        let parts = val.get("message")?.get("parts")?.as_array()?;
        let mut texts = Vec::new();
        for part in parts {
            if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                if !text.trim().is_empty() {
                    texts.push(text.trim().to_string());
                }
            }
        }
        if texts.is_empty() { None } else { Some(texts.join("\n")) }
    }

    /// Decode project path: same encoding as Claude (C--Users-yeelam → C:\Users\yeelam)
    fn decode_project_path(encoded: &str) -> PathBuf {
        let (drive, remainder) = match encoded.find("--") {
            Some(pos) => {
                let drive = format!("{}:\\", &encoded[..pos]);
                let rest = if pos + 2 < encoded.len() { &encoded[pos + 2..] } else { "" };
                (drive, rest)
            }
            None => return PathBuf::from(encoded),
        };
        if remainder.is_empty() {
            return PathBuf::from(&drive);
        }
        // Simple decode: replace - with path separator
        let decoded = remainder.replace('-', "\\");
        PathBuf::from(format!("{}{}", drive, decoded))
    }
}

#[derive(Debug, Default)]
struct JsonlScanResult {
    first_user_msg: Option<String>,
    last_assistant_msg: Option<String>,
    has_user: bool,
    event_count: usize,
    first_timestamp: Option<String>,
    last_event_age_secs: Option<u64>,
    waiting_for_user: bool,
    file_mtime: Option<String>,
}

impl Provider for QwenProvider {
    fn name(&self) -> &str {
        "Qwen CLI"
    }

    fn key(&self) -> &str {
        "qwen"
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

        if !self.projects_dir.exists() {
            return Ok(sessions);
        }

        let project_dirs = std::fs::read_dir(&self.projects_dir)
            .with_context(|| format!("Cannot read projects dir: {:?}", self.projects_dir))?;

        for proj_entry in project_dirs.flatten() {
            if !proj_entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                continue;
            }

            let proj_name = proj_entry.file_name().to_string_lossy().to_string();
            let cwd = Self::decode_project_path(&proj_name);

            // Qwen stores sessions in chats/ subdirectory
            let chats_dir = proj_entry.path().join("chats");
            let jsonl_files = match std::fs::read_dir(&chats_dir) {
                Ok(entries) => entries,
                Err(_) => continue,
            };

            for file_entry in jsonl_files.flatten() {
                let fname = file_entry.file_name().to_string_lossy().to_string();
                if !fname.ends_with(".jsonl") {
                    continue;
                }

                let session_id = fname.trim_end_matches(".jsonl").to_string();
                let scan = self.scan_jsonl(&file_entry.path());

                if !scan.has_user || scan.event_count < 3 {
                    continue;
                }

                let file_mtime = scan.file_mtime.clone().unwrap_or_default();
                let created_at = scan.first_timestamp.clone().unwrap_or_else(|| file_mtime.clone());

                let mut summary = String::new();
                if let Some(ref msg) = scan.first_user_msg {
                    summary = format!("--- First message ---\n{}", msg);
                }
                if let Some(ref msg) = scan.last_assistant_msg {
                    summary = format!("{}\n\n--- Last Qwen response ---\n{}", summary, msg);
                }

                let title = scan.first_user_msg.as_ref()
                    .map(|m| truncate_str_safe(m.lines().next().unwrap_or(m), 60))
                    .unwrap_or_else(|| session_id[..8.min(session_id.len())].to_string());

                sessions.push(Session {
                    id: format!("qwen_{}_{}", proj_name, session_id),
                    provider_session_id: session_id,
                    provider_name: "qwen".into(),
                    cwd: cwd.clone(),
                    title,
                    summary,
                    state: SessionState::default(),
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
        let procs = process_info::discover_processes("qwen");

        for session in sessions.iter_mut() {
            for (pid, proc) in &procs {
                if proc.command_line.contains(&session.provider_session_id) {
                    session.pid = Some(*pid);
                    session.state = SessionState {
                        process: ProcessState::Running,
                        interaction: if session.state.interaction == InteractionState::Unknown {
                            InteractionState::Busy
                        } else {
                            session.state.interaction
                        },
                        persistence: PersistenceState::Resumable,
                        health: HealthState::Clean,
                        confidence: Confidence::Medium,
                        reason: "process matched by session ID in command line".into(),
                    };
                    break;
                }
            }
        }

        // Infer state for sessions without a running process
        for session in sessions.iter_mut() {
            if session.pid.is_none() {
                session.state = SessionState {
                    process: ProcessState::Exited,
                    interaction: InteractionState::Idle,
                    persistence: PersistenceState::Resumable,
                    health: HealthState::Clean,
                    confidence: Confidence::High,
                    reason: "no process found, session file exists".into(),
                };
            }
        }

        Ok(())
    }
}
