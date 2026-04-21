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
        match (self.process, self.interaction) {
            (ProcessState::Running, InteractionState::WaitingInput) => "🟡",
            (ProcessState::Running, _) => "🟢",
            _ => match self.persistence {
                PersistenceState::Resumable => "💤",
                PersistenceState::Archived => "📦",
                PersistenceState::Ephemeral => "⚪",
            },
        }
    }

    /// Short label for display.
    pub fn label(&self) -> &'static str {
        match (self.process, self.interaction) {
            (ProcessState::Running, InteractionState::WaitingInput) => "Waiting",
            (ProcessState::Running, InteractionState::Busy) => "Running",
            (ProcessState::Running, InteractionState::Idle) => "Idle",
            (ProcessState::Running, InteractionState::Unknown) => "Running",
            _ => match self.persistence {
                PersistenceState::Resumable => "Resumable",
                PersistenceState::Archived => "Archived",
                PersistenceState::Ephemeral => "Stopped",
            },
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
    /// Current terminal tab title (extracted from CLI logs, e.g. report_intent).
    /// Used for tab-focus matching; `None` when the provider doesn't support it.
    pub tab_title: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state() {
        let state = SessionState::default();
        assert_eq!(state.process, ProcessState::Missing);
        assert_eq!(state.interaction, InteractionState::Unknown);
        assert_eq!(state.persistence, PersistenceState::Ephemeral);
        assert_eq!(state.health, HealthState::Clean);
        assert_eq!(state.confidence, Confidence::Low);
    }

    #[test]
    fn badge_running_busy() {
        let state = SessionState {
            process: ProcessState::Running,
            interaction: InteractionState::Busy,
            ..SessionState::default()
        };
        assert_eq!(state.badge(), "🟢");
        assert_eq!(state.label(), "Running");
    }

    #[test]
    fn badge_running_waiting() {
        let state = SessionState {
            process: ProcessState::Running,
            interaction: InteractionState::WaitingInput,
            ..SessionState::default()
        };
        assert_eq!(state.badge(), "🟡");
        assert_eq!(state.label(), "Waiting");
    }

    #[test]
    fn badge_resumable() {
        let state = SessionState {
            process: ProcessState::Exited,
            persistence: PersistenceState::Resumable,
            ..SessionState::default()
        };
        assert_eq!(state.badge(), "💤");
        assert_eq!(state.label(), "Resumable");
    }

    #[test]
    fn badge_orphaned_shows_as_resumable() {
        // Orphaned sessions are resumable — no separate display state
        let state = SessionState {
            health: HealthState::Orphaned,
            persistence: PersistenceState::Resumable,
            ..SessionState::default()
        };
        assert_eq!(state.badge(), "💤");
        assert_eq!(state.label(), "Resumable");
    }

    #[test]
    fn badge_crashed_shows_as_resumable() {
        // Crashed sessions are resumable — no separate display state
        let state = SessionState {
            health: HealthState::Crashed,
            persistence: PersistenceState::Resumable,
            ..SessionState::default()
        };
        assert_eq!(state.badge(), "💤");
        assert_eq!(state.label(), "Resumable");
    }

    #[test]
    fn confidence_ordering() {
        assert!(Confidence::Low < Confidence::Medium);
        assert!(Confidence::Medium < Confidence::High);
    }
}
