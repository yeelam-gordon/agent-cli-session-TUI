use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sysinfo::System;

use crate::config::ProviderConfig;
use crate::models::*;
use crate::provider::{
    ActivitySource, PlanItem, Provider, ProviderCapabilities, SessionDetail,
};
use crate::util::truncate_str_safe;

/// GitHub Copilot CLI provider.
///
/// Reads session state from:
///   1. `~/.copilot/session-store.db` (SQLite — sessions + checkpoints tables)
///   2. `~/.copilot/session-state/<id>/` (workspace.yaml, events.jsonl, plan.md)
///   3. `inuse.<pid>.lock` files for liveness detection
pub struct CopilotProvider {
    config: ProviderConfig,
    state_dir: PathBuf,
    store_db_path: PathBuf,
}

impl CopilotProvider {
    pub fn new(config: &ProviderConfig) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let copilot_dir = home.join(".copilot");
        let state_dir = config
            .state_dir
            .clone()
            .unwrap_or_else(|| copilot_dir.join("session-state"));
        let store_db_path = copilot_dir.join("session-store.db");

        Self {
            config: config.clone(),
            state_dir,
            store_db_path,
        }
    }

    /// Try to read summary from the session-store.db SQLite database.
    fn read_store_db_session(&self, session_id: &str) -> Option<StoredSession> {
        let db = rusqlite::Connection::open_with_flags(
            &self.store_db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .ok()?;

        let mut stmt = db
            .prepare(
                "SELECT id, summary, cwd, repository, branch, created_at, updated_at \
                 FROM sessions WHERE id = ?1",
            )
            .ok()?;

        stmt.query_row(rusqlite::params![session_id], |row| {
            Ok(StoredSession {
                id: row.get(0)?,
                summary: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                cwd: row.get::<_, Option<String>>(2)?,
                repository: row.get::<_, Option<String>>(3)?,
                branch: row.get::<_, Option<String>>(4)?,
                created_at: row.get::<_, Option<String>>(5)?,
                updated_at: row.get::<_, Option<String>>(6)?,
            })
        })
        .ok()
    }

    /// Read the latest checkpoint summary for a session.
    fn read_latest_checkpoint(&self, session_id: &str) -> Option<String> {
        let db = rusqlite::Connection::open_with_flags(
            &self.store_db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .ok()?;

        let mut stmt = db
            .prepare(
                "SELECT title, overview, work_done FROM checkpoints \
                 WHERE session_id = ?1 \
                 ORDER BY checkpoint_number DESC LIMIT 1",
            )
            .ok()?;

        stmt.query_row(rusqlite::params![session_id], |row| {
            let title: Option<String> = row.get(0)?;
            let overview: Option<String> = row.get(1)?;
            let work_done: Option<String> = row.get(2)?;
            // Compose a rich checkpoint summary from all available fields
            let mut parts = Vec::new();
            if let Some(t) = title.filter(|s| !s.is_empty()) {
                parts.push(t);
            }
            if let Some(o) = overview.filter(|s| !s.is_empty()) {
                parts.push(o);
            }
            if let Some(w) = work_done.filter(|s| !s.is_empty()) {
                parts.push(format!("Work done: {}", w));
            }
            Ok(if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            })
        })
        .ok()
        .flatten()
    }

    /// Read the first user message from the session to use as a fallback summary.
    fn read_first_user_message(&self, session_id: &str) -> Option<String> {
        let db = rusqlite::Connection::open_with_flags(
            &self.store_db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .ok()?;

        let mut stmt = db
            .prepare(
                "SELECT user_message FROM turns \
                 WHERE session_id = ?1 \
                 ORDER BY turn_index ASC LIMIT 1",
            )
            .ok()?;

        stmt.query_row(rusqlite::params![session_id], |row| {
            row.get::<_, Option<String>>(0)
        })
        .ok()
        .flatten()
        .map(|msg| {
            // Truncate long messages to first meaningful chunk (char-safe)
            let trimmed = msg.trim();
            truncate_str_safe(trimmed, 200)
        })
    }

    /// Check for `inuse.<pid>.lock` files in the session dir.
    /// Returns ALL lock files found, with live PIDs first.
    fn find_lock_files(&self, session_dir: &Path) -> Vec<(PathBuf, u32, bool)> {
        let entries = match std::fs::read_dir(session_dir) {
            Ok(e) => e,
            Err(_) => return Vec::new(),
        };

        // Collect all lock file candidates first
        let mut candidates: Vec<(PathBuf, u32)> = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("inuse.") && name_str.ends_with(".lock") {
                if let Some(pid_str) = name_str
                    .strip_prefix("inuse.")
                    .and_then(|s| s.strip_suffix(".lock"))
                {
                    if let Ok(pid) = pid_str.parse::<u32>() {
                        candidates.push((entry.path(), pid));
                    }
                }
            }
        }

        if candidates.is_empty() {
            return Vec::new();
        }

        // Batch-check all PIDs with a single System refresh
        let pids_to_check: Vec<sysinfo::Pid> = candidates
            .iter()
            .map(|(_, pid)| sysinfo::Pid::from_u32(*pid))
            .collect();
        let mut sys = System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&pids_to_check), true);

        let mut results: Vec<(PathBuf, u32, bool)> = candidates
            .into_iter()
            .map(|(path, pid)| {
                let alive = sys.process(sysinfo::Pid::from_u32(pid)).is_some();
                (path, pid, alive)
            })
            .collect();

        // Sort: alive PIDs first
        results.sort_by_key(|(_path, _pid, alive)| !alive);
        results
    }

    /// Read CWD from workspace.yaml in the session state dir.
    fn read_workspace_cwd(&self, session_dir: &Path) -> Option<PathBuf> {
        let ws_path = session_dir.join("workspace.yaml");
        let text = std::fs::read_to_string(ws_path).ok()?;
        // Simple line-based parsing — avoid pulling in a YAML crate
        for line in text.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("cwd:") {
                let cwd = rest.trim().trim_matches('"').trim_matches('\'');
                if !cwd.is_empty() {
                    return Some(PathBuf::from(cwd));
                }
            }
        }
        None
    }

    /// Read plan.md from session state dir.
    fn read_plan_md(&self, session_dir: &Path) -> Option<Vec<PlanItem>> {
        let plan_path = session_dir.join("plan.md");
        let text = std::fs::read_to_string(plan_path).ok()?;
        let mut items = Vec::new();
        for line in text.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("- [x]").or_else(|| trimmed.strip_prefix("- [X]")) {
                items.push(PlanItem {
                    title: rest.trim().to_string(),
                    done: true,
                });
            } else if let Some(rest) = trimmed.strip_prefix("- [ ]") {
                items.push(PlanItem {
                    title: rest.trim().to_string(),
                    done: false,
                });
            }
            // Also handle ### headers as plan phase markers
        }
        if items.is_empty() {
            None
        } else {
            Some(items)
        }
    }

    /// Get the last modification time of the most recently changed file in session dir.
    fn last_activity_time(&self, session_dir: &Path) -> Option<String> {
        let mut latest: Option<std::time::SystemTime> = None;
        if let Ok(entries) = std::fs::read_dir(session_dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if let Ok(modified) = meta.modified() {
                        latest = Some(match latest {
                            Some(prev) if modified > prev => modified,
                            Some(prev) => prev,
                            None => modified,
                        });
                    }
                }
            }
        }
        latest.map(|t| {
            let dt: chrono::DateTime<chrono::Local> = t.into();
            dt.to_rfc3339()
        })
    }

    /// Analyze events.jsonl for session state signals.
    fn check_events_jsonl(&self, session_dir: &Path) -> EventsState {
        let events_path = session_dir.join("events.jsonl");

        // File modification time = real "last activity" (not the DB timestamp)
        let file_mtime = std::fs::metadata(&events_path)
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|t| {
                let dt: chrono::DateTime<chrono::Local> = t.into();
                dt.to_rfc3339()
            });

        let text = match std::fs::read_to_string(&events_path) {
            Ok(t) => t,
            Err(_) => return EventsState { file_mtime, ..Default::default() },
        };

        // Forward scan through the tail (last ~100 lines) for correct ordering
        let lines: Vec<&str> = text.lines().collect();
        let scan_start = lines.len().saturating_sub(100);

        let mut last_timestamp: Option<chrono::DateTime<chrono::Utc>> = None;
        let mut last_meaningful_event: Option<String> = None;
        // Track the last turn boundary state
        let mut assistant_working = false;  // true between turn_start and turn_end
        let mut user_responded = false;     // true if user.message after last turn_end

        for line in &lines[scan_start..] {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                if let Some(event_type) = val.get("type").and_then(|v| v.as_str()) {
                    if let Some(ts) = val.get("timestamp").and_then(|v| v.as_str()) {
                        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
                            last_timestamp = Some(dt.with_timezone(&chrono::Utc));
                        }
                    }

                    match event_type {
                        "assistant.turn_start" => {
                            assistant_working = true;
                            user_responded = false;
                            last_meaningful_event = Some(event_type.to_string());
                        }
                        "assistant.turn_end" | "assistant.turn_complete" | "session.task_complete" => {
                            assistant_working = false;
                            user_responded = false;
                            last_meaningful_event = Some(event_type.to_string());
                        }
                        "user.message" => {
                            user_responded = true;
                            last_meaningful_event = Some(event_type.to_string());
                        }
                        "tool.execution_start" | "tool.execution_complete" | "assistant.message" => {
                            last_meaningful_event = Some(event_type.to_string());
                        }
                        _ => {}
                    }
                }
            }
        }

        let age_secs = last_timestamp.map(|ts| {
            let now = chrono::Utc::now();
            now.signed_duration_since(ts).num_seconds().max(0) as u64
        });

        // State interpretation:
        // assistant_working=true → assistant is actively processing (Busy)
        // assistant_working=false && !user_responded → ball is in user's court (WaitingInput)
        // assistant_working=false && user_responded → shouldn't happen at steady state
        let waiting_for_user = !assistant_working && !user_responded
            && last_meaningful_event.as_deref() != Some("session.start");

        EventsState {
            last_event_type: last_meaningful_event,
            has_unfinished_turn: assistant_working,
            waiting_for_user,
            last_event_age_secs: age_secs,
            file_mtime,
        }
    }

    /// Extract first user message and last assistant message from events.jsonl.
    /// Returns (first_user_msg, last_assistant_msg, has_user_messages, total_event_count).
    fn read_user_messages_from_events(
        &self,
        session_dir: &Path,
    ) -> (Option<String>, Option<String>, bool, usize) {
        let events_path = session_dir.join("events.jsonl");
        let text = match std::fs::read_to_string(&events_path) {
            Ok(t) => t,
            Err(_) => return (None, None, false, 0),
        };

        let lines: Vec<&str> = text.lines().collect();
        let event_count = lines.len();
        let mut first_user_msg: Option<String> = None;
        let mut last_assistant_msg: Option<String> = None;

        for line in &lines {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                let event_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");

                if event_type == "user.message" || event_type == "human.message" {
                    if let Some(msg) = val
                        .get("data")
                        .and_then(|d| {
                            d.get("content")
                                .or(d.get("message"))
                                .or(d.get("text"))
                        })
                        .and_then(|v| v.as_str())
                    {
                        let trimmed = msg.trim();
                        if !trimmed.is_empty() && first_user_msg.is_none() {
                            first_user_msg = Some(truncate_str_safe(trimmed, 300));
                        }
                    }
                }

                // Capture last assistant message — this is what tells you
                // the current state of work in the session
                if event_type == "assistant.message" {
                    if let Some(msg) = val
                        .get("data")
                        .and_then(|d| d.get("content"))
                        .and_then(|v| v.as_str())
                    {
                        let trimmed = msg.trim();
                        if !trimmed.is_empty() {
                            last_assistant_msg = Some(truncate_str_safe(trimmed, 500));
                        }
                    }
                }
            }
        }

        let has_user = first_user_msg.is_some();
        (first_user_msg, last_assistant_msg, has_user, event_count)
    }
}

