#![allow(dead_code)]

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::ProviderConfig;
use crate::models::*;
use crate::process_info;
use crate::provider::{
    ActivitySource, Provider, ProviderCapabilities, SessionDetail,
};
use crate::util::truncate_str_safe;

/// Claude Code provider.
///
/// Reads session state from:
///   `~/.claude/projects/<encoded-cwd>/<session-id>.jsonl`
///
/// Each JSONL file IS a session. Event types:
///   user, assistant, system, file-history-snapshot, permission-mode, attachment
///
/// No lock files — process detection via sysinfo only.
/// No session-store.db — all state in JSONL files.
pub struct ClaudeProvider {
    config: ProviderConfig,
    projects_dir: PathBuf,
}

impl ClaudeProvider {
    pub fn new(config: &ProviderConfig) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let projects_dir = config
            .state_dir
            .clone()
            .unwrap_or_else(|| home.join(".claude").join("projects"));

        Self {
            config: config.clone(),
            projects_dir,
        }
    }

    /// Single-pass scan of a JSONL session file. Extracts everything we need:
    /// messages (for display), state signals (for inference), and timestamps.
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
        let mut last_event_type = String::new();

        for line in &lines {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                // Track timestamps
                if let Some(ts) = val.get("timestamp").and_then(|v| v.as_str()) {
                    if first_timestamp.is_none() {
                        first_timestamp = Some(ts.to_string());
                    }
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
                        last_timestamp = Some(dt.with_timezone(&chrono::Utc));
                    }
                }

                let event_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let role = val
                    .get("message")
                    .and_then(|m| m.get("role"))
                    .and_then(|r| r.as_str())
                    .unwrap_or("");

                // Track last event type and role for state inference
                if !event_type.is_empty() {
                    last_event_type = event_type.to_string();
                }
                if !role.is_empty() {
                    last_role = role.to_string();
                }

                // Extract first user message (for title/summary)
                if event_type == "user" && role == "user" && first_user_msg.is_none() {
                    if let Some(content) = Self::extract_message_content(&val) {
                        first_user_msg = Some(truncate_str_safe(&content, 300));
                    }
                }

                // Track last assistant message (for summary)
                if event_type == "assistant" && role == "assistant" {
                    if let Some(content) = Self::extract_message_content(&val) {
                        last_assistant_msg = Some(truncate_str_safe(&content, 500));
                    }
                }
            }
        }

        // Compute event age
        let last_event_age_secs = last_timestamp.map(|ts| {
            chrono::Utc::now()
                .signed_duration_since(ts)
                .num_seconds()
                .max(0) as u64
        });

        // State interpretation:
        // last_role == "assistant" → assistant finished, ball is in user's court
        // last_role == "user" → user sent a message, assistant should be working
        let waiting_for_user = last_role == "assistant";
        let assistant_working = last_role == "user";

        let has_user = first_user_msg.is_some();

        JsonlScanResult {
            first_user_msg,
            last_assistant_msg,
            has_user,
            event_count,
            first_timestamp,
            last_event_age_secs,
            waiting_for_user,
            assistant_working,
            last_event_type,
            file_mtime,
        }
    }

    /// Extract text content from a Claude message (handles string or array content).
    fn extract_message_content(val: &serde_json::Value) -> Option<String> {
        let content = val.get("message")?.get("content")?;
        if let Some(s) = content.as_str() {
            if !s.trim().is_empty() {
                return Some(s.trim().to_string());
            }
        }
        // Content can be an array of blocks: [{"type":"text","text":"..."}]
        if let Some(arr) = content.as_array() {
            let mut parts = Vec::new();
            for item in arr {
                if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                    if !text.trim().is_empty() {
                        parts.push(text.trim().to_string());
                    }
                }
            }
            if !parts.is_empty() {
                return Some(parts.join("\n"));
            }
        }
        None
    }

    /// Decode a Claude project path name back to the original directory.
    ///
    /// Claude encodes paths by replacing `:\` with `--` and `\` (or `/`) with `-`.
    /// This is **lossy**: a literal hyphen in a directory name also becomes `-`.
    /// E.g., both `C:\Users\a2a-cli` and `C:\Users\a2a\cli` encode to `C--Users-a2a-cli`.
    ///
    /// To decode unambiguously we do a greedy backtracking search:
    /// at each `-`, try interpreting it as a path separator first (longest-match),
    /// and fall back to literal hyphen if no directory exists on disk.
    fn decode_project_path(encoded: &str) -> PathBuf {
        // Split off the drive prefix: "C--rest" → drive="C:\", remainder="rest"
        let (drive, remainder) = match encoded.find("--") {
            Some(pos) => {
                let drive = format!("{}:\\", &encoded[..pos]);
                let rest = if pos + 2 < encoded.len() {
                    &encoded[pos + 2..]
                } else {
                    ""
                };
                (drive, rest)
            }
            None => {
                // No drive letter — shouldn't happen on Windows, return as-is
                return PathBuf::from(encoded);
            }
        };

        if remainder.is_empty() {
            return PathBuf::from(&drive);
        }

        // Split remainder on '-' and try to reconstruct the path
        let segments: Vec<&str> = remainder.split('-').collect();

        fn backtrack(base: &Path, segments: &[&str], idx: usize) -> Option<PathBuf> {
            if idx >= segments.len() {
                return Some(base.to_path_buf());
            }

            // Try progressively longer segment groups (greedy: prefer fewer joins)
            // e.g., for ["a2a", "cli", "nodejs"], try "a2a" first, then "a2a-cli", etc.
            let mut combined = segments[idx].to_string();
            for end in idx + 1..=segments.len() {
                let candidate = base.join(&combined);

                if end == segments.len() {
                    // Last segment(s) — this IS the leaf, doesn't need to be a directory
                    // Accept it if the directory exists, or if we've consumed everything
                    if candidate.exists() {
                        return Some(candidate);
                    }
                } else if candidate.is_dir() {
                    // This prefix exists as a directory — recurse into it
                    if let Some(result) = backtrack(&candidate, segments, end) {
                        return Some(result);
                    }
                }

                // Try joining the next segment with a hyphen (literal '-')
                if end < segments.len() {
                    combined = format!("{}-{}", combined, segments[end]);
                }
            }

            // Nothing worked with any grouping — accept the full remainder as a
            // hyphenated leaf (for paths that no longer exist on disk)
            let mut fallback = segments[idx].to_string();
            for s in &segments[idx + 1..] {
                fallback = format!("{}-{}", fallback, s);
            }
            Some(base.join(fallback))
        }

        let drive_path = PathBuf::from(&drive);
        backtrack(&drive_path, &segments, 0).unwrap_or_else(|| {
            // Ultimate fallback: naive decode
            let decoded = encoded.replacen("--", ":\\", 1).replace('-', "\\");
            PathBuf::from(decoded)
        })
    }
}

