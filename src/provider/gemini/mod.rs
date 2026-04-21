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
                // Track user/gemini as interaction events.
                // Also: "Request cancelled" info resets to waiting (cancels assistant work).
                if event_type == "user" || event_type == "gemini" {
                    last_type = event_type.to_string();
                } else if event_type == "info" {
                    if let Some(content) = val.get("content").and_then(|v| v.as_str()) {
                        if content.contains("cancelled") || content.contains("canceled") {
                            last_type = "gemini".to_string(); // treat as assistant done → waiting
                        }
                    }
                }

                if event_type == "user"
                    && first_user_msg.is_none() {
                        if let Some(content) = Self::extract_content_text(&val) {
                            first_user_msg = Some(truncate_str_safe(&content, 300));
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
        
        if let Some(s) = content.as_str() {
            if !s.trim().is_empty() { return Some(s.trim().to_string()); }
        }

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

    /// Read session UUID from the first line of a JSONL file (the header).
    fn read_session_id(path: &Path) -> Option<String> {
        let file = std::io::BufReader::new(std::fs::File::open(path).ok()?);
        use std::io::BufRead;
        let first_line = file.lines().next()?.ok()?;
        let val: serde_json::Value = serde_json::from_str(&first_line).ok()?;
        val.get("sessionId").and_then(|v| v.as_str()).map(|s| s.to_string())
    }

    /// Find the most recently modified JSONL for a session (checks subdirectory too).
    fn find_latest_jsonl(chats_dir: &Path, session_id: &str) -> Option<PathBuf> {
        let mut candidates: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();

        // Top-level session-*.jsonl files that contain this session ID
        if let Ok(entries) = std::fs::read_dir(chats_dir) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.ends_with(".jsonl") && name.contains(crate::util::short_id(session_id, 8)) {
                    if let Ok(meta) = e.metadata() {
                        if let Ok(mtime) = meta.modified() {
                            candidates.push((e.path(), mtime));
                        }
                    }
                }
            }
        }

        // Subdirectory: <session-id>/*.jsonl
        let sub_dir = chats_dir.join(session_id);
        if sub_dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&sub_dir) {
                for e in entries.flatten() {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.ends_with(".jsonl") {
                        if let Ok(meta) = e.metadata() {
                            if let Ok(mtime) = meta.modified() {
                                candidates.push((e.path(), mtime));
                            }
                        }
                    }
                }
            }
        }

        candidates.into_iter().max_by_key(|(_, t)| *t).map(|(p, _)| p)
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

            // Group sessions by UUID (read from JSONL header)
            // Each session may have: session-<ts>-<short>.jsonl + <uuid>/continuation.jsonl
            let mut session_map: HashMap<String, Vec<PathBuf>> = HashMap::new();

            if let Ok(dir_entries) = std::fs::read_dir(&chats_dir) {
                for file_entry in dir_entries.flatten() {
                    let fname = file_entry.file_name().to_string_lossy().to_string();
                    if fname.ends_with(".jsonl") && fname.starts_with("session-") {
                        // Read session UUID from first line
                        if let Some(uuid) = Self::read_session_id(&file_entry.path()) {
                            session_map.entry(uuid).or_default().push(file_entry.path());
                        }
                    }
                    // Also check subdirectories (continuation files)
                    if file_entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                        let sub_id = fname.clone();
                        if let Ok(sub_entries) = std::fs::read_dir(file_entry.path()) {
                            for sub_file in sub_entries.flatten() {
                                let sub_name = sub_file.file_name().to_string_lossy().to_string();
                                if sub_name.ends_with(".jsonl") {
                                    session_map.entry(sub_id.clone()).or_default().push(sub_file.path());
                                }
                            }
                        }
                    }
                }
            }

            for (session_id, jsonl_files) in &session_map {
                // Find the most recently modified JSONL for this session
                let best_file = jsonl_files.iter()
                    .filter_map(|p| std::fs::metadata(p).ok().and_then(|m| m.modified().ok()).map(|t| (p, t)))
                    .max_by_key(|(_, t)| *t)
                    .map(|(p, _)| p.clone());

                let Some(jsonl_path) = best_file else { continue };

                let scan = self.scan_jsonl(&jsonl_path);
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
                    .unwrap_or_else(|| crate::util::short_id(session_id, 8).to_string());

                // Set interaction state from JSONL scan
                let interaction = if scan.waiting_for_user {
                    InteractionState::WaitingInput
                } else if scan.assistant_working {
                    InteractionState::Busy
                } else {
                    InteractionState::Unknown
                };

                sessions.push(Session {
                    id: format!("gemini_{}_{}", proj_name, session_id),
                    provider_session_id: session_id.clone(),
                    provider_name: "gemini".into(),
                    cwd: cwd.clone(),
                    title,
                    tab_title: None,
                    summary,
                    state: SessionState {
                        interaction,
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
        let processes = process_info::discover_processes("gemini");
        
        let mut live: Vec<(u32, Option<String>)> = Vec::new();
        for (pid, entry) in &processes {
            let cmd_lower = entry.command_line.to_lowercase();
            if cmd_lower.contains("gemini") && !cmd_lower.contains("agent-session-tui") {
                let id = process_info::extract_flag_value(&entry.command_line, "--resume")
                    .or_else(|| process_info::extract_flag_value(&entry.command_line, "--session-id"));
                live.push((*pid, id));
            }
        }

        let mut claimed_pids = HashSet::new();

        for session in sessions.iter_mut() {
            // 1. Try matching by session ID in command line
            let matched = live.iter().find(|(_, sid)| {
                sid.as_ref().map(|s| s == &session.provider_session_id).unwrap_or(false)
            });

            let process_alive = if let Some((pid, _)) = matched {
                session.pid = Some(*pid);
                claimed_pids.insert(*pid);
                true
            } else {
                // 2. Fallback: match unclaimed process if session was recently active
                let updated_dt = session.updated_at.parse::<chrono::DateTime<chrono::FixedOffset>>().ok();
                let recently_active = updated_dt.map(|dt| {
                    let age = chrono::Utc::now().signed_duration_since(dt.with_timezone(&chrono::Utc)).num_seconds().abs();
                    age < 120
                }).unwrap_or(false);

                if recently_active {
                    if let Some((pid, _)) = live.iter().find(|(p, _)| !claimed_pids.contains(p)) {
                        session.pid = Some(*pid);
                        claimed_pids.insert(*pid);
                        true
                    } else { false }
                } else { false }
            };

            // Re-scan the most recent JSONL for fresh state
            let scan = session.state_dir.as_ref()
                .and_then(|d| Self::find_latest_jsonl(d, &session.provider_session_id))
                .map(|p| self.scan_jsonl(&p))
                .unwrap_or_default();

            if let Some(ref mtime) = scan.file_mtime {
                session.updated_at = mtime.clone();
            }

            // Determine interaction state from scan
            let interaction = if scan.waiting_for_user {
                InteractionState::WaitingInput
            } else if scan.assistant_working {
                InteractionState::Busy
            } else {
                session.state.interaction
            };

            if process_alive {
                session.state = SessionState {
                    process: ProcessState::Running,
                    interaction,
                    persistence: PersistenceState::Resumable,
                    health: HealthState::Clean,
                    confidence: Confidence::High,
                    reason: format!("process alive, waiting_for_user={}", scan.waiting_for_user),
                };
            } else {
                session.state = SessionState {
                    process: ProcessState::Exited,
                    interaction,
                    persistence: PersistenceState::Resumable,
                    health: HealthState::Clean,
                    confidence: Confidence::High,
                    reason: format!("no process, waiting_for_user={}", scan.waiting_for_user),
                };
            }
        }

        Ok(())
    }

    fn session_detail(&self, session: &Session) -> Result<SessionDetail> {
        let scan = session.state_dir.as_ref()
            .and_then(|d| Self::find_latest_jsonl(d, &session.provider_session_id))
            .map(|p| self.scan_jsonl(&p))
            .unwrap_or_default();

        Ok(SessionDetail {
            title: Some(session.title.clone()),
            summary: scan.first_user_msg,
            plan_items: vec![],
        })
    }

    fn activity_sources(&self, session: &Session) -> Result<Vec<ActivitySource>> {
        let mut sources = Vec::new();
        if let Some(ref dir) = session.state_dir {
            if let Some(path) = Self::find_latest_jsonl(dir, &session.provider_session_id) {
                sources.push(ActivitySource::EventStream(path));
            }
        }
        Ok(sources)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn write_jsonl(dir: &std::path::Path, name: &str, lines: &[&str]) -> PathBuf {
        fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        let mut f = fs::File::create(&path).unwrap();
        for line in lines {
            writeln!(f, "{}", line).unwrap();
        }
        path
    }

    fn make_provider() -> GeminiProvider {
        GeminiProvider {
            config: crate::config::ProviderConfig {
                enabled: true,
                default: false,
                command: "gemini".into(),
                default_args: vec![],
                state_dir: None,
                resume_flag: None,
                startup_dir: None,
                launch_method: "cmd".into(),
                launch_cmd: None,
                launch_args: None,
                launch_fallback_cmd: None,
                launch_fallback_args: None,
                launch_fallback: None,
                wt_profile: None,
            },
            state_dir: PathBuf::from("."),
        }
    }

    #[test]
    fn scan_detects_waiting_for_user() {
        let dir = std::env::temp_dir().join("gemini-test-waiting");
        let _ = fs::remove_dir_all(&dir);
        let path = write_jsonl(&dir, "session.jsonl", &[
            r#"{"sessionId":"test-1","startTime":"2026-01-01T00:00:00Z","kind":"main"}"#,
            r#"{"id":"1","timestamp":"2026-01-01T00:01:00Z","type":"user","content":[{"text":"hello"}]}"#,
            r#"{"id":"2","timestamp":"2026-01-01T00:02:00Z","type":"gemini","content":"Hi there!"}"#,
        ]);
        let provider = make_provider();
        let scan = provider.scan_jsonl(&path);
        assert!(scan.waiting_for_user, "Last event is gemini → should be waiting for user");
        assert!(!scan.assistant_working);
        assert!(scan.has_user);
        assert_eq!(scan.first_user_msg.as_deref(), Some("hello"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_detects_assistant_working() {
        let dir = std::env::temp_dir().join("gemini-test-busy");
        let _ = fs::remove_dir_all(&dir);
        let path = write_jsonl(&dir, "session.jsonl", &[
            r#"{"sessionId":"test-2","startTime":"2026-01-01T00:00:00Z","kind":"main"}"#,
            r#"{"id":"1","timestamp":"2026-01-01T00:01:00Z","type":"user","content":[{"text":"do something"}]}"#,
        ]);
        let provider = make_provider();
        let scan = provider.scan_jsonl(&path);
        assert!(scan.assistant_working, "Last event is user → assistant should be working");
        assert!(!scan.waiting_for_user);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_ignores_info_events_for_state() {
        let dir = std::env::temp_dir().join("gemini-test-info");
        let _ = fs::remove_dir_all(&dir);
        let path = write_jsonl(&dir, "session.jsonl", &[
            r#"{"sessionId":"test-3","startTime":"2026-01-01T00:00:00Z","kind":"main"}"#,
            r#"{"id":"1","timestamp":"2026-01-01T00:01:00Z","type":"user","content":[{"text":"hi"}]}"#,
            r#"{"id":"2","timestamp":"2026-01-01T00:02:00Z","type":"gemini","content":"done"}"#,
            r#"{"id":"3","timestamp":"2026-01-01T00:03:00Z","type":"info","content":"Request cancelled."}"#,
            r#"{"$set":{"lastUpdated":"2026-01-01T00:03:00Z"}}"#,
        ]);
        let provider = make_provider();
        let scan = provider.scan_jsonl(&path);
        assert!(scan.waiting_for_user, "Info/metadata after gemini → still waiting for user");
        assert!(!scan.assistant_working);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_session_id_from_header() {
        let dir = std::env::temp_dir().join("gemini-test-header");
        let _ = fs::remove_dir_all(&dir);
        let path = write_jsonl(&dir, "session.jsonl", &[
            r#"{"sessionId":"abc-123-def","startTime":"2026-01-01T00:00:00Z","kind":"main"}"#,
        ]);
        let id = GeminiProvider::read_session_id(&path);
        assert_eq!(id.as_deref(), Some("abc-123-def"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_cancelled_request_means_waiting() {
        let dir = std::env::temp_dir().join("gemini-test-cancel");
        let _ = fs::remove_dir_all(&dir);
        let path = write_jsonl(&dir, "session.jsonl", &[
            r#"{"sessionId":"test-4","startTime":"2026-01-01T00:00:00Z","kind":"main"}"#,
            r#"{"id":"1","timestamp":"2026-01-01T00:01:00Z","type":"user","content":[{"text":"do it"}]}"#,
            r#"{"id":"2","timestamp":"2026-01-01T00:02:00Z","type":"info","content":"Request cancelled."}"#,
            r#"{"$set":{"lastUpdated":"2026-01-01T00:02:00Z"}}"#,
        ]);
        let provider = make_provider();
        let scan = provider.scan_jsonl(&path);
        assert!(scan.waiting_for_user, "Cancelled request after user → back to waiting");
        assert!(!scan.assistant_working);
        let _ = fs::remove_dir_all(&dir);
    }
}