struct StoredSession {
    id: String,
    summary: String,
    cwd: Option<String>,
    repository: Option<String>,
    branch: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

#[derive(Debug, Default)]
struct EventsState {
    last_event_type: Option<String>,
    has_unfinished_turn: bool,
    /// Assistant finished its turn, no user.message followed → ball in user's court
    waiting_for_user: bool,
    last_event_age_secs: Option<u64>,
    /// File modification time of events.jsonl — the real "last activity" timestamp
    file_mtime: Option<String>,
}

impl Provider for CopilotProvider {
    fn name(&self) -> &str {
        "Copilot CLI"
    }

    fn key(&self) -> &str {
        "copilot"
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
        let mut sessions = Vec::new();
        let entries = std::fs::read_dir(&self.state_dir)
            .with_context(|| format!("Cannot read state dir: {:?}", self.state_dir))?;

        for entry in entries.flatten() {
            if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                continue;
            }
            let dir_name = entry.file_name().to_string_lossy().to_string();

            // Read from session-store.db first (best source of truth)
            let stored = self.read_store_db_session(&dir_name);
            let checkpoint_summary = self.read_latest_checkpoint(&dir_name);
            let first_message_db = self.read_first_user_message(&dir_name);
            let plan_items = self.read_plan_md(&entry.path());
            let last_activity = self.last_activity_time(&entry.path());

            // Extract from events.jsonl (first user msg, last assistant msg, has user msgs, event count)
            let (first_msg_events, last_assistant_events, has_user_events, _event_count) =
                self.read_user_messages_from_events(&entry.path());

            // Pick the best available first message
            let first_message = first_message_db.or(first_msg_events);
            let last_assistant = last_assistant_events;

            // Skip sessions with zero user interaction
            let has_db_content = stored.as_ref().map_or(false, |s| !s.summary.is_empty());
            let has_turns = first_message.is_some();
            let has_checkpoint = checkpoint_summary.is_some();
            let has_plan = plan_items.is_some();

            if !has_db_content && !has_turns && !has_checkpoint && !has_plan && !has_user_events {
                continue;
            }

            // Build summary with ordered fallbacks + first/last user messages
            let db_summary = stored
                .as_ref()
                .map(|s| s.summary.clone())
                .filter(|s| !s.is_empty());

            let mut summary = if let Some(ref s) = db_summary {
                let mut enriched = s.clone();
                if let Some(ref cp) = checkpoint_summary {
                    if !cp.is_empty() && *cp != *s {
                        enriched = format!("{}\n\n--- Latest checkpoint ---\n{}", enriched, cp);
                    }
                }
                enriched
            } else if let Some(ref cp) = checkpoint_summary {
                cp.clone()
            } else if let Some(ref msg) = first_message {
                format!("User asked: {}", msg)
            } else if let Some(ref items) = plan_items {
                let done = items.iter().filter(|i| i.done).count();
                let total = items.len();
                let current = items.iter().find(|i| !i.done).map(|i| i.title.as_str()).unwrap_or("(all done)");
                format!("Plan: {}/{} done. Current: {}", done, total, current)
            } else {
                String::new()
            };

            // Always append first user message + last assistant response for context
            if let Some(ref msg) = first_message {
                summary = format!("{}\n\n--- First message ---\n{}", summary, msg);
            }
            if let Some(ref msg) = last_assistant {
                summary = format!("{}\n\n--- Last Copilot response ---\n{}", summary, msg);
            }

            // Build a meaningful title
            let repo_context = stored.as_ref()
                .and_then(|s| s.repository.as_ref())
                .map(|r| r.rsplit('/').next().unwrap_or(r).to_string());

            let title = if let Some(ref s) = db_summary {
                s.lines().next().unwrap_or("").to_string()
            } else if let Some(ref cp) = checkpoint_summary {
                cp.lines().next().unwrap_or("").to_string()
            } else if let Some(ref msg) = first_message {
                let short = msg.lines().next().unwrap_or(msg);
                truncate_str_safe(short, 60)
            } else if let Some(ref items) = plan_items {
                items
                    .iter()
                    .find(|i| !i.done)
                    .or(items.last())
                    .map(|i| i.title.clone())
                    .unwrap_or_else(|| dir_name[..8.min(dir_name.len())].to_string())
            } else if let Some(ref repo) = repo_context {
                format!("[{}] {}", repo, &dir_name[..8.min(dir_name.len())])
            } else {
                dir_name[..8.min(dir_name.len())].to_string()
            };

            // CWD fallback chain: session-store.db → workspace.yaml → "."
            let cwd_path = stored
                .as_ref()
                .and_then(|s| s.cwd.as_ref())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .or_else(|| self.read_workspace_cwd(&entry.path()))
                .unwrap_or_else(|| PathBuf::from("."));

            let created = stored
                .as_ref()
                .and_then(|s| s.created_at.clone())
                .unwrap_or_default();
            let updated = stored
                .as_ref()
                .and_then(|s| s.updated_at.clone())
                .or(last_activity)
                .unwrap_or_default();

            sessions.push(Session {
                id: format!("copilot_{}", dir_name),
                provider_session_id: dir_name,
                provider_name: "copilot".into(),
                cwd: cwd_path,
                title,
                summary,
                state: SessionState::default(),
                pid: None,
                created_at: created,
                updated_at: updated,
                state_dir: Some(entry.path()),
            });
        }

