#![allow(dead_code)] // Extension points for future providers

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Multi-axis session state (rubber-duck insight #4)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcessState {
    Running,
    Exited,
    Missing, // no process found, but session exists
    StaleLock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InteractionState {
    Busy,
    WaitingInput,
    Idle,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PersistenceState {
    Resumable,
    Ephemeral,
    Archived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthState {
    Clean,
    Crashed,
    Orphaned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Confidence {
    Low,
    Medium,
    High,
}

/// Composite session state derived from multiple signal axes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub process: ProcessState,
    pub interaction: InteractionState,
    pub persistence: PersistenceState,
    pub health: HealthState,
    pub confidence: Confidence,
    pub reason: String,
}

impl SessionState {
    /// User-facing badge for the session list.
    pub fn badge(&self) -> &'static str {
        match (self.process, self.interaction, self.health) {
            (ProcessState::Running, InteractionState::WaitingInput, _) => "🟡",
            (ProcessState::Running, InteractionState::Busy, _) => "🟢",
            (ProcessState::Running, _, _) => "🟢",
            (_, _, HealthState::Crashed) => "🔴",
            (_, _, HealthState::Orphaned) => "⚠️",
            (ProcessState::Missing, _, _) | (ProcessState::Exited, _, _) => {
                match self.persistence {
                    PersistenceState::Resumable => "💤",
                    PersistenceState::Archived => "📦",
                    PersistenceState::Ephemeral => "⚪",
                }
            }
            (ProcessState::StaleLock, _, _) => "⚠️",
        }
    }

    /// Short label for display.
    pub fn label(&self) -> &'static str {
        match (self.process, self.interaction, self.health) {
            (ProcessState::Running, InteractionState::WaitingInput, _) => "Waiting",
            (ProcessState::Running, InteractionState::Busy, _) => "Running",
            (ProcessState::Running, InteractionState::Idle, _) => "Idle",
            (ProcessState::Running, InteractionState::Unknown, _) => "Running",
            (_, _, HealthState::Crashed) => "Crashed",
            (_, _, HealthState::Orphaned) => "Orphaned",
            (ProcessState::Missing | ProcessState::Exited, _, _) => match self.persistence {
                PersistenceState::Resumable => "Resumable",
                PersistenceState::Archived => "Archived",
                PersistenceState::Ephemeral => "Stopped",
            },
            (ProcessState::StaleLock, _, _) => "Stale",
        }
    }
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            process: ProcessState::Missing,
            interaction: InteractionState::Unknown,
            persistence: PersistenceState::Ephemeral,
            health: HealthState::Clean,
            confidence: Confidence::Low,
            reason: "Initial state".into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Session model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Internal tracking ID (UUID).
    pub id: String,
    /// The agent CLI's own session identifier.
    pub provider_session_id: String,
    /// Which provider manages this session.
    pub provider_name: String,
    /// Working directory when the session was created/discovered.
    pub cwd: PathBuf,
    /// Short title derived from plan or first message.
    pub title: String,
    /// Richer summary of what the session is doing/did.
    pub summary: String,
    /// Composite state.
    pub state: SessionState,
    /// OS PID if the session process is running.
    pub pid: Option<u32>,
    /// When the session was first seen.
    pub created_at: String,
    /// Last activity timestamp (ISO-8601).
    pub updated_at: String,
    /// Path to the session's state directory (provider-specific).
    pub state_dir: Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// Provider capability flags (rubber-duck insight #10)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub supports_resume: bool,
    pub supports_discovery: bool,
    pub supports_logs: bool,
    pub supports_wait_detection: bool,
    pub supports_kill: bool,
    pub supports_archive: bool,
    pub supports_summary_extraction: bool,
}

// ---------------------------------------------------------------------------
// Activity source (rubber-duck insight #6)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum ActivitySource {
    /// Structured event stream (e.g., events.jsonl).
    EventStream(PathBuf),
    /// Process log file.
    ProcessLog(PathBuf),
    /// Generic log file.
    LogFile(PathBuf),
}

/// Signals collected by the provider for state inference.
#[derive(Debug, Clone, Default)]
pub struct StateSignals {
    pub process_alive: Option<bool>,
    pub pid: Option<u32>,
    pub lock_file_exists: Option<bool>,
    pub lock_file_pid: Option<u32>,
    pub last_event_age_secs: Option<u64>,
    pub has_unfinished_turn: Option<bool>,
    pub recent_tool_activity: Option<bool>,
    pub cpu_active: Option<bool>,
}
