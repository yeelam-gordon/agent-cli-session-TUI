#![allow(dead_code)]

use anyhow::Result;

use crate::models::{ActivitySource, ProviderCapabilities, Session, SessionState, StateSignals};

// ---------------------------------------------------------------------------
// Provider trait — DATA LAYER ONLY
//
// A provider teaches the system how to DISCOVER and INTERPRET sessions
// from a specific agent CLI's state directory.
//
// It does NOT handle launching, resuming, or killing — those are
// config-driven operations owned by the supervisor/framework.
// ---------------------------------------------------------------------------

/// Each agent CLI implements this trait as a data provider plugin.
///
/// Required methods (plugin must implement):
///   - `key()`, `name()`, `capabilities()` — identity
///   - `discover_sessions()` — scan CLI state dir for sessions
///   - `match_processes()` — match live OS processes to sessions
///
/// Optional methods (have default implementations):
///   - `session_detail()` — extra detail for the detail panel
///   - `activity_sources()` — log/event files for the log viewer
///   - `infer_state()` — override default state inference logic
///   - `tab_title()` — extract terminal tab title from session logs
pub trait Provider: Send + Sync {
    // ── Identity (required) ──────────────────────────────────────────

    /// Human-readable name (e.g., "Copilot CLI").
    fn name(&self) -> &str;

    /// Short key matching the config section (e.g., "copilot").
    fn key(&self) -> &str;

    /// What this provider supports.
    fn capabilities(&self) -> ProviderCapabilities;

    // ── Discovery (required) ─────────────────────────────────────────

    /// Scan the CLI's state directory and return all discoverable sessions.
    ///
    /// Each Session should have at minimum:
    ///   - provider_session_id (the CLI's own session identifier)
    ///   - provider_name (must match key())
    ///   - title (short, for the session list)
    ///   - summary (longer, for the detail panel — include first/last messages)
    ///   - cwd, created_at, updated_at
    ///
    /// Sessions with no user interaction should be filtered out here.
    fn discover_sessions(&self) -> Result<Vec<Session>>;

    /// Paginated discovery — return sessions sorted by recency (most recent first).
    ///
    /// `offset` is the number of sessions to skip, `limit` is the max to return.
    /// Returns a `PagedSessions` with the sessions and whether more remain.
    ///
    /// The default implementation calls `discover_sessions()` and slices.
    /// Providers with many sessions (e.g., Copilot with 500+) should override
    /// this with an optimized implementation that:
    ///   1. Lists candidate dirs/files cheaply (stat mtime, no content parsing)
    ///   2. Sorts by recency (e.g., events.jsonl mtime)
    ///   3. Only parses the requested slice
    ///
    /// The supervisor calls this in phases: page 1 from all providers (blocking),
    /// then remaining pages in background. The viewmodel merges incrementally.
    fn discover_sessions_paged(&self, offset: usize, limit: usize) -> Result<PagedSessions> {
        let all = self.discover_sessions()?;
        let total = all.len();
        let sessions: Vec<Session> = all.into_iter().skip(offset).take(limit).collect();
        let has_more = offset + sessions.len() < total;
        Ok(PagedSessions {
            sessions,
            total,
            has_more,
        })
    }

    /// Match live OS processes to discovered sessions.
    ///
    /// Called after `discover_sessions()`. Receives the sessions and should:
    ///   1. Find running processes belonging to this CLI
    ///   2. Match them to sessions (by session ID, lock file, or heuristics)
    ///   3. Set session.pid, session.state, session.updated_at accordingly
    ///
    /// Use `crate::process_info::discover_processes()` for process detection.
    fn match_processes(&self, sessions: &mut [Session]) -> Result<()>;

    // ── Detail (optional) ────────────────────────────────────────────

    /// Return extra detail for the detail panel (plan items, checkpoints, etc.).
    /// Default: returns the session's existing summary.
    fn session_detail(&self, session: &Session) -> Result<SessionDetail> {
        Ok(SessionDetail {
            title: Some(session.title.clone()),
            summary: Some(session.summary.clone()),
            plan_items: vec![],
        })
    }

    /// Return paths to log/event files for the log viewer.
    /// Default: empty (no log sources).
    fn activity_sources(&self, _session: &Session) -> Result<Vec<ActivitySource>> {
        Ok(vec![])
    }

