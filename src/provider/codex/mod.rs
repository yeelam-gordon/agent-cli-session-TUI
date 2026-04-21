use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::ProviderConfig;
use crate::models::*;
use crate::process_info;
use crate::provider::{ActivitySource, Provider, ProviderCapabilities, SessionDetail};
use crate::util::truncate_str_safe;

/// Codex CLI provider.
///
/// Reads session state from:
///   `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`
///
/// Each JSONL file is one interactive session. The most useful records are:
///   - `session_meta` for session id, cwd, and created timestamp
///   - `response_item` messages for user/assistant text
///   - `event_msg` task_started/task_complete for busy vs waiting detection
pub struct CodexProvider {
    config: ProviderConfig,
    sessions_dir: PathBuf,
}

impl CodexProvider {
    pub fn new(config: &ProviderConfig) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let sessions_dir = config
            .state_dir
            .clone()
            .unwrap_or_else(|| home.join(".codex").join("sessions"));

        Self {
            config: config.clone(),
            sessions_dir,
        }
    }

    fn collect_session_files(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let mut stack = vec![self.sessions_dir.clone()];

        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };

            for entry in entries.flatten() {
                let path = entry.path();
                if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                    stack.push(path);
                } else if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                    files.push(path);
                }
            }
        }

        files
    }

    fn scan_session_file(&self, path: &Path) -> SessionScan {
        let file_mtime = std::fs::metadata(path)
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|t| {
                let dt: chrono::DateTime<chrono::Local> = t.into();
                dt.to_rfc3339()
            });

        let text = match std::fs::read_to_string(path) {
            Ok(text) if !text.trim().is_empty() => text,
            _ => {
                return SessionScan {
                    file_mtime,
                    ..Default::default()
                }
            }
        };

        let mut scan = SessionScan {
            file_mtime,
            ..Default::default()
        };

        for line in text.lines() {
            let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };

            if let Some(ts) = val.get("timestamp").and_then(|v| v.as_str()) {
                if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
                    scan.last_timestamp = Some(dt.with_timezone(&chrono::Utc));
                }
            }

            match val.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                "session_meta" => self.scan_session_meta(&val, &mut scan),
                "event_msg" => self.scan_event_msg(&val, &mut scan),
                "response_item" => self.scan_response_item(&val, &mut scan),
                _ => {}
            }
        }

        scan
    }

    fn scan_session_meta(&self, val: &serde_json::Value, scan: &mut SessionScan) {
        let Some(payload) = val.get("payload") else {
            return;
        };

        if scan.session_id.is_none() {
            scan.session_id = payload
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
        }

        if scan.created_at.is_none() {
            scan.created_at = payload
                .get("timestamp")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
        }

        if scan.cwd.is_none() {
            scan.cwd = payload
                .get("cwd")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(PathBuf::from);
        }
    }

    fn scan_event_msg(&self, val: &serde_json::Value, scan: &mut SessionScan) {
        let Some(payload) = val.get("payload") else {
            return;
        };

        match payload.get("type").and_then(|v| v.as_str()).unwrap_or("") {
            "task_started" => {
                scan.active_task = true;
                scan.last_activity_kind = Some("task_started".into());
            }
            "task_complete" => {
                scan.active_task = false;
                scan.last_activity_kind = Some("task_complete".into());
            }
            "user_message" => {
                scan.last_role = Some("user".into());
                scan.last_activity_kind = Some("user_message".into());
            }
            _ => {}
        }
    }

    fn scan_response_item(&self, val: &serde_json::Value, scan: &mut SessionScan) {
        let Some(payload) = val.get("payload") else {
            return;
        };
        if payload.get("type").and_then(|v| v.as_str()) != Some("message") {
            return;
        }

        let role = payload.get("role").and_then(|v| v.as_str()).unwrap_or("");
        let content: Option<String> = payload
            .get("content")
            .and_then(|v| v.as_array())
            .map(|items| Self::flatten_message_content(items))
            .filter(|s| !s.trim().is_empty());

        match role {
            "user" => {
                if scan.first_user_msg.is_none() {
                    scan.first_user_msg =
                        content.as_ref().map(|s| truncate_str_safe(s.trim(), 500));
                }
                if let Some(text) = content {
                    scan.last_user_msg = Some(text.trim().to_string());
                }
                scan.last_role = Some("user".into());
                scan.last_activity_kind = Some("user".into());
            }
            "assistant" => {
                if let Some(text) = content {
                    scan.prev_assistant_msg = scan.last_assistant_msg.take();
                    scan.last_assistant_msg = Some(text.trim().to_string());
                }
                scan.last_role = Some("assistant".into());
                scan.last_activity_kind = Some("assistant".into());
            }
            _ => {}
        }
    }

    fn flatten_message_content(items: &[serde_json::Value]) -> String {
        let mut parts = Vec::new();

        for item in items {
            if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                }
                continue;
            }

            if let Some(text) = item.get("input_text").and_then(|v| v.as_str()) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                }
                continue;
            }

            if let Some(text) = item.get("output_text").and_then(|v| v.as_str()) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                }
            }
        }

        parts.join("\n")
    }
}

