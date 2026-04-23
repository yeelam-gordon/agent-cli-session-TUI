#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use tokio::sync::mpsc;

use crate::archive::ArchiveStore;
use crate::config::ProviderConfig;
use crate::models::Session;
use crate::provider::ProviderRegistry;

/// Messages from the supervisor to the TUI.
#[derive(Debug)]
pub enum SupervisorEvent {
    SessionsUpdated {
        /// Provider whose scan just completed. `None` if the event is not tied
        /// to a single provider (reserved for future broadcast-style updates).
        provider_key: Option<String>,
        active: Vec<Session>,
        hidden: Vec<Session>,
    },
    /// Fired synchronously after `handle_archive` persists the archive record
    /// to disk. The UI uses this as the authoritative signal to drain its
    /// `pending_archives` list. Clearing pending_archives based on scan
    /// results is UNSAFE: a scan that started before a later archive was
    /// persisted will still place that session in `active`, and without a
    /// pending_archives entry to filter it, the row silently "un-archives"
    /// on screen.
    ArchiveConfirmed {
        provider_key: String,
        provider_session_id: String,
    },
    /// Fired synchronously after `handle_unarchive` removes an archive record.
    /// The UI uses this as the authoritative signal to drain its
    /// `pending_unarchives` list (same race guard as archive).
    UnarchiveConfirmed {
        provider_key: String,
        provider_session_id: String,
    },
    Error(String),
}

/// Commands from the TUI to the supervisor.
#[derive(Debug)]
pub enum SupervisorCommand {
    NewSession { provider_key: String, cwd: String },
    ResumeSession {
        provider_session_id: String,
        provider_key: String,
        session_cwd: String,
    },
    KillSession {
        provider_session_id: String,
        provider_key: String,
    },
    ArchiveSession {
        provider_session_id: String,
        provider_key: String,
    },
    UnarchiveSession {
        provider_session_id: String,
        provider_key: String,
    },
    FocusSession {
        tab_title: Option<String>,
        title: String,
        provider_session_id: String,
    },
    Shutdown,
}

// ---------------------------------------------------------------------------
// Session ViewModel — merges provider results incrementally
// ---------------------------------------------------------------------------

/// Tracks all known sessions, keyed by session ID.
/// When a provider returns new results, only that provider's sessions are diffed:
/// - New sessions are added
/// - Missing sessions (from that provider) are removed
/// - Existing sessions are updated in-place (state, tab_title, etc.)
///
/// This allows progressive updates without flicker — each provider's results
/// merge independently regardless of arrival order.
struct SessionViewModel {
    /// All known sessions indexed by their unique `id` field.
    sessions: HashMap<String, Session>,
}

impl SessionViewModel {
    fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    /// Merge results from a single provider. Returns `true` if anything changed.
    fn merge_provider(&mut self, provider_name: &str, incoming: Vec<Session>) -> bool {
        let mut changed = false;

        // Build set of incoming IDs for this provider
        let incoming_ids: std::collections::HashSet<String> =
            incoming.iter().map(|s| s.id.clone()).collect();

        // Remove sessions from this provider that are no longer present
        let before_len = self.sessions.len();
        self.sessions
            .retain(|_, s| s.provider_name != provider_name || incoming_ids.contains(&s.id));
        if self.sessions.len() != before_len {
            changed = true;
        }

        // Upsert incoming sessions
        for s in incoming {
            match self.sessions.entry(s.id.clone()) {
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(s);
                    changed = true;
                }
                std::collections::hash_map::Entry::Occupied(mut e) => {
                    let existing = e.get_mut();
                    // Update mutable fields; keep stable identity
                    if existing.state != s.state
                        || existing.tab_title != s.tab_title
                        || existing.updated_at != s.updated_at
                        || existing.pid != s.pid
                        || existing.title != s.title
                        || existing.summary != s.summary
                    {
                        *existing = s;
                        changed = true;
                    }
                }
            }
        }