    /// Override state inference. Default uses the multi-signal inference engine.
    fn infer_state(&self, signals: &StateSignals) -> SessionState {
        default_state_inference(signals)
    }

    /// Extract the current terminal tab title for a session from its logs.
    ///
    /// Many agent CLIs dynamically set the terminal tab title (via ANSI OSC
    /// escape sequences) to reflect their current activity — for example,
    /// Copilot CLI emits `report_intent` tool calls whose `intent` argument
    /// becomes the tab title.
    ///
    /// Return the **latest** such title so the TUI can focus the correct
    /// Windows Terminal tab when the user presses Enter on a running session.
    ///
    /// Default: `None` (provider does not support tab-title extraction).
    fn tab_title(&self, _session: &Session) -> Option<String> {
        None
    }
}

// ---------------------------------------------------------------------------
// PagedSessions — result from paginated discovery
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PagedSessions {
    /// Sessions for this page, sorted by recency (most recent first).
    pub sessions: Vec<Session>,
    /// Total known candidates (may be approximate — some get filtered during parsing).
    pub total: usize,
    /// Whether more sessions remain beyond this page.
    pub has_more: bool,
}

// ---------------------------------------------------------------------------
// SessionDetail — extra info for the detail panel (optional from provider)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct SessionDetail {
    pub title: Option<String>,
    pub summary: Option<String>,
    pub plan_items: Vec<PlanItem>,
}

#[derive(Debug, Clone)]
pub struct PlanItem {
    pub title: String,
    pub done: bool,
}

// ---------------------------------------------------------------------------
// Default state inference from signals (multi-signal)
// ---------------------------------------------------------------------------

fn default_state_inference(s: &StateSignals) -> SessionState {
    use crate::models::*;

    let process = match (s.process_alive, s.lock_file_exists, s.lock_file_pid) {
        (Some(true), _, _) => ProcessState::Running,
        (Some(false), Some(true), Some(_pid)) => ProcessState::StaleLock,
        (Some(false), _, _) => ProcessState::Exited,
        (None, Some(true), Some(_pid)) => ProcessState::StaleLock,
        _ => ProcessState::Missing,
    };

    let interaction = if process == ProcessState::Running {
        match (
            s.has_unfinished_turn,
            s.recent_tool_activity,
            s.last_event_age_secs,
        ) {
            (Some(true), Some(false), Some(age)) if age > 30 => InteractionState::WaitingInput,
            (Some(false), _, Some(age)) if age > 60 => InteractionState::Idle,
            (_, Some(true), _) => InteractionState::Busy,
            (Some(true), _, _) => InteractionState::Busy,
            _ => InteractionState::Unknown,
        }
    } else {
        InteractionState::Unknown
    };

    let persistence = match process {
        ProcessState::Running => PersistenceState::Resumable,
        _ => {
            if s.lock_file_exists == Some(true) || s.process_alive == Some(false) {
                PersistenceState::Resumable
            } else {
                PersistenceState::Ephemeral
            }
        }
    };

    let health = match process {
        ProcessState::StaleLock => HealthState::Orphaned,
        ProcessState::Exited => {
            if s.last_event_age_secs.is_some_and(|a| a < 5) {
                HealthState::Crashed
            } else {
                HealthState::Clean
            }
        }
        _ => HealthState::Clean,
    };

    let confidence = match (
        s.process_alive,
        s.has_unfinished_turn,
        s.last_event_age_secs,
    ) {
        (Some(_), Some(_), Some(_)) => Confidence::High,
        (Some(_), _, Some(_)) => Confidence::Medium,
        _ => Confidence::Low,
    };

    let reason = format!(
        "process={:?} lock={:?} last_event_age={:?}s unfinished_turn={:?}",
        s.process_alive, s.lock_file_exists, s.last_event_age_secs, s.has_unfinished_turn
    );

    SessionState {
        process,
        interaction,
        persistence,
        health,
        confidence,
        reason,
    }
}

// ---------------------------------------------------------------------------
// Provider registry
// ---------------------------------------------------------------------------

pub struct ProviderRegistry {
    providers: Vec<Box<dyn Provider>>,
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub fn register(&mut self, provider: Box<dyn Provider>) {
        self.providers.push(provider);
    }

    pub fn providers(&self) -> &[Box<dyn Provider>] {
        &self.providers
    }
}

// Re-export submodules
pub mod claude;
pub mod codex;
pub mod copilot;
pub mod qwen;
pub mod gemini;