#[derive(Debug, Default)]
struct SessionScan {
    session_id: Option<String>,
    cwd: Option<PathBuf>,
    created_at: Option<String>,
    file_mtime: Option<String>,
    first_user_msg: Option<String>,
    last_user_msg: Option<String>,
    prev_assistant_msg: Option<String>,
    last_assistant_msg: Option<String>,
    active_task: bool,
    last_role: Option<String>,
    last_activity_kind: Option<String>,
    last_timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

impl SessionScan {
    fn has_user_content(&self) -> bool {
        self.first_user_msg.is_some() || self.last_user_msg.is_some()
    }

    fn last_event_age_secs(&self) -> Option<u64> {
        self.last_timestamp.map(|ts| {
            chrono::Utc::now()
                .signed_duration_since(ts)
                .num_seconds()
                .max(0) as u64
        })
    }
}

impl Provider for CodexProvider {
    fn name(&self) -> &str {
        let _ = &self.config;
        "Codex CLI"
    }

    fn key(&self) -> &str {
        "codex"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_resume: true,
            supports_discovery: true,
            supports_logs: true,
            supports_wait_detection: true,
            supports_kill: true,
            supports_archive: true,
            supports_summary_extraction: true,
        }
    }

    fn discover_sessions(&self) -> Result<Vec<Session>> {
        if !self.sessions_dir.exists() {
            return Ok(Vec::new());
        }

        let mut sessions = Vec::new();

        for path in self.collect_session_files() {
            let scan = self.scan_session_file(&path);
            if !scan.has_user_content() {
                continue;
            }

            let session_id = scan.session_id.clone().unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string()
            });

            let first_msg = scan.first_user_msg.clone();
            let last_user = scan.last_user_msg.clone();
            let prev_assistant = scan.prev_assistant_msg.clone();
            let last_assistant = scan.last_assistant_msg.clone();
            let file_mtime = scan.file_mtime.clone().unwrap_or_default();
            let created_at = scan
                .created_at
                .clone()
                .unwrap_or_else(|| file_mtime.clone());
            let cwd = scan.cwd.clone().unwrap_or_else(|| PathBuf::from("."));

            let title = first_msg
                .as_ref()
                .map(|m| truncate_str_safe(m.lines().next().unwrap_or(m), 60))
                .unwrap_or_else(|| truncate_str_safe(&session_id, 60));

            let mut summary = String::new();
            if let Some(ref msg) = first_msg {
                summary = format!("--- First message ---\n{}", msg);
            }
            if let Some(ref msg) = last_user {
                if first_msg.as_ref() != Some(msg) {
                    summary = format!("{}\n\n--- Last user message ---\n{}", summary, msg);
                }
            }
            if let Some(ref msg) = prev_assistant {
                summary = format!("{}\n\n--- Previous response ---\n{}", summary, msg);
            }
            if let Some(ref msg) = last_assistant {
                summary = format!("{}\n\n--- Last Codex response ---\n{}", summary, msg);
            }