        changed
    }

    /// Produce sorted active/hidden lists from current state.
    ///
    /// Takes a cheap `HashSet<String>` snapshot of the archived keys rather
    /// than a `MutexGuard`, so scans don't hold the archive mutex for the
    /// duration of discovery. Previously, holding the mutex during scans
    /// caused rapid 'a' presses to serialise behind the scan (each command
    /// waited 5–12s for the lock), and queued archives could be lost on
    /// quit before they drained.
    fn snapshot(
        &self,
        archived_keys: &HashSet<String>,
    ) -> (Vec<Session>, Vec<Session>) {
        let mut active = Vec::new();
        let mut hidden = Vec::new();
        for s in self.sessions.values() {
            let key = format!("{}:{}", s.provider_name, s.provider_session_id);
            let is_archived = archived_keys.contains(&key);
            if is_archived {
                hidden.push(s.clone());
            } else {
                active.push(s.clone());
            }
        }
        active.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        hidden.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        (active, hidden)
    }
}

/// Background supervisor that owns process lifecycle and state monitoring.
pub struct Supervisor {
    registry: Arc<ProviderRegistry>,
    archive: Arc<Mutex<ArchiveStore>>,
    poll_interval: Duration,
    provider_configs: std::collections::HashMap<String, ProviderConfig>,
}

impl Supervisor {
    pub fn new(
        registry: Arc<ProviderRegistry>,
        archive: Arc<Mutex<ArchiveStore>>,
        poll_interval_ms: u64,
        provider_configs: std::collections::HashMap<String, ProviderConfig>,
    ) -> Self {
        Self {
            registry,
            archive,
            poll_interval: Duration::from_millis(poll_interval_ms),
            provider_configs,
        }
    }