/// Result of a single-pass JSONL scan — everything we need for display and inference.
#[derive(Debug, Default)]
struct JsonlScanResult {
    // Display fields
    first_user_msg: Option<String>,
    last_assistant_msg: Option<String>,
    has_user: bool,
    event_count: usize,
    first_timestamp: Option<String>,
    // State inference fields
    last_event_age_secs: Option<u64>,
    waiting_for_user: bool,
    assistant_working: bool,
    last_event_type: String,
    file_mtime: Option<String>,
}

impl Provider for ClaudeProvider {
    fn name(&self) -> &str {
        "Claude Code"
    }

    fn key(&self) -> &str {
        "claude"
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
            if proj_name == "memory" {
                continue; // Skip Claude's memory dir
            }

            let cwd = Self::decode_project_path(&proj_name);

            // Each .jsonl file in the project dir is a session
            let jsonl_files = match std::fs::read_dir(proj_entry.path()) {
                Ok(entries) => entries,
                Err(_) => continue,
            };

            for file_entry in jsonl_files.flatten() {
                let fname = file_entry.file_name().to_string_lossy().to_string();
                if !fname.ends_with(".jsonl") {
                    continue;
                }

                let session_id = fname.trim_end_matches(".jsonl").to_string();
                let jsonl_path = file_entry.path();

                let scan = self.scan_jsonl(&jsonl_path);

                // Skip empty sessions
                if !scan.has_user || scan.event_count < 3 {
                    continue;
                }

                let file_mtime = scan.file_mtime.clone().unwrap_or_default();
                let created_at = scan.first_timestamp.clone().unwrap_or_else(|| file_mtime.clone());
                let first_msg = scan.first_user_msg.clone();
                let last_assistant = scan.last_assistant_msg.clone();

                // Build summary
                let mut summary = String::new();
                if let Some(ref msg) = first_msg {
                    summary = format!("--- First message ---\n{}", msg);
                }
                if let Some(ref msg) = last_assistant {
                    summary = format!("{}\n\n--- Last Claude response ---\n{}", summary, msg);
                }

                let title = first_msg
                    .as_ref()
                    .map(|m| {
                        let short = m.lines().next().unwrap_or(m);
                        truncate_str_safe(short, 60)
                    })
                    .unwrap_or_else(|| session_id[..8.min(session_id.len())].to_string());

                sessions.push(Session {
                    id: format!("claude_{}_{}", proj_name, session_id),
                    provider_session_id: session_id,
                    provider_name: "claude".into(),
                    cwd: cwd.clone(),
                    title,
                    summary,
                    state: SessionState::default(),
                    pid: None,
                    created_at,
                    updated_at: file_mtime,
                    state_dir: Some(proj_entry.path()),
                });
            }
        }

        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(sessions)
    }

    fn match_processes(&self, sessions: &mut [Session]) -> Result<()> {
        // Discover live claude processes via shared WMI module
        let processes = process_info::discover_processes("claude");

        let mut live: Vec<(u32, Option<String>)> = Vec::new();
        for (pid, entry) in &processes {
            let cmd_lower = entry.command_line.to_lowercase();
            let is_claude = entry.name.to_lowercase().contains("claude")
                || (cmd_lower.contains("claude") && !cmd_lower.contains("copilot"));
            if !is_claude { continue; }

            let session_id = process_info::extract_flag_value(&entry.command_line, "--session-id")
                .or_else(|| process_info::extract_flag_value(&entry.command_line, "--continue"))
                .or_else(|| process_info::extract_flag_value(&entry.command_line, "--resume"));

            crate::log::info(&format!(
                "Claude process: pid={} name={} session_id={:?} cmd={}",
                pid, entry.name, session_id, truncate_str_safe(&cmd_lower, 120)
            ));
            live.push((*pid, session_id));
        }

        crate::log::info(&format!(
            "Claude match_processes: {} sessions, {} live",
            sessions.len(), live.len()
        ));

        // Match processes to sessions
        let matched_by_id: Vec<_> = live.iter().filter(|(_, sid)| sid.is_some()).collect();
        let unknown_pids: Vec<u32> = live.iter()
            .filter(|(_, sid)| sid.is_none())
            .map(|(pid, _)| *pid).collect();
        let mut claimed_pids: std::collections::HashSet<u32> = std::collections::HashSet::new();

        for session in sessions.iter_mut() {
            // 1. Try matching by session ID from process args (best match)
            let matched = matched_by_id.iter().find(|(_, sid)| {
                sid.as_ref()
                    .map(|s| s == &session.provider_session_id)
                    .unwrap_or(false)
            });

            let process_alive = if matched.is_some() {
                session.pid = matched.map(|(pid, _)| *pid);
                true
            } else if !unknown_pids.is_empty() {
                // 2. Fallback: if there's an unclaimed claude process with unknown
                //    session ID, check if THIS session's JSONL was very recently
                //    modified (within 10s = actively being written to).
                //    Only claim one unknown PID per session, and don't reuse PIDs.
                let recently_active = session.updated_at.parse::<chrono::DateTime<chrono::FixedOffset>>()
                    .ok()
                    .map(|dt| {
                        let age = chrono::Utc::now().signed_duration_since(dt.with_timezone(&chrono::Utc));
                        age.num_seconds() < 10
                    })
                    .unwrap_or(false);

                if recently_active {
                    // Find first unclaimed unknown PID
                    if let Some(&pid) = unknown_pids.iter().find(|p| !claimed_pids.contains(p)) {
                        session.pid = Some(pid);
                        claimed_pids.insert(pid);
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            };

            // Scan JSONL for state signals (single pass)
            let jsonl_path = session
                .state_dir
                .as_ref()
                .map(|dir| dir.join(format!("{}.jsonl", session.provider_session_id)));

            let scan = jsonl_path
                .as_ref()
                .map(|p| self.scan_jsonl(p))
                .unwrap_or_default();

            if let Some(ref mtime) = scan.file_mtime {
                session.updated_at = mtime.clone();
            }

            // Build proper signals and use the shared inference engine
            let signals = StateSignals {
                process_alive: Some(process_alive),
                pid: session.pid,
                lock_file_exists: None, // Claude doesn't use lock files
                lock_file_pid: None,
                last_event_age_secs: scan.last_event_age_secs,
                has_unfinished_turn: Some(scan.assistant_working),
                recent_tool_activity: None,
                cpu_active: None,
            };

            session.state = self.infer_state(&signals);

            // Override interaction with JSONL-based detection (more accurate than
            // the generic inference for Claude's event model)
            if process_alive && scan.waiting_for_user {
                session.state.interaction = InteractionState::WaitingInput;
                session.state.confidence = Confidence::High;
            } else if process_alive && scan.assistant_working {
                session.state.interaction = InteractionState::Busy;
                session.state.confidence = Confidence::High;
            } else if !process_alive && scan.waiting_for_user {
                // Not running but last state was waiting → resumable context
                session.state.interaction = InteractionState::WaitingInput;
            }

            // All Claude JSONL sessions are resumable
            session.state.persistence = PersistenceState::Resumable;

            session.state.reason = format!(
                "process={} waiting_for_user={} working={} last_event_age={:?}s last_type={}",
                process_alive, scan.waiting_for_user, scan.assistant_working,
                scan.last_event_age_secs, scan.last_event_type
            );
        }
        Ok(())
    }

    fn session_detail(&self, session: &Session) -> Result<SessionDetail> {
        let jsonl_path = session
            .state_dir
            .as_ref()
            .map(|dir| dir.join(format!("{}.jsonl", session.provider_session_id)));

        let scan = jsonl_path
            .as_ref()
            .map(|p| self.scan_jsonl(p))
            .unwrap_or_default();

        Ok(SessionDetail {
            title: scan.first_user_msg.as_ref().map(|m| {
                truncate_str_safe(m.lines().next().unwrap_or(m), 60)
            }),
            summary: scan.first_user_msg,
            plan_items: vec![],
        })
    }

    fn activity_sources(&self, session: &Session) -> Result<Vec<ActivitySource>> {
        let mut sources = Vec::new();
        if let Some(ref dir) = session.state_dir {
            let jsonl = dir.join(format!("{}.jsonl", session.provider_session_id));
            if jsonl.exists() {
                sources.push(ActivitySource::EventStream(jsonl));
            }
        }
        Ok(sources)
    }
}