            sessions.push(Session {
                id: format!("codex_{}", session_id),
                provider_session_id: session_id,
                provider_name: "codex".into(),
                cwd,
                title,
                tab_title: None,
                summary,
                state: SessionState::default(),
                pid: None,
                created_at,
                updated_at: file_mtime,
                state_dir: Some(path),
            });
        }

        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(sessions)
    }

    fn match_processes(&self, sessions: &mut [Session]) -> Result<()> {
        let processes = process_info::discover_processes("codex");
        let mut live: Vec<(u32, Option<String>)> = Vec::new();

        for (pid, entry) in &processes {
            let cmd_lower = entry.command_line.to_lowercase();
            let is_codex = entry.name.to_lowercase().contains("codex")
                || cmd_lower.contains(" codex")
                || cmd_lower.contains("\\codex");
            if !is_codex {
                continue;
            }

            let matched_session = sessions
                .iter()
                .find(|s| entry.command_line.contains(&s.provider_session_id))
                .map(|s| s.provider_session_id.clone());

            live.push((*pid, matched_session));
        }

        let matched_by_id: Vec<_> = live.iter().filter(|(_, sid)| sid.is_some()).collect();
        let mut claimed_pids: HashSet<u32> = HashSet::new();

        for session in sessions.iter_mut() {
            let jsonl_path = session.state_dir.clone().unwrap_or_default();
            let scan = self.scan_session_file(&jsonl_path);

            if let Some(ref mtime) = scan.file_mtime {
                session.updated_at = mtime.clone();
            }
            if let Some(ref created) = scan.created_at {
                session.created_at = created.clone();
            }
            if let Some(ref cwd) = scan.cwd {
                session.cwd = cwd.clone();
            }

            let matched = matched_by_id.iter().find(|(_, sid)| {
                sid.as_ref()
                    .map(|s| s == &session.provider_session_id)
                    .unwrap_or(false)
            });

            let _recently_active = session
                .updated_at
                .parse::<chrono::DateTime<chrono::FixedOffset>>()
                .ok()
                .map(|dt| {
                    chrono::Utc::now()
                        .signed_duration_since(dt.with_timezone(&chrono::Utc))
                        .num_seconds()
                        < 15
                })
                .unwrap_or(false);

            // Only treat the session as running if we can match a process
            // by explicit session id in the command line. Avoid heuristic
            // attachment of unrelated codex processes which caused false
            // "running" states.
            let process_alive = if let Some((pid, _)) = matched {
                session.pid = Some(*pid);
                claimed_pids.insert(*pid);
                true
            } else {
                session.pid = None;
                false
            };

            let signals = StateSignals {
                process_alive: Some(process_alive),
                pid: session.pid,
                lock_file_exists: None,
                lock_file_pid: None,
                last_event_age_secs: scan.last_event_age_secs(),
                has_unfinished_turn: Some(scan.active_task),
                recent_tool_activity: None,
                cpu_active: None,
            };

            session.state = self.infer_state(&signals);
            session.state.persistence = PersistenceState::Resumable;

            if process_alive && scan.active_task {
                session.state.interaction = InteractionState::Busy;
                session.state.confidence = Confidence::High;
            } else if process_alive && scan.last_role.as_deref() == Some("assistant") {
                session.state.interaction = InteractionState::WaitingInput;
                session.state.confidence = Confidence::High;
            } else if !process_alive && scan.last_role.as_deref() == Some("assistant") {
                session.state.interaction = InteractionState::WaitingInput;
                session.state.confidence = Confidence::Medium;
            }

            session.state.reason = format!(
                "process={} active_task={} last_role={:?} last_activity={:?} age={:?}s",
                process_alive,
                scan.active_task,
                scan.last_role,
                scan.last_activity_kind,
                scan.last_event_age_secs()
            );
        }

        Ok(())
    }

    fn session_detail(&self, session: &Session) -> Result<SessionDetail> {
        let path = session
            .state_dir
            .as_ref()
            .context("Codex session is missing JSONL path")?;
        let scan = self.scan_session_file(path);

        Ok(SessionDetail {
            title: scan
                .first_user_msg
                .as_ref()
                .map(|m| truncate_str_safe(m.lines().next().unwrap_or(m), 60)),
            summary: if session.summary.is_empty() {
                scan.first_user_msg
            } else {
                Some(session.summary.clone())
            },
            plan_items: vec![],
        })
    }

    fn activity_sources(&self, session: &Session) -> Result<Vec<ActivitySource>> {
        let mut sources = Vec::new();
        if let Some(ref path) = session.state_dir {
            if path.exists() {
                sources.push(ActivitySource::EventStream(path.clone()));
            }
        }
        Ok(sources)
    }

    fn tab_title(&self, session: &Session) -> Option<String> {
        // Codex CLI sets tab title to the CWD folder name
        session.cwd.file_name().map(|n| n.to_string_lossy().to_string())
    }
}