    /// Run the supervisor loop. Returns channels for communication.
    pub async fn run(
        self,
        event_tx: mpsc::UnboundedSender<SupervisorEvent>,
        mut cmd_rx: mpsc::UnboundedReceiver<SupervisorCommand>,
    ) {
        // Shared viewmodel — persists across scan cycles for incremental diffing
        let vm = Arc::new(Mutex::new(SessionViewModel::new()));

        // Initial scan (two-phase: paged first for fast first paint, then full)
        if let Err(e) = self.scan_and_notify_initial(&vm, &event_tx) {
            crate::log::error(&format!("Initial scan failed: {}", e));
            let _ = event_tx.send(SupervisorEvent::Error(e.to_string()));
        }

        // Tick frequently so we can react to commands quickly, but gate actual scans
        // on a `next_scan_at` Instant that is updated AFTER a scan completes.
        // This ensures `poll_interval` is the minimum gap between the END of one
        // scan and the START of the next — preventing back-to-back scans when the
        // scan itself takes longer than poll_interval.
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
        let scanning = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let next_scan_at = Arc::new(std::sync::Mutex::new(std::time::Instant::now()));
        let poll_interval = self.poll_interval;

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    // Skip if a scan is already running in background
                    if scanning.load(std::sync::atomic::Ordering::Relaxed) {
                        continue;
                    }
                    // Skip if we haven't reached the next scheduled scan time
                    if let Ok(next) = next_scan_at.lock() {
                        if std::time::Instant::now() < *next {
                            continue;
                        }
                    }
                    // Run scan in background thread so commands aren't blocked
                    let registry = self.registry.clone();
                    let archive = self.archive.clone();
                    let vm_clone = vm.clone();
                    let tx = event_tx.clone();
                    let scan_flag = scanning.clone();
                    let next_at = next_scan_at.clone();
                    scan_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    std::thread::spawn(move || {
                        let scan_start = std::time::Instant::now();
                        if let Err(e) = Self::scan_providers(&registry, &archive, &vm_clone, &tx) {
                            crate::log::warn(&format!("Scan error: {}", e));
                            let _ = tx.send(SupervisorEvent::Error(e.to_string()));
                        }
                        let elapsed = scan_start.elapsed();
                        // Schedule next scan poll_interval AFTER this one finishes
                        if let Ok(mut next) = next_at.lock() {
                            *next = std::time::Instant::now() + poll_interval;
                        }
                        crate::log::info(&format!(
                            "Scan cycle: {:?}, next scan in {:?}",
                            elapsed, poll_interval
                        ));
                        scan_flag.store(false, std::sync::atomic::Ordering::Relaxed);
                    });
                }
                Some(cmd) = cmd_rx.recv() => {
                    let cmd_start = std::time::Instant::now();
                    match cmd {
                        SupervisorCommand::Shutdown => {
                            // Final sync write of any buffered archive
                            // mutations before the process exits. The
                            // persist worker runs on a 150 ms coalesce
                            // window, so without this an 'a' pressed
                            // immediately before quit would be lost.
                            match self.archive.lock() {
                                Ok(a) => {
                                    if let Err(e) = a.flush_blocking() {
                                        crate::log::warn(&format!(
                                            "Shutdown archive flush FAILED: {}",
                                            e
                                        ));
                                    }
                                }
                                Err(poisoned) => {
                                    // Best-effort: still attempt to flush.
                                    let a = poisoned.into_inner();
                                    if let Err(e) = a.flush_blocking() {
                                        crate::log::warn(&format!(
                                            "Shutdown archive flush (poisoned) FAILED: {}",
                                            e
                                        ));
                                    }
                                }
                            }
                            break;
                        }
                        SupervisorCommand::NewSession { provider_key, cwd } => {
                            self.handle_new_session(&provider_key, &cwd, &event_tx);
                        }
                        SupervisorCommand::ResumeSession { provider_session_id, provider_key, session_cwd } => {
                            self.handle_resume(&provider_key, &provider_session_id, &session_cwd, &event_tx);
                        }
                        SupervisorCommand::KillSession { provider_session_id, provider_key } => {
                            self.handle_kill(&provider_key, &provider_session_id, &event_tx);
                        }
                        SupervisorCommand::ArchiveSession { provider_session_id, provider_key } => {
                            // Persist only. The UI already moves the session to
                            // hidden instantly via `pending_archives`, and the
                            // next periodic scan reconciles. Previously this
                            // spawned a full re-scan per keypress — rapid 'a'
                            // presses stampeded with N parallel scans across
                            // 500+ sessions and pegged CPU at 100%.
                            self.handle_archive(&provider_key, &provider_session_id, &event_tx);
                        }
                        SupervisorCommand::UnarchiveSession { provider_session_id, provider_key } => {
                            // Persist only. Same reasoning as ArchiveSession:
                            // the UI handles the visual via `pending_unarchives`
                            // and the next periodic scan reconciles.
                            self.handle_unarchive(&provider_key, &provider_session_id, &event_tx);
                        }
                        SupervisorCommand::FocusSession { tab_title, title, provider_session_id } => {
                            crate::log::info(&format!("FocusSession cmd received after {:?}", cmd_start.elapsed()));
                            Self::handle_focus(tab_title.as_deref(), &title, &provider_session_id, &event_tx);
                        }
                    }
                    crate::log::info(&format!("Command processed in {:?}", cmd_start.elapsed()));
                }
            }
        }
    }

    fn scan_and_notify(&self, vm: &Arc<Mutex<SessionViewModel>>, event_tx: &mpsc::UnboundedSender<SupervisorEvent>) -> Result<()> {
        Self::scan_providers(&self.registry, &self.archive, vm, event_tx)
    }

    /// Two-phase initial scan:
    ///   Phase 1 — each provider returns only the top-N most recent sessions via
    ///             `discover_sessions_paged`. Cheap (stat-only + parse N only).
    ///             All providers run in parallel; each emits a SessionsUpdated
    ///             event as it completes. The UI gate opens once all have
    ///             reported in.
    ///   Phase 2 — each provider runs full `discover_sessions()` in parallel.
    ///             Results replace the phase-1 slice per-provider as they arrive.
    ///
    /// This gives the user a first visible list in ~1-2s (instead of waiting
    /// 10-30s for every provider's full tail-read pass to complete) while
    /// still delivering the full dataset soon after.
    fn scan_and_notify_initial(&self, vm: &Arc<Mutex<SessionViewModel>>, event_tx: &mpsc::UnboundedSender<SupervisorEvent>) -> Result<()> {
        Self::scan_providers_two_phase(&self.registry, &self.archive, vm, event_tx)
    }

    fn scan_providers_two_phase(
        registry: &Arc<ProviderRegistry>,
        archive: &Arc<Mutex<ArchiveStore>>,
        vm: &Arc<Mutex<SessionViewModel>>,
        event_tx: &mpsc::UnboundedSender<SupervisorEvent>,
    ) -> Result<()> {
        /// Number of most-recent sessions to fetch per provider in phase 1.
        /// Sized to fill one viewport comfortably; full dataset follows in phase 2.
        const FIRST_PAGE: usize = 20;

        let providers: Vec<_> = registry.providers().iter()
            .filter(|p| p.capabilities().supports_discovery)
            .collect();

        // ── Phase 1: paged, blocking — fast first paint ─────────────────────
        let phase1_start = std::time::Instant::now();
        std::thread::scope(|s| {
            let (tx, rx) = std::sync::mpsc::channel::<(String, Vec<Session>)>();
            for provider in &providers {
                let tx = tx.clone();
                s.spawn(move || {
                    let pstart = std::time::Instant::now();
                    let paged = match provider.discover_sessions_paged(0, FIRST_PAGE) {
                        Ok(p) => p,
                        Err(e) => {
                            crate::log::warn(&format!("Phase 1 '{}' error: {}", provider.key(), e));
                            crate::provider::PagedSessions { sessions: vec![], total: 0, has_more: false }
                        }
                    };
                    let mut sessions = paged.sessions;
                    let _ = provider.match_processes(&mut sessions);
                    for session in &mut sessions {
                        if session.state.process == crate::models::ProcessState::Running {
                            session.tab_title = provider.tab_title(session);
                        }
                    }
                    crate::log::info(&format!(
                        "Phase 1 '{}': {} of {} sessions in {:?}",
                        provider.key(), sessions.len(), paged.total, pstart.elapsed()
                    ));
                    let _ = tx.send((provider.key().to_string(), sessions));
                });
            }
            drop(tx);

            let archive_guard = archive
                .lock()
                .map(|a| a.snapshot_keys())
                .unwrap_or_default();
            for (provider_key, sessions) in rx {
                if let Ok(mut vm_lock) = vm.lock() {
                    vm_lock.merge_provider(&provider_key, sessions);
                    let (active, hidden) = vm_lock.snapshot(&archive_guard);
                    let _ = event_tx.send(SupervisorEvent::SessionsUpdated {
                        provider_key: Some(provider_key),
                        active,
                        hidden,
                    });
                }
            }
        });
        crate::log::info(&format!("Phase 1 complete in {:?}", phase1_start.elapsed()));

        // ── Phase 2: full discovery, background — complete dataset ──────────
        let phase2_start = std::time::Instant::now();
        std::thread::scope(|s| {
            let (tx, rx) = std::sync::mpsc::channel::<(String, Vec<Session>)>();
            for provider in &providers {
                let tx = tx.clone();
                s.spawn(move || {
                    let pstart = std::time::Instant::now();
                    let mut sessions = provider.discover_sessions().unwrap_or_default();
                    let _ = provider.match_processes(&mut sessions);
                    for session in &mut sessions {
                        if session.state.process == crate::models::ProcessState::Running {
                            session.tab_title = provider.tab_title(session);
                        }
                    }
                    crate::log::info(&format!(
                        "Phase 2 '{}': {} sessions in {:?}",
                        provider.key(), sessions.len(), pstart.elapsed()
                    ));
                    let _ = tx.send((provider.key().to_string(), sessions));
                });
            }
            drop(tx);

            let archive_guard = archive
                .lock()
                .map(|a| a.snapshot_keys())
                .unwrap_or_default();
            for (provider_key, sessions) in rx {
                if let Ok(mut vm_lock) = vm.lock() {
                    vm_lock.merge_provider(&provider_key, sessions);
                    let (active, hidden) = vm_lock.snapshot(&archive_guard);
                    let _ = event_tx.send(SupervisorEvent::SessionsUpdated {
                        provider_key: Some(provider_key),
                        active,
                        hidden,
                    });
                }
            }
        });
        crate::log::info(&format!("Phase 2 complete in {:?}", phase2_start.elapsed()));
        Ok(())
    }

    /// Static scan function — can run on any thread without borrowing &self.
    /// Uses SessionViewModel to merge results incrementally per-provider,
    /// sending progressive updates that never flicker.
    fn scan_providers(
        registry: &Arc<ProviderRegistry>,
        archive: &Arc<Mutex<ArchiveStore>>,
        vm: &Arc<Mutex<SessionViewModel>>,
        event_tx: &mpsc::UnboundedSender<SupervisorEvent>,
    ) -> Result<()> {
        let providers: Vec<_> = registry.providers().iter()
            .filter(|p| p.capabilities().supports_discovery)
            .collect();

        let archive_guard = archive
            .lock()
            .map(|a| a.snapshot_keys())
            .unwrap_or_default();

        // Scan providers in parallel; merge each provider's results as they arrive
        std::thread::scope(|s| {
            // Channel carries (provider_key, sessions)
            let (tx, rx) = std::sync::mpsc::channel::<(String, Vec<Session>)>();

            for provider in &providers {
                let tx = tx.clone();
                s.spawn(move || {
                    let pstart = std::time::Instant::now();
                    let mut sessions = provider.discover_sessions().unwrap_or_default();
                    let _ = provider.match_processes(&mut sessions);
                    for session in &mut sessions {
                        if session.state.process == crate::models::ProcessState::Running {
                            let tt_start = std::time::Instant::now();
                            session.tab_title = provider.tab_title(session);
                            crate::log::info(&format!(
                                "tab_title({}, {}) = {:?} in {:?}",
                                provider.key(),
                                crate::util::short_id(&session.provider_session_id, 8),
                                session.tab_title.as_deref().unwrap_or("None"),
                                tt_start.elapsed()
                            ));
                        }
                    }
                    crate::log::info(&format!(
                        "Provider '{}' scan: {} sessions in {:?}",
                        provider.key(), sessions.len(), pstart.elapsed()
                    ));
                    let _ = tx.send((provider.key().to_string(), sessions));
                });
            }
            drop(tx);

            // Merge each provider's results into the viewmodel as they arrive.
            // We always emit an event per provider — even if nothing changed —
            // so the UI can reliably detect "all providers have reported in".
            for (provider_key, sessions) in rx {
                if let Ok(mut vm_lock) = vm.lock() {
                    vm_lock.merge_provider(&provider_key, sessions);
                    let (active, hidden) = vm_lock.snapshot(&archive_guard);
                    let _ = event_tx.send(SupervisorEvent::SessionsUpdated {
                        provider_key: Some(provider_key),
                        active,
                        hidden,
                    });
                }
            }
        });

        Ok(())
    }

    /// Build command args from config. Framework-owned, not provider-specific.
    fn build_new_command(config: &ProviderConfig) -> Vec<String> {
        let mut cmd = vec![config.command.clone()];
        cmd.extend(config.default_args.iter().cloned());
        cmd
    }

    /// Build resume command from config. Framework-owned, not provider-specific.
    fn build_resume_command(config: &ProviderConfig, session_id: &str) -> Vec<String> {
        let mut cmd = vec![config.command.clone()];
        cmd.extend(config.default_args.iter().cloned());
        if let Some(ref flag) = config.resume_flag {
            cmd.push(flag.clone());
            cmd.push(session_id.to_string());
        }
        cmd
    }

    fn handle_new_session(
        &self,
        provider_key: &str,
        cwd: &str,
        event_tx: &mpsc::UnboundedSender<SupervisorEvent>,
    ) {
        let Some(config) = self.provider_configs.get(provider_key) else {
            let _ = event_tx.send(SupervisorEvent::Error(format!(
                "Provider '{}' not in config",
                provider_key
            )));
            return;
        };

        let effective_cwd = config
            .startup_dir
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| cwd.to_string());

        let cmd = Self::build_new_command(config);
        crate::log::info(&format!(
            "Launching new {}: {:?} in {}",
            provider_key, cmd, effective_cwd
        ));
        if let Err(e) = launch_in_terminal(&cmd, &effective_cwd, config) {
            crate::log::error(&format!("Failed to launch {}: {}", provider_key, e));
            let _ = event_tx.send(SupervisorEvent::Error(format!("Failed to launch: {}", e)));
        }
    }

    fn handle_resume(
        &self,
        provider_key: &str,
        provider_session_id: &str,
        session_cwd: &str,
        event_tx: &mpsc::UnboundedSender<SupervisorEvent>,
    ) {
        let Some(config) = self.provider_configs.get(provider_key) else {
            let _ = event_tx.send(SupervisorEvent::Error(format!(
                "Provider '{}' not in config",
                provider_key
            )));
            return;
        };

        // Use the session's original CWD (critical for CLIs like Claude that
        // tie sessions to directories). Fall back to config startup_dir, then ".".
        let effective_cwd = if !session_cwd.is_empty() && session_cwd != "." {
            session_cwd.to_string()
        } else {
            config
                .startup_dir
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| ".".to_string())
        };

        let cmd = Self::build_resume_command(config, provider_session_id);
        crate::log::info(&format!(
            "Resuming {} session {} in {:?}: {:?}",
            provider_key, provider_session_id, effective_cwd, cmd
        ));
        if let Err(e) = launch_in_terminal(&cmd, &effective_cwd, config) {
            crate::log::error(&format!("Failed to resume: {}", e));
            let _ = event_tx.send(SupervisorEvent::Error(format!("Failed to resume: {}", e)));
        }
    }

    fn handle_kill(
        &self,
        _provider_key: &str,
        _provider_session_id: &str,
        event_tx: &mpsc::UnboundedSender<SupervisorEvent>,
    ) {
        let _ = event_tx.send(SupervisorEvent::Error(
            "Kill not yet implemented".to_string(),
        ));
    }

    /// Try to focus an existing Windows Terminal tab by matching the title.
    fn handle_focus(
        tab_title: Option<&str>,
        title: &str,
        session_id: &str,
        event_tx: &mpsc::UnboundedSender<SupervisorEvent>,
    ) {
        // Search priority: tab_title (from CLI logs) → session title → short session ID
        let mut search_terms: Vec<String> = Vec::new();
        if let Some(tt) = tab_title {
            search_terms.push(tt.to_string());
        }
        search_terms.push(title.to_string());
        search_terms.push(crate::util::short_id(session_id, 8).to_string());

        for term in &search_terms {
            if crate::focus::focus_wt_tab(term) {
                crate::log::info(&format!("Focused tab matching: {}", term));
                return;
            }
        }

        let display = tab_title.unwrap_or(title);
        crate::log::warn(&format!("Could not find tab for: {} / {}", display, session_id));
        let _ = event_tx.send(SupervisorEvent::Error(
            format!("Tab not found for '{}'", display),
        ));
    }

    fn handle_archive(
        &self,
        provider_key: &str,
        provider_session_id: &str,
        event_tx: &mpsc::UnboundedSender<SupervisorEvent>,
    ) {
        let t0 = std::time::Instant::now();
        match self.archive.lock() {
            Ok(mut archive) => {
                if let Err(e) = archive.archive(provider_key, provider_session_id) {
                    crate::log::warn(&format!(
                        "handle_archive: save FAILED for {}:{}: {} (held lock {:?})",
                        provider_key, provider_session_id, e, t0.elapsed()
                    ));
                }
            }
            Err(e) => {
                crate::log::warn(&format!(
                    "handle_archive: archive mutex poisoned for {}:{}: {}",
                    provider_key, provider_session_id, e
                ));
            }
        }
        // Tell the UI that persistence is done so it can safely drop the
        // key from `pending_archives`. Without this confirmation, the UI
        // would have to guess from scan results, and it can guess wrong
        // when many scans are in flight after rapid 'a' presses.
        let _ = event_tx.send(SupervisorEvent::ArchiveConfirmed {
            provider_key: provider_key.to_string(),
            provider_session_id: provider_session_id.to_string(),
        });
    }

    fn handle_unarchive(
        &self,
        provider_key: &str,
        provider_session_id: &str,
        event_tx: &mpsc::UnboundedSender<SupervisorEvent>,
    ) {
        match self.archive.lock() {
            Ok(mut archive) => {
                if let Err(e) = archive.unarchive(provider_key, provider_session_id) {
                    crate::log::warn(&format!(
                        "handle_unarchive: save FAILED for {}:{}: {}",
                        provider_key, provider_session_id, e
                    ));
                }
            }
            Err(e) => {
                crate::log::warn(&format!(
                    "handle_unarchive: archive mutex poisoned for {}:{}: {}",
                    provider_key, provider_session_id, e
                ));
            }
        }
        let _ = event_tx.send(SupervisorEvent::UnarchiveConfirmed {
            provider_key: provider_key.to_string(),
            provider_session_id: provider_session_id.to_string(),
        });
    }
}