        // Sort by updated_at descending
        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(sessions)
    }

    fn match_processes(&self, sessions: &mut [Session]) -> Result<()> {
        // Discover live copilot processes
        let procs = crate::process_info::discover_processes("copilot");
        let mut live: Vec<(u32, Option<String>)> = Vec::new();
        for (pid, entry) in &procs {
            let is_copilot = entry.name.to_lowercase().contains("copilot")
                || entry.name.to_lowercase().contains("ghcs")
                || (entry.command_line.to_lowercase().contains("copilot")
                    && !entry.command_line.to_lowercase().contains("claude"));
            if is_copilot {
                let session_id = crate::process_info::extract_flag_value(
                    &entry.command_line, "--resume",
                );
                live.push((*pid, session_id));
            }
        }

        // Match processes to sessions
        for session in sessions.iter_mut() {
            // Check ALL lock files — prefer the one with a live PID
            let lock_files = session
                .state_dir
                .as_ref()
                .map(|dir| self.find_lock_files(dir))
                .unwrap_or_default();

            let has_any_lock = !lock_files.is_empty();
            // Find the first alive lock (sorted alive-first by find_lock_files)
            let live_lock = lock_files.iter().find(|(_, _, alive)| *alive);
            let has_stale_locks = lock_files.iter().any(|(_, _, alive)| !*alive);

            let (lock_pid, process_alive) = if let Some((_path, pid, _)) = live_lock {
                (Some(*pid), true)
            } else {
                // No live lock — check if any live process matches this session by ID
                let matched = live.iter().find(|(_, sid)| {
                    sid.as_ref()
                        .map(|s| s == &session.provider_session_id)
                        .unwrap_or(false)
                });
                if let Some((pid, _)) = matched {
                    (Some(*pid), true)
                } else {
                    // Use the stale lock PID for reporting
                    let stale_pid = lock_files.first().map(|(_, pid, _)| *pid);
                    (stale_pid, false)
                }
            };

            session.pid = if process_alive { lock_pid } else { None };

            // Collect signals for state inference
            let events = session
                .state_dir
                .as_ref()
                .map(|dir| self.check_events_jsonl(dir))
                .unwrap_or_default();

            // Use events.jsonl file mtime as updated_at (real-time, not DB timestamp)
            if let Some(ref mtime) = events.file_mtime {
                session.updated_at = mtime.clone();
            }

            let signals = StateSignals {
                process_alive: Some(process_alive),
                pid: lock_pid,
                lock_file_exists: Some(has_any_lock),
                lock_file_pid: lock_pid,
                last_event_age_secs: events.last_event_age_secs,
                has_unfinished_turn: Some(events.has_unfinished_turn),
                recent_tool_activity: None,
                cpu_active: None,
            };

            session.state = self.infer_state(&signals);

            // Override interaction state with events-based waiting detection
            if process_alive && events.waiting_for_user {
                session.state.interaction = crate::models::InteractionState::WaitingInput;
                session.state.confidence = crate::models::Confidence::High;
                session.state.reason = format!(
                    "process alive, last_event={:?}, waiting_for_user=true",
                    events.last_event_type
                );
            } else if process_alive && events.has_unfinished_turn {
                session.state.interaction = crate::models::InteractionState::Busy;
                session.state.confidence = crate::models::Confidence::High;
                session.state.reason = format!(
                    "process alive, assistant working, last_event={:?}",
                    events.last_event_type
                );
            }

            // Override: if process is alive but stale locks also exist,
            // it's still Running — not Orphaned
            if process_alive && has_stale_locks {
                session.state.health = crate::models::HealthState::Clean;
            }
        }
        Ok(())
    }

