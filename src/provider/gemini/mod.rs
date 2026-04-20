use std::path::{Path, PathBuf};
use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::config::ProviderConfig;
use crate::models::*;
use crate::process_info;
use crate::provider::{ActivitySource, Provider, ProviderCapabilities, SessionDetail};
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
                if !event_type.is_empty() {
                    last_type = event_type.to_string();
                }

                if event_type == "user" {
                    if first_user_msg.is_none() {
                        if let Some(content) = Self::extract_content_text(&val) {
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

        // If last message was from Gemini, it's waiting for user
        let waiting_for_user = last_type == "gemini";
        let assistant_working = last_type == "user";
        let has_user = first_user_msg.is_some();

        JsonlScanResult {
            first_user_msg,
            last_gemini_msg,
            has_user,
            event_count: lines.len(),
            first_timestamp,
            last_event_age_secs,
            waiting_for_user,
            assistant_working,
            file_mtime,
        }
    }

    /// Extract text from content (handles string or array of parts)
    fn extract_content_text(val: &serde_json::Value) -> Option<String> {
        let content = val.get("content")?;
        
        // Handle direct string
        if let Some(s) = content.as_str() {
            if !s.trim().is_empty() { return Some(s.trim().to_string()); }
        }

        // Handle array of parts
        if let Some(arr) = content.as_array() {
            let mut texts = Vec::new();
            for item in arr {
                if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                    if !text.trim().is_empty() {
                        texts.push(text.trim().to_string());
                    }
                }
            }
            if !texts.is_empty() { return Some(texts.join("\n")); }
        }
        None
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
    assistant_working: bool,
    file_mtime: Option<String>,
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

            let jsonl_files = match std::fs::read_dir(&chats_dir) {
                Ok(e) => e,
                Err(_) => continue,
            };

            for file_entry in jsonl_files.flatten() {
                let fname = file_entry.file_name().to_string_lossy().to_string();
                if !fname.ends_with(".jsonl") || !fname.starts_with("session-") {
                    continue;
                }

                let provider_id = fname.strip_prefix("session-")
                    .and_then(|s| s.strip_suffix(".jsonl"))
                    .unwrap_or(&fname)
                    .to_string();

                let scan = self.scan_jsonl(&file_entry.path());
                if !scan.has_user || scan.event_count < 2 {
                    continue;
                }

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
                    .unwrap_or_else(|| provider_id[..8.min(provider_id.len())].to_string());

                sessions.push(Session {
                    id: format!("gemini_{}_{}", proj_name, provider_id),
                    provider_session_id: provider_id,
                    provider_name: "gemini".into(),
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
        let processes = process_info::discover_processes("gemini");
        crate::log::info(&format!("Gemini match_processes: found {} potential processes", processes.len()));
        
        let mut live: Vec<(u32, Option<String>)> = Vec::new();
        for (pid, entry) in &processes {
            let cmd_lower = entry.command_line.to_lowercase();
            // Match "gemini" but exclude the TUI itself
            if cmd_lower.contains("gemini") && !cmd_lower.contains("agent-session-tui") {
                let id = process_info::extract_flag_value(&entry.command_line, "--session-id");
                live.push((*pid, id));
            }
        }

        let mut claimed_pids = HashSet::new();

        for session in sessions.iter_mut() {
            // 1. Try matching by ID in command line
            let matched = live.iter().find(|(_, sid)| {
                sid.as_ref().map(|s| s == &session.provider_session_id).unwrap_or(false)
            });

            let mut current_pid = None;
            let process_alive = if let Some((pid, _)) = matched {
                current_pid = Some(*pid);
                claimed_pids.insert(*pid);
                true
            } else {
                // 2. Fallback: Heuristic matching for the "current" session
                let updated_dt = session.updated_at.parse::<chrono::DateTime<chrono::FixedOffset>>().ok();
                let recently_active = updated_dt.map(|dt| {
                    let age = chrono::Utc::now().signed_duration_since(dt.with_timezone(&chrono::Utc)).num_seconds().abs();
                    age < 120
                }).unwrap_or(false);

                if recently_active {
                    if let Some((pid, _)) = live.iter().find(|(p, _)| !claimed_pids.contains(p)) {
                        current_pid = Some(*pid);
                        claimed_pids.insert(*pid);
                        true
                    } else { false }
                } else { false }
            };

            session.pid = current_pid;

            // Re-scan to get fresh signals
            let jsonl_path = session.state_dir.as_ref().map(|d| d.join(format!("session-{}.jsonl", session.provider_session_id)));
            let scan = jsonl_path.as_ref().map(|p| self.scan_jsonl(p)).unwrap_or_default();
            
            if let Some(ref mtime) = scan.file_mtime {
                session.updated_at = mtime.clone();
            }

            let signals = StateSignals {
                process_alive: Some(process_alive),
                pid: session.pid,
                lock_file_exists: None,
                lock_file_pid: None,
                last_event_age_secs: scan.last_event_age_secs,
                has_unfinished_turn: Some(scan.assistant_working),
                recent_tool_activity: None,
                cpu_active: None,
            };

            session.state = self.infer_state(&signals);

            if process_alive {
                if scan.waiting_for_user {
                    session.state.interaction = InteractionState::WaitingInput;
                    session.state.confidence = Confidence::High;
                } else if scan.assistant_working {
                    session.state.interaction = InteractionState::Busy;
                    session.state.confidence = Confidence::High;
                }
            } else {
                if scan.waiting_for_user {
                    session.state.interaction = InteractionState::WaitingInput;
                }
            }

            session.state.reason = format!(
                "process={} waiting={} working={} age={:?}s",
                process_alive, scan.waiting_for_user, scan.assistant_working, scan.last_event_age_secs
            );
        }

        Ok(())
    }

    fn session_detail(&self, session: &Session) -> Result<SessionDetail> {
        let jsonl_path = session.state_dir.as_ref().map(|d| d.join(format!("session-{}.jsonl", session.provider_session_id)));
        let scan = jsonl_path.as_ref().map(|p| self.scan_jsonl(p)).unwrap_or_default();

        Ok(SessionDetail {
            title: Some(session.title.clone()),
            summary: scan.first_user_msg,
            plan_items: vec![],
        })
    }

    fn activity_sources(&self, session: &Session) -> Result<Vec<ActivitySource>> {
        let mut sources = Vec::new();
        if let Some(ref dir) = session.state_dir {
            let jsonl = dir.join(format!("session-{}.jsonl", session.provider_session_id));
            if jsonl.exists() {
                sources.push(ActivitySource::EventStream(jsonl));
            }
        }
        Ok(sources)
    }
}