/// Expand {cwd} and {command} placeholders in launch args.
fn expand_launch_args(args: &[String], cwd: &str, command: &str) -> Vec<String> {
    args.iter()
        .map(|a| a.replace("{cwd}", cwd).replace("{command}", command))
        .collect()
}

/// Try to launch with a program + args. Returns Ok if spawned, Err if program not found.
fn try_launch(program: &str, args: &[String]) -> Result<()> {
    std::process::Command::new(program)
        .args(args)
        .spawn()?;
    Ok(())
}

/// Launch a command in a new terminal. Tries custom launch_cmd/args first,
/// then launch_method shortcut, then fallback chain.
fn launch_in_terminal(
    cmd: &[String],
    cwd: &str,
    config: &crate::config::ProviderConfig,
) -> Result<()> {
    let cmd_str = cmd.join(" ");

    // 1. Custom launch_cmd + launch_args (fully user-defined)
    if let Some(ref launch_cmd) = config.launch_cmd {
        if let Some(ref launch_args) = config.launch_args {
            let args = expand_launch_args(launch_args, cwd, &cmd_str);
            match try_launch(launch_cmd, &args) {
                Ok(_) => return Ok(()),
                Err(e) => {
                    crate::log::warn(&format!("{} failed: {}, trying fallback", launch_cmd, e));
                }
            }
        }
    }

    // 2. Custom fallback_cmd + fallback_args
    if let (Some(ref fb_cmd), Some(ref fb_args)) = (&config.launch_fallback_cmd, &config.launch_fallback_args) {
        let args = expand_launch_args(fb_args, cwd, &cmd_str);
        match try_launch(fb_cmd, &args) {
            Ok(_) => return Ok(()),
            Err(e) => {
                crate::log::warn(&format!("Fallback {} failed: {}, trying shortcut", fb_cmd, e));
            }
        }
    }

    // 3. Shortcut-based launch (launch_method → launch_fallback)
    let method = if config.launch_cmd.is_some() {
        // Custom cmd already failed, skip to fallback shortcut
        config.launch_fallback.as_deref().unwrap_or("cmd")
    } else {
        config.launch_method.as_str()
    };
    let fallback_method = if config.launch_cmd.is_some() {
        None // already tried custom, don't loop
    } else {
        config.launch_fallback.as_deref()
    };

    launch_with_shortcut(&cmd_str, cwd, method, fallback_method, config.wt_profile.as_deref())
}