    fn session_detail(&self, session: &Session) -> Result<SessionDetail> {
        let stored = self.read_store_db_session(&session.provider_session_id);
        let checkpoint = self.read_latest_checkpoint(&session.provider_session_id);
        let plan_items = session
            .state_dir
            .as_ref()
            .and_then(|dir| self.read_plan_md(dir))
            .unwrap_or_default();

        Ok(SessionDetail {
            title: stored.as_ref().map(|s| {
                s.summary
                    .lines()
                    .next()
                    .unwrap_or(&s.id)
                    .to_string()
            }),
            summary: stored
                .as_ref()
                .map(|s| s.summary.clone())
                .filter(|s| !s.is_empty())
                .or(checkpoint),
            plan_items,
        })
    }

    fn activity_sources(&self, session: &Session) -> Result<Vec<ActivitySource>> {
        let mut sources = Vec::new();
        if let Some(ref dir) = session.state_dir {
            let events = dir.join("events.jsonl");
            if events.exists() {
                sources.push(ActivitySource::EventStream(events));
            }
        }
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let log_dir = home.join(".copilot").join("logs");
        if log_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&log_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with("process-") && name.ends_with(".log") {
                        sources.push(ActivitySource::ProcessLog(entry.path()));
                    }
                }
            }
        }
        Ok(sources)
    }
}