/// Launch using shortcut method names: "wt", "pwsh", "cmd".
fn launch_with_shortcut(
    cmd_str: &str,
    cwd: &str,
    method: &str,
    fallback: Option<&str>,
    wt_profile: Option<&str>,
) -> Result<()> {
    #[cfg(windows)]
    {
        match method {
            // wt-compatible launchers: use -w 0 new-tab style args
            m @ ("wt" | "wtai") => {
                let mut args = vec!["-w".to_string(), "0".to_string(), "new-tab".to_string()];
                if let Some(profile) = wt_profile {
                    args.push("--profile".to_string());
                    args.push(profile.to_string());
                }
                args.push("--startingDirectory".to_string());
                args.push(cwd.to_string());
                args.push("cmd".to_string());
                args.push("/k".to_string());
                args.push(cmd_str.to_string());

                match std::process::Command::new(m).args(&args).spawn() {
                    Ok(_) => Ok(()),
                    Err(_) => {
                        let fb = fallback.unwrap_or("cmd");
                        crate::log::warn(&format!("{} not found, falling back to {}", m, fb));
                        launch_with_shortcut(cmd_str, cwd, fb, None, None)
                    }
                }
            }
            "pwsh" => {
                match std::process::Command::new("pwsh")
                    .args(["-NoExit", "-Command", cmd_str])
                    .current_dir(cwd)
                    .spawn()
                {
                    Ok(_) => Ok(()),
                    Err(_) if fallback.is_some() => {
                        let fb = fallback.expect("checked is_some above");
                        crate::log::warn(&format!("pwsh not found, falling back to {}", fb));
                        launch_with_shortcut(cmd_str, cwd, fb, None, None)
                    }
                    Err(e) => Err(e.into()),
                }
            }
            _ => {
                std::process::Command::new("cmd")
                    .args(["/c", "start", "cmd", "/k", cmd_str])
                    .current_dir(cwd)
                    .spawn()?;
                Ok(())
            }
        }
    }

    #[cfg(not(windows))]
    {
        let _ = (method, fallback, wt_profile);
        let shell_cmd = format!("cd {} && {}", cwd, cmd_str);
        std::process::Command::new("sh")
            .args(["-c", &format!("xterm -e '{}' &", shell_cmd)])
            .spawn()?;
        Ok(())
    }
}
