use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{prelude::*, widgets::*};
use tokio::sync::mpsc;
use unicode_width::UnicodeWidthStr;

use crate::log_search::LogSearcher;
use crate::models::{InteractionState, PersistenceState, ProcessState, Session};
use crate::provider::ProviderRegistry;
use crate::groups::GroupManager;
use crate::acp::AiSuggestion;
use crate::supervisor::{SupervisorCommand, SupervisorEvent};
use crate::util::truncate_str_safe;

/// Which panel has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    SessionList,
    Detail,
    Logs,
}

/// Which view is displayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    /// Normal: active sessions only.
    Active,
    /// Grouped: sessions organized under group headers.
    Grouped,
    /// Archive: archived + empty sessions.
    Hidden,
}

/// Decide startup state given the number of enabled providers.
///
/// Returns `(no_providers, initial_status_message)`:
/// - When `provider_count == 0`, the supervisor will never emit scan events,
///   so callers must set `initial_load_complete = true` up front to avoid a
///   forever-stuck "Loading..." spinner. The status message tells the user
///   how to fix it (install a CLI or edit config.toml).
/// - Otherwise, callers show a normal "Loading N providers..." indicator.
fn empty_provider_bootstrap(provider_count: usize) -> (bool, String) {
    if provider_count == 0 {
        (
            true,
            "No providers enabled. Install a CLI (copilot/claude/codex/qwen/gemini) or edit config.toml.".into(),
        )
    } else {
        (false, format!("Loading {} providers...", provider_count))
    }
}

/// Compute the new cursor index after the row under the cursor was removed
/// from the visible list.
///
/// Returns `None` when the list is now empty (caller should clear selection),
/// otherwise `Some(index)` clamped into the new valid range.
///
/// Semantics: the cursor stays at the same *visual* position so that the row
/// which previously lived just below the removed row slides up under it. This
/// is what enables rapid repeat-archive ('a' pressed repeatedly walks down
/// the list, consuming one row per press, without the user having to
/// re-navigate after each removal). If the last row was removed, the cursor
/// clamps to the new last row.
fn clamp_cursor_after_removal(prev_index: usize, new_len: usize) -> Option<usize> {
    if new_len == 0 {
        None
    } else {
        Some(prev_index.min(new_len - 1))
    }
}

/// Tracks a locally-applied archive or unarchive that hasn't yet been
/// fully reconciled with disk scans. An entry lives through three phases:
///
///   1. **Created** (`confirmed = false`): pushed from the 'a' handler.
///      Filters matching scan entries on every `SessionsUpdated`.
///   2. **Confirmed** (`confirmed = true`): `ArchiveConfirmed` /
///      `UnarchiveConfirmed` arrived from the supervisor, meaning the
///      archive record is now on disk. Filter is STILL applied — any
///      scan that started before persist still reports the old state.
///   3. **Drained**: a post-confirm `SessionsUpdated` independently
///      reports the session on the correct side (hidden for archives,
///      active for unarchives). Only then is the entry removed.
///
/// Both gates (confirmation + independent observation) are required to
/// drain. Dropping either one reopens the bounce-back race where the
/// count dips briefly then climbs back as stale scans land.
#[derive(Clone, Debug)]
struct PendingTransition {
    key: String,
    confirmed: bool,
}

/// State for the group-assignment text prompt.
#[derive(Debug, Clone)]
struct GroupPrompt {
    /// The session key being assigned ("provider:session_id").
    session_key: String,
    /// Current text input (filter / new-group name).
    input: String,
    /// Index into the filtered suggestions list of the highlighted item.
    /// `Enter` uses the highlighted suggestion when in range; otherwise the
    /// raw `input` value (creating a new group).
    cursor: usize,
}

/// Which group field has focus.
#[derive(Debug, Clone, Copy, PartialEq)]
enum GroupEditField {
    Name,
    Description,
}

/// State for editing a group — immediately editable, no menu.
#[derive(Debug, Clone)]
struct GroupEditPrompt {
    /// The original group name (before any edits).
    original_name: String,
    /// Which field is focused.
    field: GroupEditField,
    /// Current name input.
    name_input: String,
    /// Current description input.
    desc_input: String,
}

/// State for ACP AI suggestion flow.
#[derive(Debug, Clone)]
enum AcpState {
    /// No ACP operation running.
    Idle,
    /// ACP agent subprocess running, waiting for response.
    Running { started: std::time::Instant },
    /// Suggestions received, user reviewing them.
    Results { suggestions: Vec<AiSuggestion>, cursor: usize },
    /// ACP call failed.
    Failed(String),
}

/// The main TUI application state.
pub struct App {
    sessions: Vec<Session>,
    /// Hidden sessions (archived + filtered-out empty ones).
    hidden_sessions: Vec<Session>,
    /// Filtered view of sessions (indexes into current view's list).
    filtered_indices: Vec<usize>,
    selected_index: usize,
    list_state: ListState,
    focus: Focus,
    view_mode: ViewMode,
    log_lines: Vec<String>,
    log_scroll: usize,
    status_message: String,
    should_quit: bool,
    provider_keys: Vec<String>,
    default_provider: String,
    detail_scroll: u16,
    search_active: bool,
    search_query: String,
    log_max_lines: usize,
    /// Providers that have reported in at least once. Once all are in, initial load is complete.
    seen_providers: std::collections::HashSet<String>,
    /// True once all providers have reported their first results.
    initial_load_complete: bool,
    /// True once user has manually pressed up/down. Prevents selection reset on refresh.
    user_navigated: bool,
    /// Sessions archived locally this cycle — filtered out until supervisor confirms
    /// AND a post-persist scan independently reports them as hidden. The two-gate
    /// drain is what prevents stale in-flight scans (that started before the
    /// archive record was persisted) from bouncing the session back to active.
    pending_archives: Vec<PendingTransition>,
    /// Mirrors pending_archives for the reverse direction: keys of sessions
    /// the user just unarchived (via 'a' in Hidden view). Used to locally
    /// filter them out of `hidden_sessions` until `UnarchiveConfirmed`
    /// arrives AND a post-persist scan independently reports them as active,
    /// preventing the symmetric bounce-back race.
    pending_unarchives: Vec<PendingTransition>,
    /// Which filtered indices had a semantic match boost (for ✨ indicator).
    semantic_matches: std::collections::HashSet<usize>,
    /// Semantic plugin (shared with background indexer). Always use try_lock() — never block UI.
    semantic: std::sync::Arc<std::sync::Mutex<crate::search::SemanticPlugin>>,
    /// Separately-locked snapshot of semantic status. The indexer updates this
    /// with a *nanosecond-scale* lock (independent of the big plugin mutex it
    /// holds for seconds while embedding), so the UI always gets a fresh
    /// value via try_lock even while indexing is mid-embed.
    semantic_status_handle: std::sync::Arc<std::sync::Mutex<crate::search::SemanticStatus>>,
    /// Last known semantic status (cached from the handle above for draw()).
    semantic_status_cache: crate::search::SemanticStatus,
    /// Provider registry — needed to resolve each session's log paths.
    registry: std::sync::Arc<ProviderRegistry>,
    /// Tantivy-backed full-log search engine. `None` if the index failed to open
    /// (in that case we silently fall back to metadata-only search).
    log_searcher: Option<std::sync::Arc<LogSearcher>>,
    /// Guard to prevent overlapping refresh threads.
    log_refresh_running: std::sync::Arc<AtomicBool>,
    /// UI loop tick / event-poll interval (ms). Configurable via config.toml.
    tick_rate_ms: u64,
    /// Minimum interval (ms) between semantic-indexer runs. Even if data
    /// changes, indexing won't fire more often than this. Configurable.
    semantic_index_min_interval_ms: u64,
    /// Last instant at which the semantic indexer was spawned. Used to throttle.
    last_semantic_index_at: Option<std::time::Instant>,
    /// Per-provider shortcut keys: char → provider_key. Built at startup from YAML configs.
    shortcut_map: std::collections::HashMap<char, String>,
    /// Session group manager (multi-group assignments, persisted to groups.json).
    group_mgr: GroupManager,
    /// Active group-assignment prompt (None = not showing).
    group_prompt: Option<GroupPrompt>,
    /// Active description-editing prompt (None = not showing).
    group_edit: Option<GroupEditPrompt>,
    /// ACP AI suggestion state machine.
    acp_state: AcpState,
    /// ACP configuration (command, extra_args, prompt_template).
    acp_config: crate::config::AcpConfig,
    /// In Grouped view: maps visual row index → session index in self.sessions.
    /// None = header row (not selectable as session). Rebuilt each draw.
    grouped_row_map: Vec<Option<usize>>,
    /// In Grouped view: maps visual row index → group name for header rows.
    grouped_header_names: Vec<(usize, String)>,
    /// In Grouped view: frozen ordering of group names captured the first time
    /// the user enters the view since the last reset. Sort key is "max member
    /// activity" (most recently touched group first), so on entry the user
    /// sees what they're working on at the top — without re-shuffling on
    /// every 2s scan refresh while they browse. Cleared when leaving the
    /// view; refreshed by Shift+Tab cycling out and back in.
    grouped_view_sort_order: Option<Vec<String>>,
    /// Deferred selection restore: (provider, session_id) to find in grouped_row_map
    /// during the next draw cycle. Set on data refresh in Grouped view.
    pending_restore_selection: Option<(String, String)>,
    /// Auto-AI suggestions: session_key ("provider:session_id") → suggestion.
    /// Populated in the background after initial load completes (config gated).
    /// Consumed by `y`/`n`/`e` from the Active view when the cursor is on a
    /// session that has a pending suggestion.
    auto_suggestions: std::collections::HashMap<String, AiSuggestion>,
    /// True if AI grouping (`s` key + auto-suggest) can run — we found the
    /// prompts/group-suggest.md template at startup. When false, we hide the
    /// `s` shortcut from the help bar and skip auto-suggest entirely.
    acp_available: bool,
    /// Have we already kicked the auto-suggest run for this session?
    auto_suggest_kicked: bool,
    /// Session keys (`provider:session_id`) that have been sent to the AI in
    /// any prior auto-suggest batch this TUI run. Used to chain follow-up
    /// batches without re-asking the same top-30 every time.
    auto_suggest_asked: std::collections::HashSet<String>,
    /// True while the most recent ACP run was started by the auto-suggest path
    /// (vs the manual `s` key). Differentiates how we treat the result/error
    /// without surfacing a modal popup for an unattended background run.
    acp_run_is_auto: bool,
}

impl App {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        provider_keys: Vec<String>,
        default_provider: String,
        log_max_lines: usize,
        registry: std::sync::Arc<ProviderRegistry>,
        data_dir: PathBuf,
        semantic: std::sync::Arc<std::sync::Mutex<crate::search::SemanticPlugin>>,
        tick_rate_ms: u64,
        semantic_index_min_interval_ms: u64,
        group_mgr: GroupManager,
        acp_config: Option<crate::config::AcpConfig>,
    ) -> Self {
        let mut list_state = ListState::default();
        // No selection until all providers report in
        list_state.select(None);

        // Semantic plugin is preloaded in main.rs BEFORE the TUI enters alternate-screen
        // mode, so fastembed's first-run model download progress bar renders cleanly on
        // the normal terminal instead of corrupting the TUI's top rows.

        // Grab the shared-status handle once at construction. It stays in sync
        // with the plugin's internal status without needing the big plugin mutex.
        let (semantic_status_handle, initial_status) = {
            let guard = semantic.lock().unwrap();
            (guard.shared_status(), guard.status().clone())
        };

        let provider_count = provider_keys.len();
        // If zero providers are enabled (e.g. user hasn't installed any agent CLI
        // yet) there will never be any scan events, so mark initial load as
        // already complete and show a helpful message instead of a stuck spinner.
        let (no_providers, initial_status_msg) = empty_provider_bootstrap(provider_count);

        // Open (or create) the tantivy full-text log index. If it fails for any
        // reason we disable log content search rather than blowing up the UI —
        // metadata search still works.
        let log_searcher = match LogSearcher::open_or_create(&data_dir) {
            Ok(s) => Some(std::sync::Arc::new(s)),
            Err(e) => {
                crate::log::info(&format!("Log index unavailable: {}", e));
                None
            }
        };

        // Build per-provider shortcut map with collision detection.
        // Reserved keys that must not be used as provider shortcuts.
        let reserved: std::collections::HashSet<char> =
            ['q', '/', 'a', 'h', 'v'].iter().copied().collect();
        let mut shortcut_map: std::collections::HashMap<char, String> =
            std::collections::HashMap::new();
        let mut shortcut_errors: Vec<String> = Vec::new();
        for p in registry.providers() {
            if let Some(ch) = p.new_session_shortcut() {
                let ch = ch.to_ascii_lowercase();
                if reserved.contains(&ch) {
                    shortcut_errors.push(format!(
                        "Provider '{}': shortcut '{}' is reserved — ignored",
                        p.key(), ch
                    ));
                } else if let Some(existing) = shortcut_map.get(&ch) {
                    shortcut_errors.push(format!(
                        "Shortcut '{}' collision: '{}' and '{}' — both ignored",
                        ch, existing, p.key()
                    ));
                    shortcut_map.remove(&ch);
                } else {
                    shortcut_map.insert(ch, p.key().to_string());
                }
            }
        }
        let initial_status_msg = if !shortcut_errors.is_empty() {
            let msg = format!("⚠ {}", shortcut_errors.join("; "));
            crate::log::info(&msg);
            msg
        } else {
            initial_status_msg
        };

        // ACP is **explicit opt-in**: the feature is only available when the
        // user has written an `[acp]` section in their config.toml AND the
        // prompt template is findable next to the binary. Both conditions
        // must hold; either alone is not enough. Without explicit opt-in the
        // `s` shortcut is hidden, auto-suggest never kicks, and no copilot
        // process is spawned.
        let user_opted_in = acp_config.is_some();
        let acp_config = acp_config.unwrap_or_default();
        let acp_available = user_opted_in
            && crate::acp::resolve_template(&acp_config).is_some();

        Self {
            sessions: Vec::new(),
            hidden_sessions: Vec::new(),
            filtered_indices: Vec::new(),
            selected_index: 0,
            list_state,
            focus: Focus::SessionList,
            view_mode: ViewMode::Active,
            log_lines: vec!["Session manager started. Scanning for sessions...".into()],
            log_scroll: 0,
            status_message: initial_status_msg,
            should_quit: false,
            default_provider,
            provider_keys,
            detail_scroll: 0,
            search_active: false,
            search_query: String::new(),
            log_max_lines,
            seen_providers: std::collections::HashSet::new(),
            initial_load_complete: no_providers,
            user_navigated: false,
            pending_archives: Vec::new(),
            pending_unarchives: Vec::new(),
            semantic_matches: std::collections::HashSet::new(),
            semantic,
            semantic_status_handle,
            semantic_status_cache: initial_status,
            registry,
            log_searcher,
            log_refresh_running: std::sync::Arc::new(AtomicBool::new(false)),
            tick_rate_ms,
            semantic_index_min_interval_ms,
            last_semantic_index_at: None,
            shortcut_map,
            group_mgr,
            group_prompt: None,
            group_edit: None,
            acp_state: AcpState::Idle,
            acp_config,
            grouped_row_map: Vec::new(),
            grouped_header_names: Vec::new(),
            grouped_view_sort_order: None,
            pending_restore_selection: None,
            auto_suggestions: std::collections::HashMap::new(),
            acp_available,
            auto_suggest_kicked: false,
            auto_suggest_asked: std::collections::HashSet::new(),
            acp_run_is_auto: false,
        }
    }

    /// Preload the TUI with a curated mock session list and disable any
    /// background features that would conflict with a static demo (provider
    /// scan, real AI grouping spawn, semantic indexing).
    ///
    /// Used only by the `--mock-data` startup path. The supervisor is also
    /// skipped at the call-site, so no scan ever overwrites these sessions.
    pub fn preload_demo_data(&mut self, sessions: Vec<Session>) {
        self.sessions = sessions;
        self.initial_load_complete = true;
        // The `s` key (real AI grouping spawn) must never fire in mock mode —
        // we'd spawn `copilot` against synthetic data. The y/n/e accept/dismiss/
        // edit keys are gated on `pending_suggestion_for_selection()`, NOT on
        // `acp_available`, so they still work against the pre-populated
        // suggestions map. Inline-shadow rendering is similarly ungated.
        self.acp_available = false;
        self.list_state.select(Some(0));
        self.apply_filter();
        self.status_message =
            "🎬 Mock data — for demos and screenshots".into();
        self.log_lines
            .push(format!("Mock mode: loaded {} sessions", self.sessions.len()));
    }

    /// Add pre-existing group assignments, used in `--mock-data` mode so the
    /// Grouped view shows realistic distribution instead of an empty page.
    /// Uses in-memory assignment (no disk write) so the demo never touches
    /// the user's real groups.json.
    pub fn preload_demo_groups(&mut self, assignments: Vec<(String, String)>) {
        for (session_key, group) in assignments {
            self.group_mgr.assign_in_memory(&session_key, &group);
        }
    }

    /// Pre-populate AI auto-suggestions for the demo flow. In normal runs
    /// these come from a real ACP batch; in `--mock-data` mode we hand-pick
    /// a few so the inline `· 🤖 ⟨group⟩` shadow + y/n/e shortcuts work
    /// without ever spawning copilot.
    pub fn preload_demo_suggestions(
        &mut self,
        suggestions: std::collections::HashMap<String, crate::acp::AiSuggestion>,
    ) {
        self.auto_suggestions = suggestions;
        // Mark as kicked so `maybe_kick_auto_suggest` doesn't try to start
        // a real run on top.
        self.auto_suggest_kicked = true;
    }

    /// Get the list being displayed based on view mode.
    fn current_view_sessions(&self) -> &[Session] {
        match self.view_mode {
            ViewMode::Active | ViewMode::Grouped => &self.sessions,
            ViewMode::Hidden => &self.hidden_sessions,
        }
    }

    /// Get the currently selected session (through the filter).
    fn selected_session(&self) -> Option<&Session> {
        if self.view_mode == ViewMode::Grouped && !self.grouped_row_map.is_empty() {
            // In grouped view, use the row map to resolve visual index → session
            self.grouped_row_map
                .get(self.selected_index)
                .and_then(|opt| opt.as_ref())
                .and_then(|&idx| self.current_view_sessions().get(idx))
        } else {
            self.filtered_indices
                .get(self.selected_index)
                .and_then(|&idx| self.current_view_sessions().get(idx))
        }
    }

    /// Existing groups that match the current prompt input (case-insensitive
    /// substring). Returns ALL matches sorted by descending member count
    /// (stable: ties broken by name). The caller is responsible for
    /// rendering only a window — cursor may point at any index.
    fn filter_groups_for_prompt(&self, input: &str) -> Vec<(String, usize)> {
        let all_groups = self.group_mgr.all_groups();
        if input.trim().is_empty() {
            all_groups
        } else {
            let q = input.to_lowercase();
            all_groups
                .into_iter()
                .filter(|(name, _)| name.to_lowercase().contains(&q))
                .collect()
        }
    }

    /// Pending auto-suggestion for the currently selected session, if any.
    /// Used to gate the `y`/`n`/`e` keys and to drive the dynamic title-bar hints.
    fn pending_suggestion_for_selection(&self) -> Option<&AiSuggestion> {
        let session = self.selected_session()?;
        let key = format!("{}:{}", session.provider_name, session.provider_session_id);
        self.auto_suggestions.get(&key)
    }

    /// Kick a background AI grouping run if eligible. Wraps the same path the
    /// `s` key uses, but flagged as auto so a failure is logged silently
    /// instead of opening a modal banner.
    fn maybe_kick_auto_suggest(
        &mut self,
        cmd_tx: &mpsc::UnboundedSender<SupervisorCommand>,
    ) {
        if self.auto_suggest_kicked {
            return;
        }
        if !self.acp_available {
            crate::log::info("AI auto-suggest SKIPPED: prompts/group-suggest.md not found next to binary");
            return;
        }
        if !self.acp_config.auto_suggest {
            return;
        }
        if !self.initial_load_complete {
            return;
        }
        if matches!(self.acp_state, AcpState::Running { .. }) {
            return;
        }
        // Snapshot once so we set the flag exactly when we attempt — even if
        // prepare_prompt fails, we don't want to retry on every event tick.
        self.auto_suggest_kicked = true;
        let sem_ref = self.semantic.try_lock().ok();
        let sem_plugin = sem_ref.as_deref();
        match crate::acp::prepare_prompt(
            &self.acp_config,
            &self.sessions,
            &self.group_mgr,
            sem_plugin,
            &self.auto_suggest_asked,
        ) {
            Ok((prompt, asked_keys)) => {
                let count = asked_keys.len();
                // Record what we just sent so the next chained batch picks
                // up sessions that didn't fit in this one.
                for k in &asked_keys {
                    self.auto_suggest_asked.insert(k.clone());
                }
                self.acp_state = AcpState::Running {
                    started: std::time::Instant::now(),
                };
                self.acp_run_is_auto = true;
                self.status_message = format!("🤖 Auto-analyzing {} sessions in background...", count);
                self.log_lines.push(format!(
                    "AI auto-suggest: sending {} sessions to ACP agent (total asked so far: {})",
                    count, self.auto_suggest_asked.len()
                ));
                crate::log::info(&format!(
                    "AI auto-suggest KICKED: {} sessions (cumulative asked={})",
                    count, self.auto_suggest_asked.len()
                ));
                let cfg = self.acp_config.clone();
                let event_tx = cmd_tx.clone();
                // Pre-generate the UUID copilot will use for this grouping
                // session and archive it BEFORE spawning so the session
                // never surfaces in the user's active list. Race-free in
                // practice: the supervisor processes ArchiveSession FIFO
                // and the next periodic scan is poll_interval_ms away,
                // while copilot's -p mode takes ~30s to write any session
                // data — plenty of time for the archive cmd to land.
                let acp_session_id = uuid::Uuid::new_v4().to_string();
                let _ = cmd_tx.send(SupervisorCommand::ArchiveSession {
                    provider_session_id: acp_session_id.clone(),
                    provider_key: "copilot".to_string(),
                });
                crate::log::info(&format!(
                    "AI auto-suggest: pre-archived copilot:{} (ACP session)",
                    acp_session_id
                ));
                tokio::spawn(async move {
                    let timeout_secs = cfg.timeout_secs;
                    let timeout = tokio::time::Duration::from_secs(timeout_secs);
                    let result = tokio::time::timeout(
                        timeout,
                        crate::acp::run_acp_suggest(cfg, prompt, acp_session_id),
                    )
                    .await;
                    let _ = match result {
                        Ok(Ok(suggestions)) => {
                            let json = serde_json::to_string(&suggestions).unwrap_or_default();
                            event_tx.send(SupervisorCommand::AcpResult(json))
                        }
                        Ok(Err(e)) => event_tx.send(SupervisorCommand::AcpError(e)),
                        Err(_) => {
                            crate::log::warn(&format!(
                                "AI auto-suggest TIMED OUT after {}s",
                                timeout_secs
                            ));
                            event_tx.send(SupervisorCommand::AcpError(format!(
                                "ACP agent timed out after {}s",
                                timeout_secs
                            )))
                        }
                    };
                });
            }
            Err(e) => {
                // Most common path: "No ungrouped sessions to analyze".
                // Log silently — no banner.
                self.log_lines
                    .push(format!("AI auto-suggest skipped: {}", e));
                crate::log::info(&format!("AI auto-suggest SKIPPED: {}", e));
            }
        }
    }

    /// Rebuild the filtered indices based on the search query.
    /// Uses tiered ranking: exact → fuzzy → semantic (from cached embeddings).
    fn apply_filter(&mut self) {
        let query = self.search_query.clone();
        if query.is_empty() {
            self.semantic_matches.clear();
            let len = self.current_view_sessions().len();
            self.filtered_indices = (0..len).collect();
        } else {
            // try_lock: skip semantic if indexer holds the lock (never block UI)
            // Keep previous semantic_matches if lock unavailable (avoids sparkle flicker)
            let mut sem = if query.len() >= 5 {
                self.semantic.try_lock().ok()
            } else {
                None
            };
            // If the model was unloaded after indexing finished, reload it
            // on-demand for this query. Blocks for ~1-2s ONLY on the first
            // search after idle. No-op if already loaded.
            if let Some(ref mut guard) = sem {
                if guard.is_ready() && !guard.is_loaded() {
                    let dir = guard.cache_dir().unwrap_or("").to_string();
                    guard.ensure_loaded(&dir);
                }
            }
            let sem_ref = sem.as_deref();
            let view = self.current_view_sessions();
            // Query the tantivy index — returns session_id → BM25 score. Empty
            // map on empty query or missing index; `ranked_search` handles the
            // lookup for us.
            let log_matches = self
                .log_searcher
                .as_ref()
                .map(|ls| ls.search(&query))
                .unwrap_or_default();
            let log_ref = if log_matches.is_empty() { None } else { Some(&log_matches) };
            let results = crate::search::ranked_search_default(view, &query, sem_ref, log_ref);
            // Only update semantic matches if we actually ran semantic search
            if sem_ref.is_some() {
                self.semantic_matches.clear();
                for r in &results {
                    if r.semantic_match {
                        self.semantic_matches.insert(r.index);
                    }
                }
            }
            self.filtered_indices = results.into_iter().map(|r| r.index).collect();
        }
        // Always select the top result after filtering
        self.selected_index = 0;
        self.list_state.select(Some(0));
    }

    pub async fn run(
        mut self,
        mut event_rx: mpsc::UnboundedReceiver<SupervisorEvent>,
        cmd_tx: mpsc::UnboundedSender<SupervisorCommand>,
    ) -> Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, cursor::Hide)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;

        // Ensure terminal is always restored, even on panic
        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            crate::log::panic(panic_info);
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen, cursor::Show);
            original_hook(panic_info);
        }));

        // Tick rate is configurable (config.toml: tick_rate_ms; default 5000).
        // Higher value = lower idle CPU (5s → near-zero), but spinner animations
        // and status updates appear at that cadence. Keypresses are always
        // instant because event::poll returns immediately on input.
        let tick_rate = std::time::Duration::from_millis(self.tick_rate_ms);

        loop {
            // Update semantic status from the shared-status handle. This lock
            // is only ever held for nanoseconds (status writes), so try_lock
            // almost always succeeds — even while the indexer thread is
            // holding the big plugin mutex for an in-flight embed.
            if let Ok(status) = self.semantic_status_handle.try_lock() {
                self.semantic_status_cache = status.clone();
            }

            // Draw
            terminal.draw(|f| {
                self.draw(f);
            })?;

            // Handle events (non-blocking with timeout)
            if event::poll(tick_rate)? {
                if let Event::Key(key) = event::read()? {
                    // Only handle Press to avoid double/triple on Windows
                    if key.kind == KeyEventKind::Press {
                        self.handle_key(key, &cmd_tx);
                    }
                }
            }

            // Drain supervisor events
            while let Ok(ev) = event_rx.try_recv() {
                match ev {
                    SupervisorEvent::SessionsUpdated { provider_key, mut active, mut hidden } => {
                        // Snapshot the scan's ORIGINAL placement before any
                        // local filter runs. These snapshots tell us what the
                        // scan itself saw on disk — essential for the drain
                        // logic below. A stale in-flight scan reports an
                        // archived session in `active`; a post-persist scan
                        // reports it in `hidden`. We only drain pending
                        // entries when the scan's OWN view confirms the new
                        // state, never based on moves we performed.
                        let scan_hidden_keys: std::collections::HashSet<String> = hidden
                            .iter()
                            .map(|s| format!("{}:{}", s.provider_name, s.provider_session_id))
                            .collect();
                        let scan_active_keys: std::collections::HashSet<String> = active
                            .iter()
                            .map(|s| format!("{}:{}", s.provider_name, s.provider_session_id))
                            .collect();

                        // Filter out sessions that were just archived locally
                        if !self.pending_archives.is_empty() {
                            let mut moved = Vec::new();
                            active.retain(|s| {
                                let key = format!("{}:{}", s.provider_name, s.provider_session_id);
                                if self.pending_archives.iter().any(|p| p.key == key) {
                                    moved.push(s.clone());
                                    false
                                } else {
                                    true
                                }
                            });
                            hidden.extend(moved);
                        }

                        // Symmetric case: sessions just unarchived locally
                        // must not bounce back into `hidden` before the
                        // unarchive is persisted. Same race guard.
                        if !self.pending_unarchives.is_empty() {
                            let mut moved = Vec::new();
                            hidden.retain(|s| {
                                let key = format!("{}:{}", s.provider_name, s.provider_session_id);
                                if self.pending_unarchives.iter().any(|p| p.key == key) {
                                    moved.push(s.clone());
                                    false
                                } else {
                                    true
                                }
                            });
                            active.extend(moved);
                        }

                        // Drain pending entries now that the filter has run.
                        // Two gates: (1) the supervisor has confirmed the
                        // persist; (2) the scan's ORIGINAL view (captured
                        // before our filter moved anything) reports the
                        // session on the expected side. Both gates must
                        // pass — otherwise a stale scan that started before
                        // persist would prematurely drain the entry, the
                        // pending filter would disappear, and a later stale
                        // scan would repopulate the session on the wrong
                        // side. That exact sequence is what produced the
                        // "count drops to 2xx, bounces to 4xx" regression
                        // and the "unarchived session vanishes entirely"
                        // regression.
                        self.pending_archives.retain(|p| {
                            !(p.confirmed && scan_hidden_keys.contains(&p.key))
                        });
                        self.pending_unarchives.retain(|p| {
                            !(p.confirmed && scan_active_keys.contains(&p.key))
                        });

                        let active_count = active.len();
                        let hidden_count = hidden.len();

                        // Check if data actually changed
                        // Check if data actually changed
                        // Exclude updated_at: mtime changes every scan for running sessions
                        // Compare summary instead — it only changes when content actually changes
                        let data_changed = active.len() != self.sessions.len()
                            || active.iter().zip(self.sessions.iter()).any(|(new, old)| {
                                new.id != old.id
                                    || new.state != old.state
                                    || new.title != old.title
                                    || new.tab_title != old.tab_title
                                    || new.summary != old.summary
                            });

                        // Track which providers have reported in. Prefer the
                        // provider_key on the event (reliable even when a
                        // provider returns 0 sessions), fall back to inferring
                        // from session data for legacy / broadcast events.
                        if let Some(ref key) = provider_key {
                            self.seen_providers.insert(key.clone());
                        } else {
                            for s in &active {
                                self.seen_providers.insert(s.provider_name.clone());
                            }
                            for s in &hidden {
                                self.seen_providers.insert(s.provider_name.clone());
                            }
                        }
                        let all_providers_in = self.provider_keys.iter()
                            .all(|k| self.seen_providers.contains(k));

                        // Detect the transition from "still loading" → "done".
                        let just_completed_initial_load =
                            all_providers_in && !self.initial_load_complete;
                        if just_completed_initial_load {
                            self.initial_load_complete = true;
                        }

                        // Kick the auto AI grouping run once the first full
                        // discovery is in. Idempotent — `maybe_kick_auto_suggest`
                        // self-gates on `auto_suggest_kicked`.
                        self.maybe_kick_auto_suggest(&cmd_tx);

                        // Always accumulate sessions so they're ready the moment
                        // initial load completes. But we gate *rendering* of the
                        // list/selection/detail on initial_load_complete to avoid
                        // the cold-start flicker where rows appear without a
                        // highlight and the detail pane churns against partial data.
                        let user_reading_detail = self.focus == Focus::Detail && self.detail_scroll > 0;

                        let prev_selected_id = if self.user_navigated {
                            self.selected_session()
                                .map(|s| (s.provider_name.clone(), s.provider_session_id.clone()))
                        } else {
                            None
                        };

                        let set_changed = active.len() != self.sessions.len()
                            || active.iter().zip(self.sessions.iter()).any(|(new, old)| new.id != old.id);

                        self.sessions = active;
                        self.hidden_sessions = hidden;

                        if !self.initial_load_complete {
                            // Still waiting for at least one provider — keep the
                            // list empty and the selection cleared. User sees
                            // "Loading X/N providers..." with nothing flickering.
                            self.filtered_indices.clear();
                            self.semantic_matches.clear();
                            self.selected_index = 0;
                            self.list_state.select(None);
                        } else if !user_reading_detail && (just_completed_initial_load || data_changed) && (set_changed || !self.search_active) {
                            self.apply_filter();

                            if just_completed_initial_load || !self.user_navigated {
                                // First full render, or user hasn't navigated yet → row 0.
                                self.selected_index = 0;
                                self.list_state.select(Some(0));
                                self.detail_scroll = 0;
                            } else if let Some((prev_provider, prev_id)) = &prev_selected_id {
                                // User navigated → restore their position across refreshes.
                                if self.view_mode == ViewMode::Grouped {
                                    // Defer to draw cycle when grouped_row_map is rebuilt
                                    self.pending_restore_selection = Some((prev_provider.clone(), prev_id.clone()));
                                } else {
                                    let view = self.current_view_sessions();
                                    if let Some(pos) = self.filtered_indices.iter().position(|&idx| {
                                        let s = &view[idx];
                                        &s.provider_name == prev_provider && &s.provider_session_id == prev_id
                                    }) {
                                        self.selected_index = pos;
                                        self.list_state.select(Some(pos));
                                    }
                                }
                            }
                        }

                        if data_changed {
                            // Throttle: skip if we ran the indexer recently.
                            // Configurable via semantic_index_min_interval_ms.
                            let should_run = match self.last_semantic_index_at {
                                None => true,
                                Some(t) => {
                                    t.elapsed().as_millis() as u64
                                        >= self.semantic_index_min_interval_ms
                                }
                            };
                            if !should_run {
                                let remain_ms = self
                                    .last_semantic_index_at
                                    .map(|t| {
                                        self.semantic_index_min_interval_ms
                                            .saturating_sub(t.elapsed().as_millis() as u64)
                                    })
                                    .unwrap_or(0);
                                crate::log::info(&format!(
                                    "[idx] data_changed=true throttled, {}ms remaining",
                                    remain_ms
                                ));
                            } else {
                                self.last_semantic_index_at = Some(std::time::Instant::now());
                                crate::log::info("[idx] data_changed=true, eligible to run");
                            // Background semantic indexing. Embeds title + summary
                            // + cwd + log head/tail per session (hash-gated so
                            // unchanged sessions skip). CRITICAL: acquire and
                            // release the plugin mutex PER SESSION so the UI can
                            // (a) read live progress via the separate
                            // shared_status handle and (b) run user searches
                            // without waiting for the whole indexing run.
                            let sem_clone = self.semantic.clone();
                            let registry = std::sync::Arc::clone(&self.registry);
                            let all_sessions: Vec<Session> = self.sessions.clone();

                            // Quick pre-check: if nothing needs re-embedding,
                            // don't spawn the indexer thread at all. This avoids
                            // N round-trip locks per refresh tick once the
                            // corpus is fully indexed.
                            let precheck_start = std::time::Instant::now();
                            let total_sessions = all_sessions.len();
                            let stale_count = {
                                match sem_clone.lock() {
                                    Ok(sem) => {
                                        if sem.lib.is_none() {
                                            0
                                        } else {
                                            sem.count_needing_embedding(&all_sessions, |s| {
                                                build_semantic_chunks(s, &registry)
                                                    .first()
                                                    .map(|(t, _)| t.clone())
                                                    .unwrap_or_default()
                                            })
                                        }
                                    }
                                    Err(_) => 0,
                                }
                            };
                            let precheck_ms = precheck_start.elapsed().as_millis();
                            crate::log::info(&format!(
                                "[idx] precheck: stale={} total={} ({}ms)",
                                stale_count, total_sessions, precheck_ms
                            ));
                            let should_index = stale_count > 0;

                            if should_index {
                                std::thread::spawn(move || {
                                let thread_start = std::time::Instant::now();
                                let total = all_sessions.len();
                                let mut embedded_since_flush = 0usize;

                                // Make sure the model is loaded. After an idle
                                // period we may have unloaded it to save memory.
                                let load_start = std::time::Instant::now();
                                let was_already_loaded;
                                {
                                    let mut sem = match sem_clone.lock() {
                                        Ok(g) => g,
                                        Err(_) => return,
                                    };
                                    was_already_loaded = sem.lib.is_some();
                                    let dir = sem.cache_dir().unwrap_or("").to_string();
                                    if !sem.ensure_loaded(&dir) {
                                        crate::log::warn("[idx] ensure_loaded failed");
                                        return;
                                    }
                                }
                                let load_ms = load_start.elapsed().as_millis();
                                crate::log::info(&format!(
                                    "[idx] model_load: {}ms (already_loaded={})",
                                    load_ms, was_already_loaded
                                ));

                                let embed_loop_start = std::time::Instant::now();
                                let mut embedded_count = 0usize;
                                let mut total_embed_ms: u128 = 0;
                                for (i, session) in all_sessions.iter().enumerate() {
                                    let chunks = build_semantic_chunks(session, &registry);
                                    // Use first chunk's hash as identity
                                    let identity_hash = chunks.first()
                                        .map(|(t, _)| crate::search::hash_text(t))
                                        .unwrap_or(0);

                                    // Short lock: skip-check.
                                    let needs = {
                                        let sem = match sem_clone.lock() {
                                            Ok(g) => g,
                                            Err(_) => return,
                                        };
                                        sem.needs_embedding(&session.id, identity_hash)
                                    };

                                    if needs {
                                        let one_embed_start = std::time::Instant::now();
                                        let mut sem = match sem_clone.lock() {
                                            Ok(g) => g,
                                            Err(_) => return,
                                        };
                                        let chunk_pairs: Vec<(String, u64)> = chunks.iter()
                                            .map(|(t, _)| (t.clone(), crate::search::hash_text(t)))
                                            .collect();
                                        let count = sem.embed_and_cache_multi(&session.id, &chunk_pairs);
                                        if count > 0 {
                                            embedded_since_flush += 1;
                                            embedded_count += 1;
                                        }
                                        sem.update_progress(i + 1, total);
                                        if embedded_since_flush >= 20 {
                                            sem.save_cache();
                                            embedded_since_flush = 0;
                                        }
                                        drop(sem);
                                        let one_embed_ms =
                                            one_embed_start.elapsed().as_millis();
                                        total_embed_ms += one_embed_ms;
                                        crate::log::info(&format!(
                                            "[idx] embed session={} chunks={} ({}ms)",
                                            &session.id[..session.id.len().min(8)],
                                            count,
                                            one_embed_ms
                                        ));
                                    } else {
                                        if let Ok(mut sem) = sem_clone.lock() {
                                            sem.update_progress(i + 1, total);
                                        }
                                    }
                                }
                                let embed_loop_ms = embed_loop_start.elapsed().as_millis();
                                crate::log::info(&format!(
                                    "[idx] embed_loop: {} embedded in {}ms (sum_embed={}ms)",
                                    embedded_count, embed_loop_ms, total_embed_ms
                                ));

                                // Final flush, mark Ready, and unload the model
                                // to return ~550MB of weights to the OS. The
                                // model reloads on demand next time the user
                                // runs a semantic search query.
                                let unload_start = std::time::Instant::now();
                                if let Ok(mut sem) = sem_clone.lock() {
                                    if embedded_since_flush > 0 {
                                        sem.save_cache();
                                    }
                                    sem.mark_ready();
                                    sem.unload();
                                }
                                let unload_ms = unload_start.elapsed().as_millis();
                                let total_ms = thread_start.elapsed().as_millis();
                                crate::log::info(&format!(
                                    "[idx] DONE total={}ms (load={}ms embed_loop={}ms unload={}ms embedded={})",
                                    total_ms, load_ms, embed_loop_ms, unload_ms, embedded_count
                                ));
                                });
                            }
                            } // end else (should_run)
                        }

                        // Background log-content index refresh. Guarded by an
                        // atomic flag so overlapping spawns collapse into one.
                        // Pass BOTH active + hidden so archived sessions stay
                        // searchable (Hidden-view still finds content); any
                        // session no longer in either list gets evicted, so
                        // deleted sessions can't match phantom content.
                        if just_completed_initial_load || data_changed {
                            if let Some(log_searcher) = &self.log_searcher {
                                if !self.log_refresh_running.swap(true, Ordering::SeqCst) {
                                    let registry = std::sync::Arc::clone(&self.registry);
                                    let searcher = std::sync::Arc::clone(log_searcher);
                                    let running = std::sync::Arc::clone(&self.log_refresh_running);
                                    let mut all_sessions: Vec<Session> = self.sessions.clone();
                                    all_sessions.extend(self.hidden_sessions.iter().cloned());
                                    std::thread::spawn(move || {
                                        if let Err(e) = searcher.refresh(&all_sessions, &registry) {
                                            // Use {:#} to surface the full anyhow chain
                                            // (e.g. "tantivy commit (chunk): IO error: ...").
                                            // Without this, only the top-level context shows,
                                            // making tantivy failures un-diagnosable.
                                            crate::log::error(&format!(
                                                "log index refresh failed: {:#}",
                                                e
                                            ));
                                        }
                                        running.store(false, Ordering::SeqCst);
                                    });
                                }
                            }
                        }

                        let now = chrono::Local::now().format("%H:%M:%S");
                        self.status_message = if !self.initial_load_complete {
                            let seen = self.seen_providers.len();
                            let total_providers = self.provider_keys.len();
                            format!("Loading providers ({}/{})...", seen, total_providers)
                        } else {
                            let shown = self.filtered_indices.len();
                            let total = match self.view_mode {
                                ViewMode::Active | ViewMode::Grouped => active_count,
                                ViewMode::Hidden => hidden_count,
                            };
                            let mode_label = match self.view_mode {
                                ViewMode::Active => "active",
                                ViewMode::Grouped => "grouped",
                                ViewMode::Hidden => "hidden",
                            };
                            format!(
                                "{}/{} {} · {} hidden · refreshed {}",
                                shown, total, mode_label, hidden_count, now
                            )
                        };

                        // (Duplicate semantic-indexer spawn removed — the
                        // data_changed-guarded spawn above is the only one we need.
                        // This one fired on every SupervisorEvent and burned
                        // ~1% idle CPU spawning redundant threads.)
                    }
                    SupervisorEvent::ArchiveConfirmed { provider_key, provider_session_id } => {
                        // Persist is done, but DO NOT drain the pending entry
                        // here. Scans that were already in flight when 'a'
                        // was pressed can still arrive and report the
                        // session as active. Just mark the entry confirmed
                        // so the SessionsUpdated handler can drain it once
                        // the scan's own view agrees.
                        let key = format!("{}:{}", provider_key, provider_session_id);
                        for p in self.pending_archives.iter_mut() {
                            if p.key == key {
                                p.confirmed = true;
                            }
                        }
                    }
                    SupervisorEvent::UnarchiveConfirmed { provider_key, provider_session_id } => {
                        let key = format!("{}:{}", provider_key, provider_session_id);
                        for p in self.pending_unarchives.iter_mut() {
                            if p.key == key {
                                p.confirmed = true;
                            }
                        }
                    }
                    SupervisorEvent::Error(e) => {
                        self.status_message = format!("Error: {}", e);
                        self.log_lines.push(format!("ERROR: {}", e));
                    }
                    SupervisorEvent::AcpResult(json) => {
                        let was_auto = self.acp_run_is_auto;
                        self.acp_run_is_auto = false;
                        match serde_json::from_str::<Vec<AiSuggestion>>(&json) {
                            Ok(suggestions) if suggestions.is_empty() => {
                                self.acp_state = AcpState::Idle;
                                self.status_message =
                                    "AI: no strong grouping suggestions found".to_string();
                            }
                            Ok(suggestions) => {
                                // Resolve AI session keys against the live
                                // session list. Some agents echo a truncated
                                // session id (e.g. `qwen:4b900c0e` instead of
                                // `qwen:4b900c0e-...-full-uuid`) by mimicking
                                // the example in the prompt template. We match
                                // by exact-or-prefix and rewrite to the canonical
                                // full key so build_session_item finds it.
                                let live_keys: Vec<String> = self
                                    .sessions
                                    .iter()
                                    .map(|s| {
                                        format!("{}:{}", s.provider_name, s.provider_session_id)
                                    })
                                    .collect();
                                let mut resolved: Vec<AiSuggestion> = Vec::new();
                                let mut dropped = 0usize;
                                for mut sg in suggestions {
                                    let raw = sg.session.clone();
                                    // Exact match wins; otherwise prefix match
                                    // (handles truncated UUIDs).
                                    let canonical = live_keys
                                        .iter()
                                        .find(|k| **k == raw)
                                        .cloned()
                                        .or_else(|| {
                                            live_keys
                                                .iter()
                                                .find(|k| k.starts_with(&raw))
                                                .cloned()
                                        });
                                    match canonical {
                                        Some(full) => {
                                            sg.session = full;
                                            resolved.push(sg);
                                        }
                                        None => {
                                            dropped += 1;
                                            crate::log::info(&format!(
                                                "AI suggestion dropped — no session matches '{}'",
                                                raw
                                            ));
                                        }
                                    }
                                }
                                let count = resolved.len();
                                crate::log::info(&format!(
                                    "AI auto-suggest RESOLVED: {} matched, {} dropped (auto={})",
                                    count, dropped, was_auto
                                ));
                                if dropped > 0 {
                                    self.log_lines.push(format!(
                                        "AI: dropped {} suggestion(s) with unmatched session ids",
                                        dropped
                                    ));
                                }
                                for sg in &resolved {
                                    self.auto_suggestions
                                        .insert(sg.session.clone(), sg.clone());
                                }
                                if count == 0 {
                                    self.acp_state = AcpState::Idle;
                                    self.status_message =
                                        "AI: no usable grouping suggestions returned".to_string();
                                } else if was_auto {
                                    // Background run: stay in normal view, no popup.
                                    self.acp_state = AcpState::Idle;
                                    self.status_message = format!(
                                        "🤖 {} AI suggestions ready — y accept · n dismiss · e edit",
                                        count
                                    );
                                } else {
                                    // Manual `s`: legacy popup view, lets the
                                    // user step through the whole batch.
                                    self.acp_state = AcpState::Results {
                                        suggestions: resolved,
                                        cursor: 0,
                                    };
                                    self.status_message = format!(
                                        "AI: {} suggestions ready — review with y/n/e",
                                        count
                                    );
                                }

                                // Auto-suggest runs ONCE per session startup, not in
                                // a chain. Earlier versions chained batches to cover
                                // the entire ungrouped list, but that burned a
                                // copilot session per 30 rows and made the user wait
                                // through 5-10 batches for users with deep history.
                                // One batch (top 30 by recency) is enough — the user
                                // can press `s` for more on demand.
                            }
                            Err(e) => {
                                if was_auto {
                                    self.acp_state = AcpState::Idle;
                                    self.log_lines.push(format!(
                                        "AI auto-suggest parse error: {}",
                                        e
                                    ));
                                } else {
                                    self.acp_state =
                                        AcpState::Failed(format!("Parse error: {}", e));
                                    self.status_message =
                                        format!("⚠ AI response parse error: {}", e);
                                }
                            }
                        }
                    }
                    SupervisorEvent::AcpError(msg) => {
                        let was_auto = self.acp_run_is_auto;
                        self.acp_run_is_auto = false;
                        if was_auto {
                            // Background run: don't lock the screen behind a
                            // modal — log + status line + return to idle.
                            self.acp_state = AcpState::Idle;
                            self.status_message = format!("⚠ AI auto-suggest: {}", msg);
                            self.log_lines
                                .push(format!("ACP auto-suggest error: {}", msg));
                        } else {
                            self.acp_state = AcpState::Failed(msg.clone());
                            self.status_message = format!("⚠ AI: {}", msg);
                            self.log_lines.push(format!("ACP ERROR: {}", msg));
                        }
                    }
                }
            }

            // Trim log lines to configured maximum
            if self.log_max_lines > 0 && self.log_lines.len() > self.log_max_lines {
                let excess = self.log_lines.len() - self.log_max_lines;
                self.log_lines.drain(..excess);
            }

            if self.should_quit {
                let _ = cmd_tx.send(SupervisorCommand::Shutdown);
                break;
            }
        }

        // Restore terminal fully
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen, cursor::Show)?;
        terminal.show_cursor()?;
        Ok(())
    }

    /// Handle Enter key: focus running/waiting sessions, resume others.
    /// Shared between normal mode and search mode.
    fn handle_enter(&mut self, cmd_tx: &mpsc::UnboundedSender<SupervisorCommand>) {
        if let Some(session) = self.selected_session() {
            let psid = session.provider_session_id.clone();
            let pname = session.provider_name.clone();
            let title = session.title.clone();
            let tab_title = session.tab_title.clone();
            let scwd = session.cwd.to_string_lossy().to_string();
            let is_running = session.state.process == crate::models::ProcessState::Running;

            crate::log::info(&format!(
                "Enter: {} state={:?} process={:?} tab_title={:?}",
                crate::util::short_id(&psid, 8),
                session.state.label(),
                session.state.process,
                tab_title.as_deref().unwrap_or("None"),
            ));

            if is_running {
                if let Some(ref tt) = tab_title {
                    let _ = cmd_tx.send(SupervisorCommand::FocusSession {
                        tab_title: Some(tt.clone()),
                        title: title.clone(),
                        provider_session_id: psid.clone(),
                    });
                    self.status_message = format!(
                        "🔍 Focusing: {} ({})",
                        tt, crate::util::short_id(&psid, 8)
                    );
                    self.log_lines.push(format!(
                        "Focusing tab: {} ({})",
                        tt, crate::util::short_id(&psid, 8)
                    ));
                } else {
                    self.status_message = format!(
                        "⚠ Tab focus not available for {} sessions",
                        pname
                    );
                }
            } else {
                let _ = cmd_tx.send(SupervisorCommand::ResumeSession {
                    provider_session_id: psid.clone(),
                    provider_key: pname,
                    session_cwd: scwd,
                });
                self.status_message = format!(
                    "▶ Resuming: {} ({})",
                    title, crate::util::short_id(&psid, 8)
                );
                self.log_lines.push(format!(
                    "Resuming: {} ({})",
                    title, crate::util::short_id(&psid, 8)
                ));
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent, cmd_tx: &mpsc::UnboundedSender<SupervisorCommand>) {
        // ── Group assignment prompt intercepts all keys ──────────────
        if self.group_prompt.is_some() {
            // Snapshot once — lets us call `self.filter_groups_for_prompt`
            // without the borrow checker yelling about an outstanding mutable
            // borrow into the prompt below.
            let (input, cursor, session_key) = {
                let p = self.group_prompt.as_ref().unwrap();
                (p.input.clone(), p.cursor, p.session_key.clone())
            };
            match key.code {
                KeyCode::Esc => {
                    self.group_prompt = None;
                    return;
                }
                KeyCode::Backspace => {
                    if let Some(p) = self.group_prompt.as_mut() {
                        p.input.pop();
                        p.cursor = 0;
                    }
                    return;
                }
                KeyCode::Char(c) => {
                    if let Some(p) = self.group_prompt.as_mut() {
                        p.input.push(c);
                        p.cursor = 0;
                    }
                    return;
                }
                KeyCode::Up | KeyCode::Left => {
                    if let Some(p) = self.group_prompt.as_mut() {
                        if p.cursor > 0 {
                            p.cursor -= 1;
                        }
                    }
                    return;
                }
                KeyCode::Down | KeyCode::Right => {
                    let filtered_len = self.filter_groups_for_prompt(&input).len();
                    if let Some(p) = self.group_prompt.as_mut() {
                        if filtered_len > 0 && p.cursor + 1 < filtered_len {
                            p.cursor += 1;
                        }
                    }
                    return;
                }
                KeyCode::Enter => {
                    let filtered = self.filter_groups_for_prompt(&input);
                    // Pick the highlighted suggestion when in range; otherwise
                    // fall back to whatever the user typed (which may be a
                    // brand-new group name).
                    let chosen = if !filtered.is_empty() && cursor < filtered.len() {
                        filtered[cursor].0.clone()
                    } else {
                        input.trim().to_string()
                    };
                    if !chosen.is_empty() {
                        self.group_mgr.assign_human(&session_key, &chosen);
                        self.auto_suggestions.remove(&session_key);
                        self.log_lines.push(format!(
                            "Assigned to group '{}': {}",
                            chosen,
                            crate::util::short_id(&session_key, 16)
                        ));
                    }
                    self.group_prompt = None;
                    return;
                }
                _ => return,
            }
        }

        // ── Group edit intercepts all keys ───────────────────────────
        if let Some(ref mut edit) = self.group_edit {
            match key.code {
                KeyCode::Esc => {
                    // Discard and exit
                    self.group_edit = None;
                    return;
                }
                KeyCode::Up | KeyCode::Down | KeyCode::Tab | KeyCode::BackTab => {
                    // Switch between fields
                    edit.field = match edit.field {
                        GroupEditField::Name => GroupEditField::Description,
                        GroupEditField::Description => GroupEditField::Name,
                    };
                    return;
                }
                KeyCode::Enter => {
                    // Save all changes and exit
                    let new_name = edit.name_input.trim().to_string();
                    let old_name = edit.original_name.clone();
                    let desc = edit.desc_input.trim().to_string();

                    // Apply rename if changed
                    let effective_name = if !new_name.is_empty() && new_name != old_name {
                        self.group_mgr.rename_group(&old_name, &new_name);
                        self.log_lines.push(format!("Renamed '{}' → '{}'", old_name, new_name));
                        new_name
                    } else {
                        old_name
                    };

                    // Apply description if non-empty
                    if !desc.is_empty() {
                        self.group_mgr.set_group_description(&effective_name, &desc);
                        self.log_lines.push(format!("Description for '{}': {}", effective_name, desc));
                    }

                    self.group_edit = None;
                    return;
                }
                KeyCode::Backspace => {
                    match edit.field {
                        GroupEditField::Name => { edit.name_input.pop(); }
                        GroupEditField::Description => { edit.desc_input.pop(); }
                    }
                    return;
                }
                KeyCode::Char(c) => {
                    match edit.field {
                        GroupEditField::Name => { edit.name_input.push(c); }
                        GroupEditField::Description => { edit.desc_input.push(c); }
                    }
                    return;
                }
                _ => return,
            }
        }

        // ── ACP suggestion results intercept keys ────────────────────
        if let AcpState::Results { ref suggestions, ref mut cursor } = self.acp_state {
            match key.code {
                KeyCode::Esc => {
                    self.acp_state = AcpState::Idle;
                    return;
                }
                KeyCode::Up | KeyCode::Char('k') if *cursor > 0 => {
                    *cursor -= 1;
                    return;
                }
                KeyCode::Down | KeyCode::Char('j') if *cursor + 1 < suggestions.len() => {
                    *cursor += 1;
                    return;
                }
                KeyCode::Char('y') => {
                    // Accept current suggestion
                    let sg = suggestions[*cursor].clone();
                    self.group_mgr.assign_human(&sg.session, &sg.group);
                    self.log_lines.push(format!(
                        "AI: assigned '{}' → {} ({}%)",
                        crate::util::short_id(&sg.session, 16),
                        sg.group,
                        (sg.score * 100.0) as u32
                    ));
                    // Remove accepted suggestion, stay in Results if more remain
                    let mut sgs = suggestions.clone();
                    sgs.remove(*cursor);
                    if sgs.is_empty() {
                        self.acp_state = AcpState::Idle;
                        self.status_message = "All suggestions processed".to_string();
                    } else {
                        let new_cursor = (*cursor).min(sgs.len() - 1);
                        self.acp_state = AcpState::Results { suggestions: sgs, cursor: new_cursor };
                    }
                    return;
                }
                KeyCode::Char('n') => {
                    // Dismiss current suggestion
                    let sg = suggestions[*cursor].clone();
                    self.group_mgr.dismiss(&sg.session, &sg.group);
                    self.log_lines.push(format!(
                        "AI: dismissed '{}' for {}",
                        sg.group,
                        crate::util::short_id(&sg.session, 16)
                    ));
                    let mut sgs = suggestions.clone();
                    sgs.remove(*cursor);
                    if sgs.is_empty() {
                        self.acp_state = AcpState::Idle;
                        self.status_message = "All suggestions processed".to_string();
                    } else {
                        let new_cursor = (*cursor).min(sgs.len() - 1);
                        self.acp_state = AcpState::Results { suggestions: sgs, cursor: new_cursor };
                    }
                    return;
                }
                KeyCode::Char('e') => {
                    // Edit: open group prompt pre-filled with suggestion name
                    let sg = suggestions[*cursor].clone();
                    self.group_prompt = Some(GroupPrompt {
                        session_key: sg.session.clone(),
                        input: sg.group.clone(),
                        cursor: 0,
                    });
                    // Remove from suggestions list
                    let mut sgs = suggestions.clone();
                    sgs.remove(*cursor);
                    if sgs.is_empty() {
                        self.acp_state = AcpState::Idle;
                    } else {
                        let new_cursor = (*cursor).min(sgs.len() - 1);
                        self.acp_state = AcpState::Results { suggestions: sgs, cursor: new_cursor };
                    }
                    return;
                }
                _ => return,
            }
        }

        // ── ACP running: input lock only applies to MANUAL runs ──────
        // Manual `s` from Grouped view shows the popup and we want a clean
        // wait-for-result UX. Auto-suggest, by contrast, must NOT lock the
        // UI — the whole point is that it runs invisibly in the background.
        if matches!(self.acp_state, AcpState::Running { .. }) && !self.acp_run_is_auto {
            if key.code == KeyCode::Esc {
                self.acp_state = AcpState::Idle;
                self.status_message = "AI suggestion cancelled".to_string();
            }
            return;
        }

        // ── ACP failed: any key returns to idle ──────────────────────
        if matches!(self.acp_state, AcpState::Failed(_)) {
            self.acp_state = AcpState::Idle;
            return;
        }

        // Global shortcuts — always work regardless of mode
        match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('c')) | (_, KeyCode::Char('q'))
                if !self.search_active =>
            {
                self.should_quit = true;
                return;
            }
            (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                // Ctrl+C in search mode: quit
                self.should_quit = true;
                return;
            }
            _ => {}
        }

        // Search mode
        if self.search_active {
            match key.code {
                KeyCode::Esc => {
                    self.search_active = false;
                    self.search_query.clear();
                    self.apply_filter();
                }
                KeyCode::Enter => {
                    // Exit search mode, then open/focus the selected session
                    self.search_active = false;
                    // Reuse the same Enter logic as normal mode
                    self.handle_enter(cmd_tx);
                }
                KeyCode::Tab => {
                    // Switch to detail pane while keeping search results
                    self.focus = Focus::Detail;
                }
                KeyCode::Up
                    if self.selected_index > 0 => {
                        self.selected_index -= 1;
                        self.list_state.select(Some(self.selected_index));
                        self.detail_scroll = 0;
                        self.user_navigated = true;
                }
                KeyCode::Down
                    if self.selected_index + 1 < self.filtered_indices.len() => {
                        self.selected_index += 1;
                        self.list_state.select(Some(self.selected_index));
                        self.detail_scroll = 0;
                        self.user_navigated = true;
                }
                KeyCode::Backspace => {
                    self.search_query.pop();
                    self.apply_filter();
                }
                // While typing a search, if the highlighted row has a pending
                // AI suggestion, allow y/n/e to act on it (rather than being
                // appended to the query). This makes the demo flow:
                //   /regression  →  arrow to a suggested row  →  press y.
                // It only steals these chars when there is actually a
                // suggestion to act on, so typing words like "yarn" still
                // works in the common case.
                KeyCode::Char('y') if self.pending_suggestion_for_selection().is_some() => {
                    if let Some(sg) = self.pending_suggestion_for_selection().cloned() {
                        self.group_mgr.assign_human(&sg.session, &sg.group);
                        self.auto_suggestions.remove(&sg.session);
                        self.log_lines.push(format!(
                            "AI ✓ accepted {} → {} ({}%)",
                            crate::util::short_id(&sg.session, 16),
                            sg.group,
                            (sg.score * 100.0) as u32
                        ));
                        self.status_message = format!("✓ Assigned to '{}'", sg.group);
                    }
                }
                KeyCode::Char('n') if self.pending_suggestion_for_selection().is_some() => {
                    if let Some(sg) = self.pending_suggestion_for_selection().cloned() {
                        self.group_mgr.dismiss(&sg.session, &sg.group);
                        self.auto_suggestions.remove(&sg.session);
                        self.log_lines.push(format!(
                            "AI ✗ dismissed '{}' for {}",
                            sg.group,
                            crate::util::short_id(&sg.session, 16)
                        ));
                        self.status_message = "✗ Dismissed suggestion".to_string();
                    }
                }
                KeyCode::Char('e') if self.pending_suggestion_for_selection().is_some() => {
                    if let Some(sg) = self.pending_suggestion_for_selection().cloned() {
                        self.auto_suggestions.remove(&sg.session);
                        self.group_prompt = Some(GroupPrompt {
                            session_key: sg.session.clone(),
                            input: sg.group.clone(),
                            cursor: 0,
                        });
                        // Pop out of search-typing so the group prompt is
                        // not visually buried by the search title bar.
                        self.search_active = false;
                    }
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                    self.apply_filter();
                }
                _ => {}
            }
            return;
        }

        match self.focus {
            Focus::SessionList => match key.code {
                KeyCode::Esc
                    if !self.search_query.is_empty() => {
                        self.search_query.clear();
                        self.apply_filter();
                }
                KeyCode::Up | KeyCode::Char('k')
                    if self.selected_index > 0 => {
                        self.selected_index -= 1;
                        self.list_state.select(Some(self.selected_index));
                        self.detail_scroll = 0;
                        self.user_navigated = true;
                }
                KeyCode::Down | KeyCode::Char('j')
                    if self.selected_index + 1 < self.filtered_indices.len() => {
                        self.selected_index += 1;
                        self.list_state.select(Some(self.selected_index));
                        self.detail_scroll = 0;
                        self.user_navigated = true;
                }
                KeyCode::Tab => {
                    self.focus = Focus::Detail;
                }
                KeyCode::Char('/') => {
                    self.search_active = true;
                    self.search_query.clear();
                }
                KeyCode::Char('y') if self.pending_suggestion_for_selection().is_some() => {
                    // Accept the inline AI suggestion for the selected session.
                    if let Some(sg) = self.pending_suggestion_for_selection().cloned() {
                        self.group_mgr.assign_human(&sg.session, &sg.group);
                        self.auto_suggestions.remove(&sg.session);
                        self.log_lines.push(format!(
                            "AI ✓ accepted {} → {} ({}%)",
                            crate::util::short_id(&sg.session, 16),
                            sg.group,
                            (sg.score * 100.0) as u32
                        ));
                        self.status_message = format!("✓ Assigned to '{}'", sg.group);
                    }
                }
                KeyCode::Char('n') if self.pending_suggestion_for_selection().is_some() => {
                    // Dismiss the inline AI suggestion (won't re-suggest later).
                    if let Some(sg) = self.pending_suggestion_for_selection().cloned() {
                        self.group_mgr.dismiss(&sg.session, &sg.group);
                        self.auto_suggestions.remove(&sg.session);
                        self.log_lines.push(format!(
                            "AI ✗ dismissed '{}' for {}",
                            sg.group,
                            crate::util::short_id(&sg.session, 16)
                        ));
                        self.status_message = "✗ Dismissed suggestion".to_string();
                    }
                }
                KeyCode::Char('e') if self.pending_suggestion_for_selection().is_some() => {
                    // Edit: open the group prompt pre-filled with the suggestion.
                    if let Some(sg) = self.pending_suggestion_for_selection().cloned() {
                        self.auto_suggestions.remove(&sg.session);
                        self.group_prompt = Some(GroupPrompt {
                            session_key: sg.session.clone(),
                            input: sg.group.clone(),
                            cursor: 0,
                        });
                    }
                }
                KeyCode::Char('n') => {
                    let key = self.default_provider.clone();
                    if self.provider_keys.contains(&key) {
                        let cwd = std::env::current_dir()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();
                        let _ = cmd_tx.send(SupervisorCommand::NewSession {
                            provider_key: key.clone(),
                            cwd,
                        });
                        self.log_lines
                            .push(format!("Launching new {} session...", key));
                    }
                }
                KeyCode::Char(ch) if self.shortcut_map.contains_key(&ch) => {
                    let key = self.shortcut_map[&ch].clone();
                    if self.provider_keys.contains(&key) {
                        let cwd = std::env::current_dir()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();
                        let _ = cmd_tx.send(SupervisorCommand::NewSession {
                            provider_key: key.clone(),
                            cwd,
                        });
                        self.log_lines
                            .push(format!("Launching new {} session...", key));
                    }
                }
                KeyCode::Enter => {
                    self.handle_enter(cmd_tx);
                }
                KeyCode::Char('g') if self.group_prompt.is_none() => {
                    // Open group-assignment prompt for the selected session.
                    if let Some(session) = self.selected_session() {
                        let session_key = format!(
                            "{}:{}",
                            session.provider_name, session.provider_session_id
                        );
                        self.group_prompt = Some(GroupPrompt {
                            session_key,
                            input: String::new(),
                            cursor: 0,
                        });
                    }
                }
                KeyCode::Char('u') => {
                    // Unassign: remove selected session from its first group.
                    if let Some(session) = self.selected_session() {
                        let session_key = format!(
                            "{}:{}",
                            session.provider_name, session.provider_session_id
                        );
                        let groups = self.group_mgr.groups_for(&session_key);
                        if let Some(first) = groups.first() {
                            let g = first.clone();
                            self.group_mgr.unassign(&session_key, &g);
                            self.log_lines.push(format!(
                                "Unassigned from '{}': {}",
                                g,
                                crate::util::short_id(&session_key, 16)
                            ));
                        }
                    }
                }
                KeyCode::Char('e') if matches!(self.view_mode, ViewMode::Grouped) => {
                    // Edit group: immediately editable, no menu
                    let sel = self.list_state.selected().unwrap_or(0);
                    if let Some((_idx, group_name)) = self.grouped_header_names.iter().find(|(idx, _)| *idx == sel) {
                        let desc = self.group_mgr.get_group_description(group_name)
                            .unwrap_or_default();
                        self.group_edit = Some(GroupEditPrompt {
                            original_name: group_name.clone(),
                            field: GroupEditField::Name,
                            name_input: group_name.clone(),
                            desc_input: desc,
                        });
                    }
                }
                KeyCode::Char('s') if matches!(self.view_mode, ViewMode::Grouped) => {
                    if !self.acp_available {
                        self.status_message = "AI grouping not set up — see README → AI Auto-Suggest".to_string();
                        return;
                    }
                    // AI suggest: prepare prompt and spawn ACP background task
                    // Try to get semantic similarities (non-blocking try_lock)
                    let sem_ref = self.semantic.try_lock().ok();
                    let sem_plugin = sem_ref.as_deref();
                    // Manual `s` resets the ask-history so the user gets a
                    // fresh batch starting from the top of the ungrouped list.
                    self.auto_suggest_asked.clear();
                    match crate::acp::prepare_prompt(
                        &self.acp_config,
                        &self.sessions,
                        &self.group_mgr,
                        sem_plugin,
                        &self.auto_suggest_asked,
                    ) {
                        Ok((prompt, asked_keys)) => {
                            let count = asked_keys.len();
                            for k in &asked_keys {
                                self.auto_suggest_asked.insert(k.clone());
                            }
                            self.acp_state = AcpState::Running {
                                started: std::time::Instant::now(),
                            };
                            self.acp_run_is_auto = false;
                            self.status_message = format!("🤖 Analyzing {} sessions...", count);
                            self.log_lines.push(format!("AI: sending {} ungrouped sessions to ACP agent", count));

                            // Spawn background task with timeout
                            let cfg = self.acp_config.clone();
                            let event_tx = cmd_tx.clone();
                            // Pre-archive the ACP session UUID before spawn
                            // — see auto-suggest path for rationale.
                            let acp_session_id = uuid::Uuid::new_v4().to_string();
                            let _ = cmd_tx.send(SupervisorCommand::ArchiveSession {
                                provider_session_id: acp_session_id.clone(),
                                provider_key: "copilot".to_string(),
                            });
                            crate::log::info(&format!(
                                "AI manual suggest: pre-archived copilot:{} (ACP session)",
                                acp_session_id
                            ));
                            tokio::spawn(async move {
                                let timeout_secs = cfg.timeout_secs;
                                let timeout = tokio::time::Duration::from_secs(timeout_secs);
                                let result = tokio::time::timeout(
                                    timeout,
                                    crate::acp::run_acp_suggest(cfg, prompt, acp_session_id),
                                ).await;
                                let _ = match result {
                                    Ok(Ok(suggestions)) => {
                                        let json = serde_json::to_string(&suggestions).unwrap_or_default();
                                        event_tx.send(SupervisorCommand::AcpResult(json))
                                    }
                                    Ok(Err(e)) => {
                                        event_tx.send(SupervisorCommand::AcpError(e))
                                    }
                                    Err(_) => {
                                        crate::log::warn(&format!(
                                            "AI manual suggest TIMED OUT after {}s",
                                            timeout_secs
                                        ));
                                        event_tx.send(SupervisorCommand::AcpError(format!(
                                            "ACP agent timed out after {}s",
                                            timeout_secs
                                        )))
                                    }
                                };
                            });
                        }
                        Err(e) => {
                            self.status_message = format!("⚠ {}", e);
                        }
                    }
                }
                KeyCode::Char('a') => {
                    if let Some(session) = self.selected_session() {
                        let psid = session.provider_session_id.clone();
                        let pname = session.provider_name.clone();
                        let key = format!("{}:{}", pname, psid);
                        match self.view_mode {
                            ViewMode::Active | ViewMode::Grouped => {
                                let _ = cmd_tx.send(SupervisorCommand::ArchiveSession {
                                    provider_session_id: psid.clone(),
                                    provider_key: pname.clone(),
                                });
                                // Track locally so incoming refreshes don't put it back
                                self.pending_archives.push(PendingTransition { key, confirmed: false });
                                // Resolve the actual session index (Grouped view has header rows)
                                let session_idx_opt = if self.view_mode == ViewMode::Grouped && !self.grouped_row_map.is_empty() {
                                    self.grouped_row_map.get(self.selected_index).and_then(|o| *o)
                                } else {
                                    self.filtered_indices.get(self.selected_index).copied()
                                };
                                // Instantly move from active to hidden
                                if let Some(idx) = session_idx_opt {
                                    if idx < self.sessions.len() {
                                        let removed = self.sessions.remove(idx);
                                        self.hidden_sessions.insert(0, removed);
                                        // Preserve cursor at the same visual position
                                        // so the next row slides up under it — this
                                        // enables rapid repeat-archive. apply_filter()
                                        // zeroes the selection, so capture and
                                        // restore here via `clamp_cursor_after_removal`.
                                        let prev = self.selected_index;
                                        self.apply_filter();
                                        match clamp_cursor_after_removal(
                                            prev,
                                            self.filtered_indices.len(),
                                        ) {
                                            Some(idx) => {
                                                self.selected_index = idx;
                                                self.list_state.select(Some(idx));
                                            }
                                            None => {
                                                self.selected_index = 0;
                                                self.list_state.select(None);
                                            }
                                        }
                                    }
                                }
                                self.log_lines
                                    .push(format!("Archived: {}", crate::util::short_id(&psid, 8)));
                            }
                            ViewMode::Hidden => {
                                // 'a' in the archived view restores the
                                // session — symmetric to archive. Mirror
                                // the same local-update + pending-key
                                // tracking pattern so rapid repeat works.
                                let _ = cmd_tx.send(SupervisorCommand::UnarchiveSession {
                                    provider_session_id: psid.clone(),
                                    provider_key: pname.clone(),
                                });
                                self.pending_unarchives.push(PendingTransition { key, confirmed: false });
                                if let Some(&idx) = self.filtered_indices.get(self.selected_index) {
                                    if idx < self.hidden_sessions.len() {
                                        let removed = self.hidden_sessions.remove(idx);
                                        self.sessions.insert(0, removed);
                                        let prev = self.selected_index;
                                        self.apply_filter();
                                        match clamp_cursor_after_removal(
                                            prev,
                                            self.filtered_indices.len(),
                                        ) {
                                            Some(idx) => {
                                                self.selected_index = idx;
                                                self.list_state.select(Some(idx));
                                            }
                                            None => {
                                                self.selected_index = 0;
                                                self.list_state.select(None);
                                            }
                                        }
                                    }
                                }
                                self.log_lines
                                    .push(format!("Unarchived: {}", crate::util::short_id(&psid, 8)));
                            }
                        }
                    }
                }
                KeyCode::BackTab => {
                    // Shift+Tab: cycle Active → Grouped → Hidden → Active
                    self.view_mode = match self.view_mode {
                        ViewMode::Active => ViewMode::Grouped,
                        ViewMode::Grouped => ViewMode::Hidden,
                        ViewMode::Hidden => ViewMode::Active,
                    };
                    // Capture a stable group order on entering Grouped view;
                    // clear it on leaving so the next entry recomputes fresh.
                    if self.view_mode == ViewMode::Grouped {
                        self.grouped_view_sort_order = Some(self.compute_group_sort_order());
                    } else {
                        self.grouped_view_sort_order = None;
                    }
                    self.selected_index = 0;
                    self.list_state.select(Some(0));
                    self.search_query.clear();
                    self.apply_filter();
                    self.log_lines.push(format!(
                        "View: {}",
                        match self.view_mode {
                            ViewMode::Active => "Active sessions",
                            ViewMode::Grouped => "Grouped sessions",
                            ViewMode::Hidden => "Archived & hidden sessions",
                        }
                    ));
                }
                _ => {}
            },
            Focus::Detail => match key.code {
                KeyCode::Tab => {
                    self.focus = Focus::Logs;
                }
                KeyCode::BackTab => {
                    self.focus = Focus::SessionList;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.detail_scroll = self.detail_scroll.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.detail_scroll = self.detail_scroll.saturating_add(1);
                }
                KeyCode::PageUp => {
                    self.detail_scroll = self.detail_scroll.saturating_sub(20);
                }
                KeyCode::PageDown => {
                    self.detail_scroll = self.detail_scroll.saturating_add(20);
                }
                KeyCode::Home => {
                    self.detail_scroll = 0;
                }
                KeyCode::End => {
                    self.detail_scroll = u16::MAX; // capped during render
                }
                _ => {}
            },
            Focus::Logs => match key.code {
                KeyCode::Tab | KeyCode::BackTab => {
                    self.focus = Focus::SessionList;
                }
                KeyCode::Up => {
                    self.log_scroll = self.log_scroll.saturating_sub(1);
                }
                KeyCode::Down
                    if self.log_scroll + 1 < self.log_lines.len() => {
                        self.log_scroll += 1;
                }
                _ => {}
            },
        }
    }

    fn draw(&mut self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Title bar
                Constraint::Min(10),   // Main area
                Constraint::Length(8), // Log viewer
                Constraint::Length(1), // Status bar
            ])
            .split(f.area());

        // Title bar
        self.draw_title_bar(f, chunks[0]);

        // Main area: session list | detail
        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
            .split(chunks[1]);

        self.draw_session_list(f, main_chunks[0]);
        self.draw_session_detail(f, main_chunks[1]);

        // Log viewer
        self.draw_log_viewer(f, chunks[2]);

        // Status bar
        self.draw_status_bar(f, chunks[3]);

        // Group assignment prompt overlay (renders over status bar area)
        if let Some(ref prompt) = self.group_prompt {
            let filtered = self.filter_groups_for_prompt(&prompt.input);
            let mut spans: Vec<Span<'static>> = vec![
                Span::styled(
                    " Group: ",
                    Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("{}▏", prompt.input)),
            ];
            if filtered.is_empty() {
                let new_hint = if prompt.input.trim().is_empty() {
                    "  ←→ pick · ⏎ assign · Esc cancel".to_string()
                } else {
                    format!(
                        "  (⏎ creates new group '{}') · Esc cancel",
                        prompt.input
                    )
                };
                spans.push(Span::styled(new_hint, Style::default().fg(Color::DarkGray)));
            } else {
                // Sliding window: show up to 5 pills centered around the cursor.
                // Caller (filter_groups_for_prompt) returns the full filtered
                // list now, so the cursor can navigate the entire group list,
                // not just the first 5.
                const WINDOW: usize = 5;
                let total = filtered.len();
                let cursor = prompt.cursor.min(total.saturating_sub(1));
                // Center the window on the cursor when possible.
                let half = WINDOW / 2;
                let start = cursor.saturating_sub(half);
                let end = (start + WINDOW).min(total);
                // Re-anchor `start` so the window is full when at the right edge.
                let start = end.saturating_sub(WINDOW);

                if start > 0 {
                    spans.push(Span::styled(
                        format!("  «{} ", start),
                        Style::default().fg(Color::DarkGray),
                    ));
                } else {
                    spans.push(Span::raw("  "));
                }
                for (i, (name, count)) in filtered.iter().enumerate().take(end).skip(start) {
                    if i > start {
                        spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
                    }
                    if i == cursor {
                        spans.push(Span::styled(
                            format!("▸ {} ({})", name, count),
                            Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD),
                        ));
                    } else {
                        spans.push(Span::styled(
                            format!("{} ({})", name, count),
                            Style::default().fg(Color::Gray),
                        ));
                    }
                }
                if end < total {
                    spans.push(Span::styled(
                        format!(" {}»", total - end),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                spans.push(Span::styled(
                    format!("   {}/{}  ←→ pick · ⏎ assign · Esc cancel", cursor + 1, total),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            let prompt_text = Paragraph::new(Line::from(spans))
                .style(Style::default().bg(Color::Black).fg(Color::White));
            f.render_widget(prompt_text, chunks[3]);
        }

    }

    fn draw_title_bar(&self, f: &mut Frame, area: Rect) {
        let hl = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);

        if self.group_prompt.is_some() {
            // Group assignment prompt mode
            let title = Paragraph::new(Line::from(vec![
                Span::styled(" Assign Group ", Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::raw("  type name  "),
                Span::styled("⏎", hl),
                Span::raw(" assign  "),
                Span::styled("↑↓", hl),
                Span::raw(" pick existing  "),
                Span::styled("Esc", hl),
                Span::raw(" cancel"),
            ]));
            f.render_widget(title, area);
        } else if let Some(ref ge) = self.group_edit {
            let field_label = match ge.field {
                GroupEditField::Name => "Name",
                GroupEditField::Description => "Description",
            };
            let title = Paragraph::new(Line::from(vec![
                Span::styled(" ✏ Edit Group ", Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw(format!("  [{}]  editing: {}  ", ge.original_name, field_label)),
                Span::styled("↑↓", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::raw(" switch  "),
                Span::styled("⏎", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::raw(" save  "),
                Span::styled("Esc", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::raw(" cancel"),
            ]));
            f.render_widget(title, area);
        } else if let AcpState::Running { started } = &self.acp_state {
            if !self.acp_run_is_auto {
                // Manual `s` run — full takeover with cancel hint.
                let elapsed = started.elapsed().as_secs();
                let spinner = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
                let frame = spinner[(elapsed as usize) % spinner.len()];
                let title = Paragraph::new(Line::from(vec![
                    Span::styled(" 🤖 AI Analyzing ", Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    Span::raw(format!("  {} {}s  ", frame, elapsed)),
                    Span::styled("Esc", hl),
                    Span::raw(" cancel"),
                ]));
                f.render_widget(title, area);
            } else {
                // Auto-suggest run — DO NOT take over the title bar. Drop
                // through to the normal per-view title so the user keeps
                // navigating; the spinner shows in the status bar instead.
                self.draw_normal_title_bar(f, area);
            }
        } else if let AcpState::Results { suggestions, cursor } = &self.acp_state {
            let title = Paragraph::new(Line::from(vec![
                Span::styled(" 🤖 AI Suggestions ", Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::raw(format!("  {}/{} ", cursor + 1, suggestions.len())),
                Span::styled("y", hl),
                Span::raw(" accept  "),
                Span::styled("n", hl),
                Span::raw(" dismiss  "),
                Span::styled("e", hl),
                Span::raw(" edit  "),
                Span::styled("↑↓", hl),
                Span::raw(" nav  "),
                Span::styled("Esc", hl),
                Span::raw(" done"),
            ]));
            f.render_widget(title, area);
        } else if let AcpState::Failed(ref msg) = self.acp_state {
            let display = if msg.len() > 60 { &msg[..60] } else { msg.as_str() };
            let title = Paragraph::new(Line::from(vec![
                Span::styled(" ⚠ AI Error ", Style::default().fg(Color::Black).bg(Color::Red).add_modifier(Modifier::BOLD)),
                Span::raw(format!("  {}  ", display)),
                Span::raw("press any key"),
            ]));
            f.render_widget(title, area);
        } else if self.search_active {
            // If the highlighted row has a pending AI suggestion, surface
            // the y/n/e actions in the title bar — same as normal mode —
            // so the user knows they can act on it without leaving search.
            if let Some(sg) = self.pending_suggestion_for_selection().cloned() {
                let pct = (sg.score * 100.0) as u32;
                let title = Paragraph::new(Line::from(vec![
                    Span::styled(
                        " 🤖 Suggestion ",
                        Style::default().fg(Color::Black).bg(Color::LightCyan).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(format!("  → {} ({}%)  ", sg.group, pct)),
                    Span::styled("y", hl),
                    Span::raw(" accept  "),
                    Span::styled("n", hl),
                    Span::raw(" dismiss  "),
                    Span::styled("e", hl),
                    Span::raw(" edit  "),
                    Span::styled("⏎", hl),
                    Span::raw(" open  "),
                    Span::styled("Esc", hl),
                    Span::raw(" quit search"),
                ]));
                f.render_widget(title, area);
            } else {
                let title = Paragraph::new(Line::from(vec![
                    Span::styled(" Search ", Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    Span::raw("  "),
                    Span::styled("⏎", hl),
                    Span::raw(" open  "),
                    Span::styled("Tab", hl),
                    Span::raw(" detail  "),
                    Span::styled("↑↓", hl),
                    Span::raw(" nav  "),
                    Span::styled("Esc", hl),
                    Span::raw(" quit search"),
                ]));
                f.render_widget(title, area);
            }
        } else {
            self.draw_normal_title_bar(f, area);
        }
    }

    /// Per-view title bar (Active / Grouped / Hidden) — also used as the
    /// fall-through when an auto AI run is in flight, so the user keeps
    /// their normal navigation hints instead of an opaque "AI Analyzing"
    /// banner.
    fn draw_normal_title_bar(&self, f: &mut Frame, area: Rect) {
        let hl = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
        match self.view_mode {
            ViewMode::Active => {
                let has_suggestion = self.pending_suggestion_for_selection().is_some();
                if has_suggestion {
                    // Cursor is on a session with a pending AI suggestion —
                    // surface the y/n/e actions front-and-centre.
                    let sg = self.pending_suggestion_for_selection().unwrap().clone();
                    let pct = (sg.score * 100.0) as u32;
                    let title = Paragraph::new(Line::from(vec![
                        Span::styled(
                            " 🤖 Suggestion ",
                            Style::default().fg(Color::Black).bg(Color::LightCyan).add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(format!("  → {} ({}%)  ", sg.group, pct)),
                        Span::styled("y", hl),
                        Span::raw(" accept  "),
                        Span::styled("n", hl),
                        Span::raw(" dismiss  "),
                        Span::styled("e", hl),
                        Span::raw(" edit  "),
                        Span::styled("⏎", hl),
                        Span::raw(" open  "),
                        Span::styled("/", hl),
                        Span::raw("search  "),
                        Span::styled("q", hl),
                        Span::raw("uit"),
                    ]));
                    f.render_widget(title, area);
                } else {
                    let suggestion_count = self.auto_suggestions.len();
                    let auto_running = matches!(self.acp_state, AcpState::Running { .. })
                        && self.acp_run_is_auto;
                    let header_label = if auto_running {
                        let elapsed = if let AcpState::Running { started } = &self.acp_state {
                            started.elapsed().as_secs()
                        } else {
                            0
                        };
                        let spinner = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
                        let frame = spinner[(elapsed as usize) % spinner.len()];
                        format!(" Agent Session Manager · 🤖 analyzing {} {}s ", frame, elapsed)
                    } else if suggestion_count > 0 {
                        format!(" Agent Session Manager · 🤖 {} ", suggestion_count)
                    } else {
                        " Agent Session Manager ".to_string()
                    };
                    let mut spans = vec![
                        Span::styled(header_label, Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
                        Span::raw("  "),
                        Span::styled("⏎", hl),
                        Span::raw(" open  "),
                        Span::styled("n", hl),
                        Span::raw("ew  "),
                    ];
                    let mut shortcuts: Vec<_> = self.shortcut_map.iter().collect();
                    shortcuts.sort_by_key(|(ch, _)| *ch);
                    for (ch, key) in &shortcuts {
                        spans.push(Span::styled(ch.to_string(), hl));
                        spans.push(Span::raw(format!(" {} ", key)));
                    }
                    spans.extend_from_slice(&[
                        Span::styled("g", hl),
                        Span::raw("roup  "),
                        Span::styled("a", hl),
                        Span::raw("rchive  "),
                        Span::styled("/", hl),
                        Span::raw("search  "),
                        Span::styled("q", hl),
                        Span::raw("uit"),
                    ]);
                    let title = Paragraph::new(Line::from(spans));
                    f.render_widget(title, area);
                }
            }
            ViewMode::Grouped => {
                let title = Paragraph::new(Line::from({
                    let mut spans = vec![
                        Span::styled(" 📂 Grouped ", Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)),
                        Span::raw("  "),
                        Span::styled("⏎", hl),
                        Span::raw(" open  "),
                        Span::styled("g", hl),
                        Span::raw("roup  "),
                        Span::styled("u", hl),
                        Span::raw("nassign  "),
                    ];
                    if self.acp_available {
                        spans.push(Span::styled("s", hl));
                        spans.push(Span::raw(" AI suggest  "));
                    }
                    spans.extend([
                        Span::styled("e", hl),
                        Span::raw("dit group  "),
                        Span::styled("a", hl),
                        Span::raw("rchive  "),
                        Span::styled("/", hl),
                        Span::raw("search  "),
                        Span::styled("q", hl),
                        Span::raw("uit"),
                    ]);
                    spans
                }));
                f.render_widget(title, area);
            }
            ViewMode::Hidden => {
                let title = Paragraph::new(Line::from(vec![
                    Span::styled(" 📦 Archived ", Style::default().fg(Color::Black).bg(Color::Magenta).add_modifier(Modifier::BOLD)),
                    Span::raw("  "),
                    Span::styled("⏎", hl),
                    Span::raw(" open  "),
                    Span::styled("a", hl),
                    Span::raw(" unarchive  "),
                    Span::styled("/", hl),
                    Span::raw("search  "),
                    Span::styled("q", hl),
                    Span::raw("uit"),
                ]));
                f.render_widget(title, area);
            }
        }
    }

    fn build_session_item(&self, list_idx: usize, session_idx: usize, show_badges: bool) -> ListItem<'static> {
        let s = &self.current_view_sessions()[session_idx];
        let badge = s.state.badge();
        let age = format_age(&s.updated_at);
        let short_id = crate::util::short_id(&s.provider_session_id, 8);

        let title_display = if s.title.is_empty() {
            short_id.to_string()
        } else {
            truncate_str_safe(&s.title, 25)
        };

        let sem_icon = if self.semantic_matches.contains(&session_idx) {
            "✨"
        } else {
            ""
        };

        let line = Line::from(vec![
            Span::raw(format!("{} ", badge)),
            Span::styled(
                format!("{:<6}", s.provider_name),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw(" "),
            Span::styled(
                title_display,
                if list_idx == self.selected_index {
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray)
                },
            ),
            Span::styled(
                format!(" {}", sem_icon),
                Style::default().fg(Color::Magenta),
            ),
        ]);

        let session_key = format!("{}:{}", s.provider_name, s.provider_session_id);
        let group_names = self.group_mgr.groups_for(&session_key);
        let group_badge_str = if !show_badges || group_names.is_empty() {
            String::new()
        } else {
            format!("  {}", group_names.iter().map(|g| format!("[{}]", g)).collect::<Vec<_>>().join(" "))
        };

        // Inline AI suggestion — shown whenever a pending auto-suggestion
        // exists for this session, regardless of current group membership.
        // This makes the "🤖 N" badge findable: the user can spot the blue
        // arrow line directly in the list.
        let suggestion = if show_badges {
            self.auto_suggestions.get(&session_key)
        } else {
            None
        };

        let mut age_spans: Vec<Span<'static>> = vec![
            Span::raw("   "),
            Span::styled(
                format!("{} · {}", s.state.label(), age),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                group_badge_str,
                Style::default().fg(Color::Blue),
            ),
        ];
        if let Some(sg) = suggestion {
            // "Shadow" rendering — visibly dimmer than the title but bright
            // enough to read on dark terminals without italic (which not
            // every terminal renders). Communicates "this group is tentative,
            // not yet committed".
            let new_marker = if sg.is_new { "✨ " } else { "" };
            age_spans.push(Span::styled(
                format!("  · ⟨{}{}⟩", new_marker, sg.group),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::DIM),
            ));
        }
        let age_line = Line::from(age_spans);

        ListItem::new(vec![line, age_line])
    }

    /// Compute the freeze-sort order for groups: sort by maximum member
    /// activity (most recently touched group first), name ascending as a
    /// stable tiebreak. Snapshotted on entry to grouped view; refreshed
    /// implicitly when the user leaves and re-enters the view.
    ///
    /// updated_at is an ISO-8601 string; lexical comparison is chronological
    /// for that format, so we compare as strings to avoid a parse step.
    fn compute_group_sort_order(&self) -> Vec<String> {
        use std::collections::HashMap;

        let mut group_latest: HashMap<String, String> = HashMap::new();
        for s in &self.sessions {
            let key = format!("{}:{}", s.provider_name, s.provider_session_id);
            for g in self.group_mgr.groups_for(&key) {
                let entry = group_latest.entry(g).or_default();
                if s.updated_at > *entry {
                    *entry = s.updated_at.clone();
                }
            }
        }

        // Include any group that exists in the manager but has no members yet,
        // so newly-created empty groups still appear (sorted last by activity).
        for g in self.group_mgr.all_groups().into_iter().map(|(n, _)| n) {
            group_latest.entry(g).or_default();
        }

        let mut pairs: Vec<(String, String)> = group_latest.into_iter().collect();
        pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        pairs.into_iter().map(|(n, _)| n).collect()
    }

    fn build_flat_list_items(&self) -> Vec<ListItem<'static>> {
        self.filtered_indices
            .iter()
            .enumerate()
            .map(|(list_idx, &session_idx)| self.build_session_item(list_idx, session_idx, true))
            .collect()
    }

    fn build_grouped_list_items(&mut self) -> Vec<ListItem<'static>> {
        // Collect sessions by group. A session can appear under multiple groups.
        let sessions = self.current_view_sessions();
        let mut grouped: std::collections::HashMap<String, Vec<usize>> = std::collections::HashMap::new();
        let mut ungrouped_indices: Vec<usize> = Vec::new();

        for &session_idx in &self.filtered_indices {
            let s = &sessions[session_idx];
            let key = format!("{}:{}", s.provider_name, s.provider_session_id);
            let groups = self.group_mgr.groups_for(&key);
            if groups.is_empty() {
                ungrouped_indices.push(session_idx);
            } else {
                for g in groups {
                    grouped.entry(g).or_default().push(session_idx);
                }
            }
        }

        // Determine iteration order. Prefer the frozen snapshot captured on
        // entering the grouped view (most-recently-active group first); fall
        // back to a fresh compute if the snapshot is missing.
        let snapshot: Vec<String> = match &self.grouped_view_sort_order {
            Some(order) => order.clone(),
            None => self.compute_group_sort_order(),
        };
        // Build ordered list of (group_name, members), keeping only groups
        // that have visible members AND respecting the snapshot order. Any
        // groups in `grouped` but not in the snapshot (e.g. brand-new) are
        // appended at the end, alphabetically.
        let mut ordered: Vec<(String, Vec<usize>)> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for name in &snapshot {
            if let Some(members) = grouped.remove(name) {
                ordered.push((name.clone(), members));
                seen.insert(name.clone());
            }
        }
        let mut leftover: Vec<(String, Vec<usize>)> = grouped.into_iter().collect();
        leftover.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, members) in leftover {
            if !seen.contains(&name) {
                ordered.push((name, members));
            }
        }

        let mut items: Vec<ListItem<'static>> = Vec::new();
        let mut row_map: Vec<Option<usize>> = Vec::new();
        let mut header_names: Vec<(usize, String)> = Vec::new();
        let mut visual_idx = 0usize;

        // Render each group with a header
        for (group_name, session_indices) in &ordered {
            // Group header
            let header = Line::from(vec![
                Span::styled(
                    format!("▼ {} ({})", group_name, session_indices.len()),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ),
            ]);
            items.push(ListItem::new(vec![header]));
            row_map.push(None);
            header_names.push((visual_idx, group_name.clone()));
            visual_idx += 1;

            // Sessions under this group (indented)
            for &session_idx in session_indices {
                let s = &sessions[session_idx];
                let badge = s.state.badge();
                let age = format_age(&s.updated_at);
                let title_display = if s.title.is_empty() {
                    crate::util::short_id(&s.provider_session_id, 8).to_string()
                } else {
                    truncate_str_safe(&s.title, 23)
                };

                let line = Line::from(vec![
                    Span::raw("  "),
                    Span::raw(format!("{} ", badge)),
                    Span::styled(
                        format!("{:<6}", s.provider_name),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        title_display,
                        if visual_idx == self.selected_index {
                            Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::Gray)
                        },
                    ),
                ]);
                let age_line = Line::from(vec![
                    Span::raw("     "),
                    Span::styled(
                        format!("{} · {}", s.state.label(), age),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]);
                items.push(ListItem::new(vec![line, age_line]));
                row_map.push(Some(session_idx));
                visual_idx += 1;
            }
        }

        // Ungrouped section
        if !ungrouped_indices.is_empty() {
            let header = Line::from(vec![
                Span::styled(
                    format!("▼ Ungrouped ({})", ungrouped_indices.len()),
                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
                ),
            ]);
            items.push(ListItem::new(vec![header]));
            row_map.push(None);
            visual_idx += 1;

            for &session_idx in &ungrouped_indices {
                let s = &sessions[session_idx];
                let badge = s.state.badge();
                let age = format_age(&s.updated_at);
                let title_display = if s.title.is_empty() {
                    crate::util::short_id(&s.provider_session_id, 8).to_string()
                } else {
                    truncate_str_safe(&s.title, 23)
                };

                let line = Line::from(vec![
                    Span::raw("  "),
                    Span::raw(format!("{} ", badge)),
                    Span::styled(
                        format!("{:<6}", s.provider_name),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        title_display,
                        if visual_idx == self.selected_index {
                            Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::Gray)
                        },
                    ),
                ]);
                let age_line = Line::from(vec![
                    Span::raw("     "),
                    Span::styled(
                        format!("{} · {}", s.state.label(), age),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]);
                items.push(ListItem::new(vec![line, age_line]));
                row_map.push(Some(session_idx));
                visual_idx += 1;
            }
        }

        self.grouped_row_map = row_map;
        self.grouped_header_names = header_names;

        // Resolve deferred selection restore after row_map is rebuilt
        if let Some((ref prov, ref sid)) = self.pending_restore_selection.take() {
            let sessions = self.current_view_sessions();
            for (visual_idx, entry) in self.grouped_row_map.iter().enumerate() {
                if let Some(session_idx) = entry {
                    let s = &sessions[*session_idx];
                    if s.provider_name == *prov && s.provider_session_id == *sid {
                        self.selected_index = visual_idx;
                        self.list_state.select(Some(visual_idx));
                        break;
                    }
                }
            }
        }

        items
    }

    fn draw_session_list(&mut self, f: &mut Frame, area: Rect) {
        // When AI suggestions are showing, render them instead of sessions
        if let AcpState::Results { ref suggestions, ref cursor } = self.acp_state.clone() {
            self.draw_suggestion_list(f, area, suggestions, *cursor);
            return;
        }

        let items: Vec<ListItem> = if self.view_mode == ViewMode::Grouped {
            self.build_grouped_list_items()
        } else {
            self.build_flat_list_items()
        };

        let border_style = if self.focus == Focus::SessionList || self.search_active {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let view_label = match self.view_mode {
            ViewMode::Active => "Sessions",
            ViewMode::Grouped => "📂 Grouped",
            ViewMode::Hidden => "📦 Archived & Hidden",
        };
        let view_count = self.current_view_sessions().len();

        // In Grouped view, show grouped/total counts
        let grouped_count = if self.view_mode == ViewMode::Grouped {
            let sessions = self.current_view_sessions();
            sessions.iter().filter(|s| {
                let key = format!("{}:{}", s.provider_name, s.provider_session_id);
                !self.group_mgr.groups_for(&key).is_empty()
            }).count()
        } else {
            0
        };

        let title = if self.search_active {
            format!(" Search: {}▌ ", self.search_query)
        } else if !self.search_query.is_empty() {
            format!(
                " {} ({}/{}) [{}] ",
                view_label,
                self.filtered_indices.len(),
                view_count,
                self.search_query
            )
        } else if self.view_mode == ViewMode::Grouped {
            format!(" {} ({} grouped / {} total) ", view_label, grouped_count, view_count)
        } else {
            format!(" {} ({}) ", view_label, self.filtered_indices.len())
        };

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(border_style)
                    .title(title),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .scroll_padding(2); // Keep 2 items visible above/below selection before scrolling

        f.render_stateful_widget(list, area, &mut self.list_state);
    }

    fn draw_suggestion_list(
        &self,
        f: &mut Frame,
        area: Rect,
        suggestions: &[AiSuggestion],
        cursor: usize,
    ) {
        // Compact: one line per suggestion — title → group (score%)
        let mut items: Vec<ListItem> = suggestions
            .iter()
            .enumerate()
            .map(|(i, sg)| {
                let is_selected = i == cursor;
                let new_badge = if sg.is_new { "✨" } else { "" };
                let score_pct = (sg.score * 100.0) as u32;

                // Try to find session title from self.sessions
                let title = self.sessions.iter()
                    .find(|s| {
                        let key = format!("{}:{}", s.provider_name, s.provider_session_id);
                        key == sg.session
                    })
                    .map(|s| truncate_str_safe(&s.title, 20))
                    .unwrap_or_else(|| crate::util::short_id(&sg.session, 12).to_string());

                let line = Line::from(vec![
                    Span::styled(
                        if is_selected { " ▸ " } else { "   " },
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::styled(
                        title,
                        if is_selected {
                            Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::Gray)
                        },
                    ),
                    Span::raw(" → "),
                    Span::styled(
                        format!("{}{}", new_badge, sg.group),
                        Style::default().fg(Color::Green),
                    ),
                    Span::styled(
                        format!(" {}%", score_pct),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]);

                ListItem::new(vec![line])
            })
            .collect();

        // Pad items to fill the entire visible area (prevents ghost artifacts from prior frames)
        let inner_height = area.height.saturating_sub(2) as usize; // subtract border top+bottom
        while items.len() < inner_height {
            items.push(ListItem::new(Line::from("")));
        }

        let mut list_state = ListState::default();
        list_state.select(Some(cursor));

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Green))
                    .title(format!(" 🤖 AI Suggestions ({}) ", suggestions.len())),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .scroll_padding(2);

        f.render_stateful_widget(list, area, &mut list_state);
    }

    fn draw_session_detail(&self, f: &mut Frame, area: Rect) {
        let border_style = if self.focus == Focus::Detail {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        // When group edit is active, render the edit form in the detail pane
        if let Some(ref ge) = self.group_edit {
            let mut lines = vec![];
            let name_focused = ge.field == GroupEditField::Name;
            let desc_focused = ge.field == GroupEditField::Description;

            lines.push(Line::from(""));

            // Name field — always shows input, cursor on focused field
            let name_label_style = if name_focused {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            lines.push(Line::from(vec![
                Span::styled(
                    if name_focused { " ✎ " } else { "   " },
                    name_label_style,
                ),
                Span::styled("Name: ", name_label_style),
                Span::styled(
                    if name_focused {
                        format!("{}▏", ge.name_input)
                    } else {
                        ge.name_input.clone()
                    },
                    if name_focused {
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    },
                ),
            ]));

            lines.push(Line::from(""));

            // Description field
            let desc_label_style = if desc_focused {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let desc_display = if ge.desc_input.is_empty() && !desc_focused {
                "(none)".to_string()
            } else if desc_focused {
                format!("{}▏", ge.desc_input)
            } else {
                ge.desc_input.clone()
            };
            lines.push(Line::from(vec![
                Span::styled(
                    if desc_focused { " ✎ " } else { "   " },
                    desc_label_style,
                ),
                Span::styled("Description: ", desc_label_style),
                Span::styled(
                    desc_display,
                    if desc_focused {
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    },
                ),
            ]));

            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                " ─────────────────────────────",
                Style::default().fg(Color::DarkGray),
            )));

            // Show member sessions
            let sessions = self.current_view_sessions();
            let group_name = &ge.original_name;
            let member_count = sessions.iter().filter(|s| {
                let key = format!("{}:{}", s.provider_name, s.provider_session_id);
                self.group_mgr.groups_for(&key).contains(group_name)
            }).count();
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" Members: {} sessions", member_count),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
            for s in sessions.iter() {
                let key = format!("{}:{}", s.provider_name, s.provider_session_id);
                if self.group_mgr.groups_for(&key).contains(group_name) {
                    let title_display = if s.title.is_empty() {
                        crate::util::short_id(&s.provider_session_id, 8).to_string()
                    } else {
                        truncate_str_safe(&s.title, 40)
                    };
                    lines.push(Line::from(vec![
                        Span::raw("   "),
                        Span::styled(format!("{} ", s.state.badge()), Style::default()),
                        Span::styled(format!("{:<6} ", s.provider_name), Style::default().fg(Color::DarkGray)),
                        Span::styled(title_display, Style::default().fg(Color::Gray)),
                    ]));
                }
            }

            let detail = Paragraph::new(lines)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan))
                        .title(" ✏ Edit Group "),
                )
                .wrap(ratatui::widgets::Wrap { trim: false });
            f.render_widget(detail, area);
            return;
        }

        // When AI suggestions are showing, render suggestion detail + session info
        if let AcpState::Results { ref suggestions, ref cursor } = self.acp_state {
            if let Some(sg) = suggestions.get(*cursor) {
                let mut lines = vec![];

                // Suggestion header
                lines.push(Line::from(Span::styled(
                    " AI Suggestion",
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(Span::styled(
                    " ─────────────────────────",
                    Style::default().fg(Color::DarkGray),
                )));

                // Group info
                let new_badge = if sg.is_new { " ✨ new group" } else { " existing group" };
                lines.push(Line::from(vec![
                    Span::styled(" Group: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(&sg.group, Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                    Span::styled(new_badge, Style::default().fg(Color::Yellow)),
                ]));

                // Score
                lines.push(Line::from(vec![
                    Span::styled(" Score: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("{}%", (sg.score * 100.0) as u32),
                        Style::default().fg(Color::Yellow),
                    ),
                ]));

                // Reason (full, wrapped)
                lines.push(Line::from(vec![
                    Span::styled(" Reason: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(&sg.reason, Style::default().fg(Color::White)),
                ]));

                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    " ─────────────────────────",
                    Style::default().fg(Color::DarkGray),
                )));

                // Session info (if found)
                if let Some(session) = self.sessions.iter().find(|s| {
                    let key = format!("{}:{}", s.provider_name, s.provider_session_id);
                    key == sg.session
                }) {
                    lines.push(Line::from(Span::styled(
                        " Original Session",
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                    )));
                    lines.push(Line::from(vec![
                        Span::styled(" Title: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(&session.title, Style::default().fg(Color::White)),
                    ]));
                    lines.push(Line::from(vec![
                        Span::styled(" Provider: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(&session.provider_name, Style::default().fg(Color::Cyan)),
                    ]));
                    lines.push(Line::from(vec![
                        Span::styled(" CWD: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            session.cwd.to_string_lossy().to_string(),
                            Style::default().fg(Color::Gray),
                        ),
                    ]));
                    if !session.summary.is_empty() {
                        lines.push(Line::from(""));
                        lines.push(Line::from(Span::styled(
                            " Summary:",
                            Style::default().fg(Color::DarkGray),
                        )));
                        // Wrap summary text
                        for chunk in session.summary.as_bytes().chunks(70) {
                            let text = String::from_utf8_lossy(chunk);
                            lines.push(Line::from(Span::styled(
                                format!(" {}", text),
                                Style::default().fg(Color::Gray),
                            )));
                        }
                    }
                } else {
                    lines.push(Line::from(vec![
                        Span::styled(" Session: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(&sg.session, Style::default().fg(Color::Gray)),
                    ]));
                }

                let detail = Paragraph::new(lines)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(Color::Green))
                            .title(" Suggestion Detail "),
                    )
                    .wrap(ratatui::widgets::Wrap { trim: false });
                f.render_widget(detail, area);
                return;
            }
        }

        if let Some(session) = self.selected_session() {
            let mut lines = vec![];

            // Header
            lines.push(Line::from(vec![
                Span::styled("ID: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    &session.provider_session_id,
                    Style::default().fg(Color::White),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("Provider: ", Style::default().fg(Color::DarkGray)),
                Span::styled(&session.provider_name, Style::default().fg(Color::Cyan)),
            ]));
            lines.push(Line::from(vec![
                Span::styled("CWD: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    session.cwd.to_string_lossy().to_string(),
                    Style::default().fg(Color::White),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("State: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!(
                        "{} {} ({})",
                        session.state.badge(),
                        session.state.label(),
                        format!("{:?}", session.state.confidence).to_lowercase()
                    ),
                    state_color(&session.state),
                ),
            ]));

            if let Some(pid) = session.pid {
                lines.push(Line::from(vec![
                    Span::styled("PID: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(format!("{}", pid), Style::default().fg(Color::White)),
                ]));
            }

            lines.push(Line::from(vec![
                Span::styled("Created: ", Style::default().fg(Color::DarkGray)),
                Span::styled(&session.created_at, Style::default().fg(Color::DarkGray)),
            ]));
            lines.push(Line::from(vec![
                Span::styled("Updated: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!(
                        "{} ({})",
                        &session.updated_at,
                        format_age(&session.updated_at)
                    ),
                    Style::default().fg(Color::White),
                ),
            ]));

            // Summary
            if !session.summary.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "── Summary ──",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )));
                for summary_line in session.summary.lines() {
                    lines.push(Line::from(Span::raw(summary_line)));
                }
            }

            // State reason (debug info)
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "── State Signals ──",
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(Span::styled(
                &session.state.reason,
                Style::default().fg(Color::DarkGray),
            )));

            // Manual word-wrap: split long lines at panel width.
            // We can't use ratatui's Wrap because it interferes with our padding.
            let inner_width = area.width.saturating_sub(2) as usize;
            let inner_height = area.height.saturating_sub(2) as usize;

            let mut wrapped_lines: Vec<Line<'_>> = Vec::new();
            for line in lines {
                // Flatten all spans into a single string for wrapping
                let mut full_text = String::new();
                let mut style = Style::default();
                for span in &line.spans {
                    full_text.push_str(&span.content);
                    if full_text.len() == span.content.len() {
                        style = span.style; // use first span's style
                    }
                }
                full_text = full_text.replace('\t', "    ");

                // Wrap the text at inner_width using unicode-width
                if UnicodeWidthStr::width(full_text.as_str()) <= inner_width {
                    wrapped_lines.push(Line::from(Span::styled(full_text, style)));
                } else {
                    // Word-wrap: split at word boundaries near inner_width
                    let mut remaining = full_text.as_str();
                    while !remaining.is_empty() {
                        let mut cut = 0;
                        let mut last_space = 0;
                        for (i, ch) in remaining.char_indices() {
                            let w = UnicodeWidthStr::width(&remaining[..i + ch.len_utf8()]);
                            if w > inner_width {
                                break;
                            }
                            cut = i + ch.len_utf8();
                            if ch == ' ' || ch == '-' {
                                last_space = cut;
                            }
                        }
                        if cut == 0 {
                            // Single char wider than panel — force 1 char
                            cut = remaining.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                        }
                        // Prefer breaking at word boundary
                        let break_at = if last_space > 0 && last_space > cut / 2 {
                            last_space
                        } else {
                            cut
                        };
                        wrapped_lines.push(Line::from(Span::styled(
                            remaining[..break_at].to_string(),
                            style,
                        )));
                        remaining = &remaining[break_at..];
                        // Skip leading space on continuation line
                        remaining = remaining.strip_prefix(' ').unwrap_or(remaining);
                    }
                }
            }

            // Pad every line with trailing spaces to fill panel width
            for line in &mut wrapped_lines {
                let display_width: usize = line
                    .spans
                    .iter()
                    .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                    .sum();
                if display_width < inner_width {
                    line.spans
                        .push(Span::raw(" ".repeat(inner_width - display_width)));
                }
            }
            // Pad to fill visible area after scroll
            let total_needed = inner_height + self.detail_scroll as usize;
            while wrapped_lines.len() < total_needed {
                wrapped_lines.push(Line::from(" ".repeat(inner_width)));
            }

            let detail = Paragraph::new(wrapped_lines)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(border_style)
                        .title(" Detail "),
                )
                .scroll((self.detail_scroll, 0));

            f.render_widget(detail, area);
        } else {
            let empty = Paragraph::new("No session selected")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(border_style)
                        .title(" Detail "),
                )
                .style(Style::default().fg(Color::DarkGray));
            f.render_widget(empty, area);
        }
    }

    fn draw_log_viewer(&self, f: &mut Frame, area: Rect) {
        let border_style = if self.focus == Focus::Logs {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let visible_height = area.height.saturating_sub(2) as usize;
        let start = if self.log_lines.len() > visible_height {
            self.log_lines.len() - visible_height
        } else {
            0
        };

        let log_text: Vec<Line> = self.log_lines[start..]
            .iter()
            .map(|l| {
                if l.starts_with("ERROR:") {
                    Line::from(Span::styled(l.as_str(), Style::default().fg(Color::Red)))
                } else {
                    Line::from(Span::styled(
                        l.as_str(),
                        Style::default().fg(Color::DarkGray),
                    ))
                }
            })
            .collect();

        let logs = Paragraph::new(log_text).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .title(" Activity Log "),
        );

        f.render_widget(logs, area);
    }

    fn draw_status_bar(&self, f: &mut Frame, area: Rect) {
        let view_hint = match self.view_mode {
            ViewMode::Active => "Shift+Tab: grouped view",
            ViewMode::Grouped => "Shift+Tab: archived view",
            ViewMode::Hidden => "Shift+Tab: active view",
        };
        let sem_indicator = match &self.semantic_status_cache {
            crate::search::SemanticStatus::Ready { count } => {
                // Cap by current view size so archiving/unarchiving is reflected
                // immediately. The raw cache_count includes embeddings for
                // archived sessions (embeddings are retained on archive and only
                // evicted when the session is deleted from disk).
                let display = (*count).min(self.current_view_sessions().len());
                Span::styled(
                    format!("🧠 {} ", display),
                    Style::default().fg(Color::Green),
                )
            }
            crate::search::SemanticStatus::Indexing { done, total } => Span::styled(
                format!("⏳ {}/{} ", done, total),
                Style::default().fg(Color::Yellow),
            ),
            crate::search::SemanticStatus::Failed(_) => Span::styled("⚠ Semantic failed ", Style::default().fg(Color::Red)),
            crate::search::SemanticStatus::Unavailable => Span::raw(""),
        };
        let status = Paragraph::new(Line::from(vec![
            sem_indicator,
            Span::styled(" Tab", Style::default().fg(Color::Yellow)),
            Span::raw(": panel  "),
            Span::styled("↑↓", Style::default().fg(Color::Yellow)),
            Span::raw(": nav  "),
            Span::styled(view_hint, Style::default().fg(Color::Gray)),
            Span::raw("  "),
            Span::raw(&self.status_message),
        ]))
        .style(Style::default().bg(Color::DarkGray).fg(Color::White));

        f.render_widget(status, area);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build the text that represents a session for semantic embedding.
///
/// Budget: up to ~30 KB, using Nomic Embed v1.5's 8192-token window
/// (~32 KB UTF-8). Prefixed with `search_document:` as required by the
/// model for asymmetric retrieval quality.
///
/// Layout (pipe-separated):
///     search_document: <title> | <summary> | cwd=<basename> | provider=<key> | HEAD:<head> | TAIL:<tail>
///
/// HEAD/TAIL come from the first activity source (JSONL events/logs). HEAD
/// surfaces the initial ask/setup; TAIL surfaces the most recent work. If the
/// provider has no activity sources or they can't be read, the text degrades
/// gracefully to title+summary+cwd+provider.
/// Build multiple embedding chunks for a session. Each chunk is independently
/// embedded so semantic search can match against any aspect (title, compaction
/// summaries, task completions, early user messages).
fn build_semantic_chunks(session: &Session, _registry: &ProviderRegistry) -> Vec<(String, String)> {
    let cwd_name = session
        .cwd
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    let mut chunks = Vec::new();

    // Chunk 1: base identity (always present)
    chunks.push((
        format!(
            "search_document: {} | {} | cwd={} | provider={}",
            session.title, session.summary, cwd_name, session.provider_name
        ),
        "base".to_string(),
    ));

    // Extract structured signals from events.jsonl
    if let Some(ref dir) = session.state_dir {
        let events_path = dir.join("events.jsonl");
        if events_path.exists() {
            let signals = extract_semantic_signals(&events_path);
            for (label, text) in signals {
                if !text.is_empty() {
                    chunks.push((
                        format!("search_document: {}", text),
                        label,
                    ));
                }
            }
        }
    }

    chunks
}

/// Extract stable semantic signals from events.jsonl as separate chunks:
/// - Last compaction summary (most recent context overview, ~500 chars)
/// - Task_complete summaries (concatenated, last 5)
/// - First 3 user messages (session's initial topic)
fn extract_semantic_signals(path: &std::path::Path) -> Vec<(String, String)> {
    use std::io::BufRead;

    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let reader = std::io::BufReader::new(file);

    let mut last_compaction = String::new();
    let mut task_summaries = Vec::new();
    let mut user_msgs = Vec::new();
    const MAX_USER_MSGS: usize = 3;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };

        if line.contains("session.compaction_complete") {
            if let Ok(obj) = serde_json::from_str::<serde_json::Value>(&line) {
                if obj.get("type").and_then(|v| v.as_str()) == Some("session.compaction_complete") {
                    if let Some(s) = obj.get("data").and_then(|d| d.get("summaryContent")).and_then(|v| v.as_str()) {
                        if !s.is_empty() {
                            // Keep only the last compaction (most recent overview)
                            last_compaction = s.chars().take(800).collect();
                        }
                    }
                }
            }
        } else if line.contains("session.task_complete") {
            if let Ok(obj) = serde_json::from_str::<serde_json::Value>(&line) {
                if obj.get("type").and_then(|v| v.as_str()) == Some("session.task_complete") {
                    if let Some(s) = obj.get("data").and_then(|d| d.get("summary")).and_then(|v| v.as_str()) {
                        if !s.is_empty() {
                            task_summaries.push(s.chars().take(200).collect::<String>());
                        }
                    }
                }
            }
        } else if user_msgs.len() < MAX_USER_MSGS && line.contains("\"user.message\"") {
            if let Ok(obj) = serde_json::from_str::<serde_json::Value>(&line) {
                if obj.get("type").and_then(|v| v.as_str()) == Some("user.message") {
                    if let Some(c) = obj.get("data").and_then(|d| d.get("content")).and_then(|v| v.as_str()) {
                        let trimmed = c.trim();
                        if !trimmed.is_empty() && !trimmed.starts_with('<') {
                            user_msgs.push(trimmed.chars().take(150).collect::<String>());
                        }
                    }
                }
            }
        }
    }

    let mut results = Vec::new();

    // Last compaction as its own chunk
    if !last_compaction.is_empty() {
        results.push(("compaction".to_string(), last_compaction));
    }

    // Task summaries combined as one chunk (last 5)
    if !task_summaries.is_empty() {
        let combined: String = task_summaries.iter().rev().take(5)
            .cloned().collect::<Vec<_>>().join(" | ");
        results.push(("tasks".to_string(), combined));
    }

    // User messages combined as one chunk
    if !user_msgs.is_empty() {
        let combined = user_msgs.join(" | ");
        results.push(("user_msgs".to_string(), combined));
    }

    results
}

fn state_color(state: &crate::models::SessionState) -> Style {
    match (state.process, state.interaction) {
        (ProcessState::Running, InteractionState::WaitingInput) => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        (ProcessState::Running, _) => Style::default().fg(Color::Green),
        _ => match state.persistence {
            PersistenceState::Resumable => Style::default().fg(Color::Blue),
            _ => Style::default().fg(Color::DarkGray),
        },
    }
}

fn format_age(iso_timestamp: &str) -> String {
    let Ok(dt) = chrono::DateTime::parse_from_rfc3339(iso_timestamp) else {
        // Try parsing other common formats — assume UTC for naive timestamps
        // (timestamps may lack timezone info)
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(iso_timestamp, "%Y-%m-%d %H:%M:%S")
        {
            let dt_utc = naive.and_utc();
            let duration = chrono::Utc::now().signed_duration_since(dt_utc);
            return format_duration(duration);
        }
        return iso_timestamp.to_string();
    };
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(dt.with_timezone(&chrono::Utc));
    format_duration(duration)
}

fn format_duration(d: chrono::Duration) -> String {
    let secs = d.num_seconds();
    if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

// ---------------------------------------------------------------------------
// Unit tests — test UI logic with mock data (no terminal needed).
// ---------------------------------------------------------------------------
// NOTE: this module is currently disabled because its `make_app` helper calls
// the old 3-arg `App::new` signature, but `App::new` now takes 8 args (adds
// `registry`, `data_dir`, `semantic`, `tick_rate_ms`, `semantic_index_min_interval_ms`).
// Constructing a real App in unit tests would require standing up an on-disk
// provider registry and a semantic plugin, which is out of scope for pure
// UI-logic tests. Re-enable and rewrite these tests against a lightweight
// `AppBuilder` or against pure helper functions (see `empty_provider_bootstrap`
// in `ui_invariant_tests` for the preferred pattern).
#[cfg(any())]
mod ui_logic_tests {
    use super::*;
    use crate::models::*;
    use std::path::PathBuf;

    /// Build a mock session with configurable state axes.
    fn mock_session(
        id: &str,
        title: &str,
        summary: &str,
        provider: &str,
        process: ProcessState,
        interaction: InteractionState,
        persistence: PersistenceState,
    ) -> Session {
        Session {
            id: id.into(),
            provider_session_id: id.into(),
            provider_name: provider.into(),
            cwd: PathBuf::from("D:\\Demo"),
            title: title.into(),
            tab_title: None,
            summary: summary.into(),
            state: SessionState {
                process,
                interaction,
                persistence,
                health: HealthState::Clean,
                confidence: Confidence::High,
                reason: "mock".into(),
            },
            pid: if process == ProcessState::Running { Some(1234) } else { None },
            created_at: "2025-01-15T10:00:00Z".into(),
            updated_at: "2025-01-15T10:30:00Z".into(),
            state_dir: None,
        }
    }

    fn mock_running(id: &str, title: &str) -> Session {
        mock_session(id, title, "doing work", "copilot",
            ProcessState::Running, InteractionState::Busy, PersistenceState::Ephemeral)
    }

    fn mock_waiting(id: &str, title: &str) -> Session {
        mock_session(id, title, "needs input", "copilot",
            ProcessState::Running, InteractionState::WaitingInput, PersistenceState::Ephemeral)
    }

    fn mock_resumable(id: &str, title: &str) -> Session {
        mock_session(id, title, "paused work", "copilot",
            ProcessState::Exited, InteractionState::Idle, PersistenceState::Resumable)
    }

    fn make_app(sessions: Vec<Session>) -> App {
        let mut app = App::new(vec!["copilot".into()], "copilot".into(), 100);
        app.sessions = sessions;
        app.initial_load_complete = true;
        app.apply_filter();
        app
    }

    // ── format_age / format_duration ─────────────────────────────────

    #[test]
    fn format_duration_seconds() {
        let d = chrono::Duration::seconds(45);
        assert_eq!(format_duration(d), "45s ago");
    }

    #[test]
    fn format_duration_minutes() {
        let d = chrono::Duration::seconds(125);
        assert_eq!(format_duration(d), "2m ago");
    }

    #[test]
    fn format_duration_hours() {
        let d = chrono::Duration::seconds(7200);
        assert_eq!(format_duration(d), "2h ago");
    }

    #[test]
    fn format_duration_days() {
        let d = chrono::Duration::seconds(172800);
        assert_eq!(format_duration(d), "2d ago");
    }

    #[test]
    fn format_duration_zero() {
        let d = chrono::Duration::seconds(0);
        assert_eq!(format_duration(d), "0s ago");
    }

    #[test]
    fn format_age_invalid_timestamp_returns_as_is() {
        assert_eq!(format_age("not-a-date"), "not-a-date");
    }

    #[test]
    fn format_age_naive_timestamp_parses() {
        // Should parse and return a duration string (not the raw input)
        let result = format_age("2020-01-01 00:00:00");
        assert!(result.ends_with(" ago"), "expected duration, got: {}", result);
    }

    #[test]
    fn format_age_rfc3339_parses() {
        let result = format_age("2020-01-01T00:00:00Z");
        assert!(result.ends_with(" ago"), "expected duration, got: {}", result);
    }

    // ── state_color ──────────────────────────────────────────────────

    #[test]
    fn state_color_running_waiting_is_yellow_bold() {
        let state = SessionState {
            process: ProcessState::Running,
            interaction: InteractionState::WaitingInput,
            ..SessionState::default()
        };
        let style = state_color(&state);
        assert_eq!(style.fg, Some(Color::Yellow));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn state_color_running_busy_is_green() {
        let state = SessionState {
            process: ProcessState::Running,
            interaction: InteractionState::Busy,
            ..SessionState::default()
        };
        assert_eq!(state_color(&state).fg, Some(Color::Green));
    }

    #[test]
    fn state_color_resumable_is_blue() {
        let state = SessionState {
            process: ProcessState::Exited,
            persistence: PersistenceState::Resumable,
            ..SessionState::default()
        };
        assert_eq!(state_color(&state).fg, Some(Color::Blue));
    }

    #[test]
    fn state_color_ephemeral_is_dark_gray() {
        let state = SessionState::default(); // Ephemeral + Missing
        assert_eq!(state_color(&state).fg, Some(Color::DarkGray));
    }

    // ── App::new initial state ───────────────────────────────────────

    #[test]
    fn app_new_starts_empty() {
        let app = App::new(vec!["copilot".into()], "copilot".into(), 100);
        assert!(app.sessions.is_empty());
        assert!(!app.search_active);
        assert_eq!(app.selected_index, 0);
        assert!(!app.should_quit);
        assert!(!app.initial_load_complete);
    }

    // ── apply_filter ─────────────────────────────────────────────────

    #[test]
    fn apply_filter_empty_query_shows_all() {
        let app = make_app(vec![
            mock_running("1", "Fix auth"),
            mock_waiting("2", "Add tests"),
            mock_resumable("3", "Refactor UI"),
        ]);
        assert_eq!(app.filtered_indices.len(), 3);
    }

    #[test]
    fn apply_filter_with_query_narrows_results() {
        let mut app = make_app(vec![
            mock_running("1", "Fix auth bug"),
            mock_waiting("2", "Add search tests"),
            mock_resumable("3", "Refactor UI layout"),
        ]);
        app.search_query = "auth".into();
        app.apply_filter();
        assert!(app.filtered_indices.len() < 3, "search should filter sessions");
        // The "Fix auth bug" session should be in results
        let view = app.current_view_sessions();
        let matched: Vec<_> = app.filtered_indices.iter()
            .map(|&i| view[i].title.as_str())
            .collect();
        assert!(matched.contains(&"Fix auth bug"), "auth session should match");
    }

    #[test]
    fn apply_filter_no_match_returns_empty() {
        let mut app = make_app(vec![
            mock_running("1", "Fix auth"),
            mock_waiting("2", "Add tests"),
        ]);
        app.search_query = "zzzznonexistent".into();
        app.apply_filter();
        assert_eq!(app.filtered_indices.len(), 0);
    }

    #[test]
    fn apply_filter_resets_selection_to_zero() {
        let mut app = make_app(vec![
            mock_running("1", "A"),
            mock_waiting("2", "B"),
            mock_resumable("3", "C"),
        ]);
        app.selected_index = 2;
        app.search_query = "A".into();
        app.apply_filter();
        assert_eq!(app.selected_index, 0, "filter should reset selection to top");
    }

    // ── Navigation ───────────────────────────────────────────────────

    #[test]
    fn navigate_down_increments_selection() {
        let mut app = make_app(vec![
            mock_running("1", "A"),
            mock_waiting("2", "B"),
            mock_resumable("3", "C"),
        ]);
        let (tx, _rx) = mpsc::unbounded_channel();
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &tx);
        assert_eq!(app.selected_index, 1);
        assert!(app.user_navigated);
    }

    #[test]
    fn navigate_up_at_top_stays_at_zero() {
        let mut app = make_app(vec![
            mock_running("1", "A"),
            mock_waiting("2", "B"),
        ]);
        let (tx, _rx) = mpsc::unbounded_channel();
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), &tx);
        assert_eq!(app.selected_index, 0);
    }

    #[test]
    fn navigate_down_at_bottom_stays() {
        let mut app = make_app(vec![
            mock_running("1", "A"),
        ]);
        let (tx, _rx) = mpsc::unbounded_channel();
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &tx);
        assert_eq!(app.selected_index, 0, "can't go below last item");
    }

    #[test]
    fn j_and_k_navigate_like_arrows() {
        let mut app = make_app(vec![
            mock_running("1", "A"),
            mock_waiting("2", "B"),
            mock_resumable("3", "C"),
        ]);
        let (tx, _rx) = mpsc::unbounded_channel();
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE), &tx);
        assert_eq!(app.selected_index, 1, "j should move down");
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE), &tx);
        assert_eq!(app.selected_index, 0, "k should move up");
    }

    // ── Search mode ──────────────────────────────────────────────────

    #[test]
    fn slash_enters_search_mode() {
        let mut app = make_app(vec![mock_running("1", "A")]);
        let (tx, _rx) = mpsc::unbounded_channel();
        assert!(!app.search_active);
        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE), &tx);
        assert!(app.search_active);
        assert!(app.search_query.is_empty());
    }

    #[test]
    fn search_typing_updates_query() {
        let mut app = make_app(vec![
            mock_running("1", "Fix auth"),
            mock_waiting("2", "Add tests"),
        ]);
        let (tx, _rx) = mpsc::unbounded_channel();
        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE), &tx);
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE), &tx);
        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE), &tx);
        assert_eq!(app.search_query, "au");
    }

    #[test]
    fn search_backspace_removes_char() {
        let mut app = make_app(vec![mock_running("1", "A")]);
        let (tx, _rx) = mpsc::unbounded_channel();
        app.search_active = true;
        app.search_query = "abc".into();
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE), &tx);
        assert_eq!(app.search_query, "ab");
    }

    #[test]
    fn search_esc_exits_and_clears() {
        let mut app = make_app(vec![mock_running("1", "A")]);
        let (tx, _rx) = mpsc::unbounded_channel();
        app.search_active = true;
        app.search_query = "test".into();
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &tx);
        assert!(!app.search_active);
        assert!(app.search_query.is_empty());
    }

    // ── Focus cycling ────────────────────────────────────────────────

    #[test]
    fn tab_cycles_focus_forward() {
        let mut app = make_app(vec![mock_running("1", "A")]);
        let (tx, _rx) = mpsc::unbounded_channel();
        assert_eq!(app.focus, Focus::SessionList);
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &tx);
        assert_eq!(app.focus, Focus::Detail);
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &tx);
        assert_eq!(app.focus, Focus::Logs);
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &tx);
        assert_eq!(app.focus, Focus::SessionList);
    }

    #[test]
    fn backtab_in_detail_goes_to_session_list() {
        let mut app = make_app(vec![mock_running("1", "A")]);
        let (tx, _rx) = mpsc::unbounded_channel();
        app.focus = Focus::Detail;
        app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT), &tx);
        assert_eq!(app.focus, Focus::SessionList);
    }

    // ── View mode toggle ─────────────────────────────────────────────

    #[test]
    fn shift_tab_toggles_view_mode() {
        let mut app = make_app(vec![mock_running("1", "A")]);
        app.hidden_sessions = vec![mock_resumable("2", "Hidden")];
        let (tx, _rx) = mpsc::unbounded_channel();
        assert_eq!(app.view_mode, ViewMode::Active);
        app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT), &tx);
        assert_eq!(app.view_mode, ViewMode::Hidden);
        assert_eq!(app.filtered_indices.len(), 1, "should show hidden sessions");
        app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT), &tx);
        assert_eq!(app.view_mode, ViewMode::Active);
    }

    // ── handle_enter dispatch ────────────────────────────────────────

    #[test]
    fn enter_on_running_with_tab_title_sends_focus() {
        let mut session = mock_running("1", "Active task");
        session.tab_title = Some("Fixing auth".into());
        let mut app = make_app(vec![session]);
        let (tx, mut rx) = mpsc::unbounded_channel();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &tx);
        match rx.try_recv() {
            Ok(SupervisorCommand::FocusSession { tab_title, .. }) => {
                assert_eq!(tab_title, Some("Fixing auth".into()));
            }
            other => panic!("expected FocusSession, got {:?}", other),
        }
    }

    #[test]
    fn enter_on_running_without_tab_title_shows_warning() {
        let app_session = mock_running("1", "Active task");
        let mut app = make_app(vec![app_session]);
        let (tx, mut rx) = mpsc::unbounded_channel();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &tx);
        // No command should be sent (tab_title is None)
        assert!(rx.try_recv().is_err(), "no command when tab_title is None");
        assert!(app.status_message.contains("not available"));
    }

    #[test]
    fn enter_on_resumable_sends_resume() {
        let mut app = make_app(vec![mock_resumable("1", "Paused task")]);
        let (tx, mut rx) = mpsc::unbounded_channel();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &tx);
        match rx.try_recv() {
            Ok(SupervisorCommand::ResumeSession { provider_session_id, .. }) => {
                assert_eq!(provider_session_id, "1");
            }
            other => panic!("expected ResumeSession, got {:?}", other),
        }
    }

    #[test]
    fn enter_on_empty_list_does_nothing() {
        let mut app = make_app(vec![]);
        let (tx, mut rx) = mpsc::unbounded_channel();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &tx);
        assert!(rx.try_recv().is_err(), "no command on empty list");
    }

    // ── Quit ─────────────────────────────────────────────────────────

    #[test]
    fn q_sets_should_quit() {
        let mut app = make_app(vec![]);
        let (tx, _rx) = mpsc::unbounded_channel();
        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE), &tx);
        assert!(app.should_quit);
    }

    #[test]
    fn ctrl_c_sets_should_quit() {
        let mut app = make_app(vec![]);
        let (tx, _rx) = mpsc::unbounded_channel();
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL), &tx);
        assert!(app.should_quit);
    }

    #[test]
    fn ctrl_c_in_search_mode_also_quits() {
        let mut app = make_app(vec![]);
        app.search_active = true;
        let (tx, _rx) = mpsc::unbounded_channel();
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL), &tx);
        assert!(app.should_quit);
    }

    // ── Detail scroll ────────────────────────────────────────────────

    #[test]
    fn detail_scroll_up_down() {
        let mut app = make_app(vec![mock_running("1", "A")]);
        let (tx, _rx) = mpsc::unbounded_channel();
        app.focus = Focus::Detail;
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &tx);
        assert_eq!(app.detail_scroll, 1);
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &tx);
        assert_eq!(app.detail_scroll, 2);
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), &tx);
        assert_eq!(app.detail_scroll, 1);
    }

    #[test]
    fn detail_scroll_home_resets() {
        let mut app = make_app(vec![mock_running("1", "A")]);
        let (tx, _rx) = mpsc::unbounded_channel();
        app.focus = Focus::Detail;
        app.detail_scroll = 50;
        app.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE), &tx);
        assert_eq!(app.detail_scroll, 0);
    }

    #[test]
    fn detail_scroll_end_sets_max() {
        let mut app = make_app(vec![mock_running("1", "A")]);
        let (tx, _rx) = mpsc::unbounded_channel();
        app.focus = Focus::Detail;
        app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE), &tx);
        assert_eq!(app.detail_scroll, u16::MAX);
    }

    #[test]
    fn detail_page_up_down() {
        let mut app = make_app(vec![mock_running("1", "A")]);
        let (tx, _rx) = mpsc::unbounded_channel();
        app.focus = Focus::Detail;
        app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), &tx);
        assert_eq!(app.detail_scroll, 20);
        app.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE), &tx);
        assert_eq!(app.detail_scroll, 0);
    }

    // ── Log scroll ───────────────────────────────────────────────────

    #[test]
    fn log_scroll_respects_bounds() {
        let mut app = make_app(vec![]);
        app.focus = Focus::Logs;
        app.log_lines = vec!["line1".into(), "line2".into(), "line3".into()];
        let (tx, _rx) = mpsc::unbounded_channel();
        // Can scroll down
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &tx);
        assert_eq!(app.log_scroll, 1);
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &tx);
        assert_eq!(app.log_scroll, 2);
        // Can't scroll past end
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &tx);
        assert_eq!(app.log_scroll, 2, "should not scroll past last line");
        // Can scroll back up
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), &tx);
        assert_eq!(app.log_scroll, 1);
    }

    // ── selected_session ─────────────────────────────────────────────

    #[test]
    fn selected_session_returns_correct_item() {
        let app = make_app(vec![
            mock_running("1", "First"),
            mock_waiting("2", "Second"),
        ]);
        let s = app.selected_session().expect("should have selection");
        assert_eq!(s.title, "First");
    }

    #[test]
    fn selected_session_after_navigate() {
        let mut app = make_app(vec![
            mock_running("1", "First"),
            mock_waiting("2", "Second"),
            mock_resumable("3", "Third"),
        ]);
        let (tx, _rx) = mpsc::unbounded_channel();
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &tx);
        let s = app.selected_session().expect("should have selection");
        assert_eq!(s.title, "Second");
    }

    // ── Navigation resets detail scroll ──────────────────────────────

    #[test]
    fn navigate_resets_detail_scroll() {
        let mut app = make_app(vec![
            mock_running("1", "A"),
            mock_waiting("2", "B"),
        ]);
        let (tx, _rx) = mpsc::unbounded_channel();
        app.detail_scroll = 10;
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &tx);
        assert_eq!(app.detail_scroll, 0, "navigating should reset detail scroll");
    }

    // ── New session command ──────────────────────────────────────────

    #[test]
    fn n_key_sends_new_session() {
        let mut app = make_app(vec![]);
        let (tx, mut rx) = mpsc::unbounded_channel();
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE), &tx);
        match rx.try_recv() {
            Ok(SupervisorCommand::NewSession { provider_key, .. }) => {
                assert_eq!(provider_key, "copilot");
            }
            other => panic!("expected NewSession, got {:?}", other),
        }
    }
}

// ---------------------------------------------------------------------------
// Regression tests — enforce UI invariants so future changes can't silently
// break rendering or terminal cleanup.
// These read the source file and assert critical patterns are present.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod ui_invariant_tests {
    use std::fs;
    use super::{clamp_cursor_after_removal, empty_provider_bootstrap};

    fn ui_source() -> String {
        fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/ui/mod.rs"))
            .expect("should read ui/mod.rs")
    }

    fn code_section() -> String {
        let src = ui_source();
        src.split("#[cfg(test)]").next().unwrap_or(&src).to_string()
    }

    // ── Zero-providers startup resilience ───────────────────────────────
    // A user who installs the release zip with NO agent CLIs (or disables
    // all providers in config.toml, or ships with a broken providers/ dir
    // where all YAMLs fail to parse) must see a responsive UI with an
    // actionable message — NOT a forever-stuck "Loading..." spinner.
    // These guard the fix in App::new.

    #[test]
    fn empty_providers_marks_initial_load_complete() {
        let (no_providers, _status) = empty_provider_bootstrap(0);
        assert!(
            no_providers,
            "With zero providers, initial_load_complete must start true — \
             supervisor will never emit scan events to flip it later."
        );
    }

    #[test]
    fn empty_providers_shows_actionable_status() {
        let (_no_providers, status) = empty_provider_bootstrap(0);
        assert!(
            status.contains("No providers enabled"),
            "Zero-providers status must explicitly tell the user that no providers are enabled, got: {status:?}"
        );
        assert!(
            status.contains("config.toml"),
            "Zero-providers status must mention config.toml so the user knows where to fix it, got: {status:?}"
        );
    }

    #[test]
    fn non_empty_providers_uses_normal_loading_status() {
        let (no_providers, status) = empty_provider_bootstrap(3);
        assert!(
            !no_providers,
            "With >0 providers, initial_load_complete must start false and flip only after all providers report in."
        );
        assert_eq!(
            status, "Loading 3 providers...",
            "Normal startup path must show the loading count."
        );
    }

    #[test]
    fn no_mouse_capture() {
        let code = code_section();
        assert!(
            !code.contains("EnableMouseCapture"),
            "No mouse capture — native click-drag text selection must work"
        );
        assert!(
            !code.contains("DisableMouseCapture"),
            "No DisableMouseCapture needed when capture is not enabled"
        );
        assert!(
            !code.contains("Event::Mouse"),
            "No mouse event handling — terminal handles mouse natively"
        );
        assert!(
            !code.contains("fn handle_mouse"),
            "No handle_mouse method — no mouse capture"
        );
    }

    #[test]
    fn detail_panel_pads_lines_to_fill() {
        let code = code_section();
        assert!(
            code.contains("inner_width"),
            "draw_session_detail must pad lines to fill panel width (prevents ghost characters)"
        );
        assert!(
            code.contains("inner_height"),
            "draw_session_detail must pad rows to fill panel height"
        );
    }

    #[test]
    fn no_clear_widget_in_detail() {
        let code = code_section();
        assert!(
            !code.contains("render_widget(Clear"),
            "Do NOT use Clear widget — causes flicker by resetting all cells every frame"
        );
    }

    #[test]
    fn no_terminal_clear_for_redraw() {
        let code = code_section();
        let clear_count = code.matches("terminal.clear()").count();
        assert!(
            clear_count <= 1,
            "terminal.clear() only at startup — found {clear_count}"
        );
        assert!(
            !code.contains("needs_full_redraw"),
            "No full-screen redraw machinery"
        );
    }

    #[test]
    fn only_press_events_handled() {
        let src = ui_source();
        assert!(
            src.contains("KeyEventKind::Press"),
            "Filter to Press only (Windows double/triple)"
        );
    }

    #[test]
    fn terminal_restored_on_quit_and_panic() {
        let code = code_section();
        let leave_count = code.matches("LeaveAlternateScreen").count();
        assert!(
            leave_count >= 2,
            "LeaveAlternateScreen in quit + panic (found {leave_count})"
        );
    }

    // ── Archive cursor preservation (pure-fn + structural checks) ───────
    //
    // Regression guard: when the user presses 'a' to archive the row under
    // the cursor, the cursor must stay at the same visual index so the next
    // row slides up into it. This supports rapid repeat-archive (press 'a'
    // over and over to clear a run of rows). `apply_filter()` zeroes the
    // selection after archive, so the handler must capture the previous
    // index and restore it via `clamp_cursor_after_removal`.

    #[test]
    fn clamp_cursor_empty_list_returns_none() {
        assert_eq!(clamp_cursor_after_removal(0, 0), None);
        assert_eq!(clamp_cursor_after_removal(5, 0), None);
    }

    #[test]
    fn clamp_cursor_preserves_middle_position() {
        // Was at row 1 of 4; after removing row 1, list is length 3 and
        // cursor should STAY at index 1 so the row that was #2 slides up.
        assert_eq!(clamp_cursor_after_removal(1, 3), Some(1));
    }

    #[test]
    fn clamp_cursor_clamps_to_last_row_when_out_of_range() {
        // Was at row 3 of 4 (last); after removing it, list is length 3
        // and cursor must clamp down to new last row (index 2).
        assert_eq!(clamp_cursor_after_removal(3, 3), Some(2));
        // Cursor way past the end still clamps to last row.
        assert_eq!(clamp_cursor_after_removal(99, 3), Some(2));
    }

    #[test]
    fn clamp_cursor_preserves_zero() {
        // Archiving the first row of a multi-row list keeps cursor at 0.
        assert_eq!(clamp_cursor_after_removal(0, 3), Some(0));
    }

    #[test]
    fn archive_handler_calls_clamp_cursor_after_removal() {
        // Structural invariant: the 'a' key handler MUST go through
        // `clamp_cursor_after_removal` after `apply_filter()` so the cursor
        // doesn't jump back to row 0. If someone refactors this to call
        // `apply_filter` without restoring the cursor, this test fails.
        let code = code_section();
        assert!(
            code.contains("KeyCode::Char('a')"),
            "archive handler ('a' key) must exist in ui::mod"
        );
        assert!(
            code.contains("clamp_cursor_after_removal"),
            "archive handler must call clamp_cursor_after_removal after \
             apply_filter() to preserve cursor position for rapid-repeat \
             archive. Otherwise the cursor jumps back to row 0 every press."
        );
    }

    // ── Archive bounce-back race (pending_archives drain must wait for
    //    `ArchiveConfirmed`, never scan `hidden`) ────────────────────────
    // Regression: rapid 'a' spam caused count to briefly drop (e.g. 500 →
    // 480) and then bounce back up (480 → 505) several seconds later. The
    // cause was `pending_archives.retain(|k| !hidden.contains(k))` running
    // on every SessionsUpdated. The UI's own filter had just pushed keys
    // into `hidden` *locally* (before the archive was persisted on disk),
    // so those keys were dropped from pending_archives. Subsequent scans —
    // still reflecting pre-archive disk state — placed the sessions back
    // in `active` with nothing to filter them out.
    //
    // The fix: drain `pending_archives` ONLY on `SupervisorEvent::
    // ArchiveConfirmed`, which is fired by `handle_archive` *after* the
    // archive record has been written. This test enforces the contract
    // so the regression can't silently return.

    #[test]
    fn archive_confirmed_event_exists_and_ui_handles_it() {
        let full = ui_source();
        assert!(
            full.contains("SupervisorEvent::ArchiveConfirmed"),
            "UI must handle SupervisorEvent::ArchiveConfirmed — that is \
             the only safe signal that an archive has been persisted and \
             its pending_archives entry can be dropped."
        );
    }

    #[test]
    fn pending_archives_not_drained_from_hidden_scan() {
        // Enforces that the eager drain is gone. If anyone re-introduces
        // `pending_archives.retain(...hidden...)` the rapid-archive
        // bounce-back returns.
        let code = code_section();
        let bad_patterns = [
            "pending_archives.retain(|k| {\n                                !hidden",
            "pending_archives.retain(|k| !hidden",
        ];
        for pat in bad_patterns {
            assert!(
                !code.contains(pat),
                "pending_archives must NOT be drained based on scan \
                 `hidden` list. That is the race that caused the archive \
                 bounce-back (count drops then reappears). Drain only on \
                 SupervisorEvent::ArchiveConfirmed."
            );
        }
    }

    #[test]
    fn supervisor_emits_archive_confirmed_after_persist() {
        // Cross-module invariant: the UI's correctness depends on the
        // supervisor actually firing the event. Re-read supervisor/mod.rs
        // and make sure ArchiveConfirmed is both declared and sent from
        // handle_archive.
        let sup = fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/supervisor/mod.rs"
        ))
        .expect("should read supervisor/mod.rs");
        assert!(
            sup.contains("ArchiveConfirmed"),
            "supervisor must declare SupervisorEvent::ArchiveConfirmed"
        );
        // Verify it's also used (sent), not just declared.
        let occurrences = sup.matches("ArchiveConfirmed").count();
        assert!(
            occurrences >= 2,
            "ArchiveConfirmed must be declared AND sent from handle_archive \
             (found {} occurrence(s))",
            occurrences
        );
        assert!(
            sup.contains("handle_archive"),
            "handle_archive must exist in supervisor/mod.rs"
        );
    }

    // ── Unarchive feature (symmetric to archive) ──────────────────────
    //
    // 'a' in the Hidden view must restore the session. The implementation
    // mirrors the archive path exactly — including the bounce-back race
    // guard via `pending_unarchives` + `SupervisorEvent::UnarchiveConfirmed`.

    #[test]
    fn unarchive_confirmed_event_exists_and_ui_handles_it() {
        let full = fs::read_to_string(file!()).expect("should read ui/mod.rs");
        assert!(
            full.contains("SupervisorEvent::UnarchiveConfirmed"),
            "UI must handle SupervisorEvent::UnarchiveConfirmed — mirrors \
             ArchiveConfirmed so pending_unarchives drains only after the \
             supervisor has persisted the unarchive."
        );
        assert!(
            full.contains("pending_unarchives"),
            "App must track pending_unarchives to prevent bounce-back of \
             just-unarchived sessions back into hidden view."
        );
    }

    #[test]
    fn pending_unarchives_not_drained_from_active_scan() {
        let full = fs::read_to_string(file!()).expect("should read ui/mod.rs");
        // Build the anti-patterns at runtime so this test's own source
        // doesn't trip its own match (as the archive sibling test avoids
        // via a line-break).
        let retain_head = format!("{}{}", "pending_unarch", "ives.retain(|k| ");
        let bad_patterns = [
            format!("{}!active", retain_head),
            format!("{}{{\n                                !active", retain_head),
        ];
        for pattern in &bad_patterns {
            assert!(
                !full.contains(pattern),
                "pending_unarchives must NOT be drained based on scan \
                 results containing the session in active — that reintroduces \
                 the bounce-back race. Drain only on \
                 SupervisorEvent::UnarchiveConfirmed."
            );
        }
    }

    #[test]
    fn supervisor_emits_unarchive_confirmed_after_persist() {
        let sup = fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/supervisor/mod.rs"
        ))
        .expect("should read supervisor/mod.rs");
        assert!(
            sup.contains("UnarchiveConfirmed"),
            "supervisor must declare SupervisorEvent::UnarchiveConfirmed"
        );
        let occurrences = sup.matches("UnarchiveConfirmed").count();
        assert!(
            occurrences >= 2,
            "UnarchiveConfirmed must be declared AND sent from \
             handle_unarchive (found {} occurrence(s))",
            occurrences
        );
        assert!(
            sup.contains("handle_unarchive"),
            "handle_unarchive must exist in supervisor/mod.rs"
        );
        assert!(
            sup.contains("UnarchiveSession"),
            "SupervisorCommand::UnarchiveSession must exist so the UI can \
             request an unarchive."
        );
    }

    #[test]
    fn a_key_handler_branches_on_view_mode_for_unarchive() {
        // The 'a' handler must dispatch to UnarchiveSession when in
        // ViewMode::Hidden, otherwise unarchive is unreachable from the UI.
        let full = fs::read_to_string(file!()).expect("should read ui/mod.rs");
        assert!(
            full.contains("SupervisorCommand::UnarchiveSession"),
            "UI must send SupervisorCommand::UnarchiveSession from the 'a' \
             key handler in the Hidden view."
        );
    }

    // ── Post-confirm bounce-back race (two-gate drain) ───────────────────
    // Regression: even after the first fix (drain only on ArchiveConfirmed),
    // the user still saw counts drop to ~2xx then climb to ~4xx after
    // rapid 'a' spam. Root cause: scans that were in flight BEFORE persist
    // can arrive AFTER ArchiveConfirmed. If the pending entry was drained
    // the instant ArchiveConfirmed fired, those stale scans saw an empty
    // pending filter and repopulated the freshly-archived sessions in
    // active. Symmetric failure for unarchive: stale scan repopulated the
    // session back in hidden, so the unarchived session vanished from
    // every view.
    //
    // The fix: drain requires TWO gates. (1) Supervisor confirms persist.
    // (2) A subsequent scan's ORIGINAL view (captured before our filter
    // moved anything) independently reports the session on the expected
    // side. Stale scans fail gate (2), so the filter persists through
    // them. The tests below enforce both halves of that contract in
    // source.
    #[test]
    fn pending_transitions_have_confirmed_gate() {
        // The PendingTransition struct must carry a `confirmed` flag; the
        // two-gate drain depends on it.
        let full = fs::read_to_string(file!()).expect("should read ui/mod.rs");
        assert!(
            full.contains("struct PendingTransition"),
            "PendingTransition struct must exist — it is what allows the \
             two-gate drain (confirmed + independent scan observation)."
        );
        assert!(
            full.contains("confirmed: bool"),
            "PendingTransition must expose a `confirmed` flag. Without it \
             the drain logic collapses back to single-gate and the \
             bounce-back race returns."
        );
    }

    #[test]
    fn archive_confirmed_marks_only_does_not_drain() {
        // ArchiveConfirmed must NOT drain the pending entry — it only
        // flips `confirmed = true`. Draining is the SessionsUpdated
        // handler's job, after it has observed the scan's OWN placement
        // of the session.
        let full = fs::read_to_string(file!()).expect("should read ui/mod.rs");
        // Locate the ArchiveConfirmed handler block and check it does
        // NOT contain a retain on pending_archives. Build the forbidden
        // needle at runtime so this test's own source doesn't self-match.
        let forbidden = format!("{}{}", "pending_arch", "ives.retain");
        // Extract the ArchiveConfirmed handler block (approximate):
        let handler_start = full
            .find("SupervisorEvent::ArchiveConfirmed {")
            .expect("ArchiveConfirmed handler must exist");
        let handler_end = full[handler_start..]
            .find("SupervisorEvent::UnarchiveConfirmed")
            .map(|i| handler_start + i)
            .expect("UnarchiveConfirmed must follow ArchiveConfirmed in the match");
        let handler_body = &full[handler_start..handler_end];
        assert!(
            !handler_body.contains(&forbidden),
            "ArchiveConfirmed handler must NOT drain pending_archives \
             directly (found `{}` in the handler body). Drain belongs \
             in the SessionsUpdated handler, gated by both confirmation \
             AND an independent scan observation of the session in \
             `hidden`. Single-gate drain on ArchiveConfirmed alone \
             reopens the bounce-back race.",
            forbidden
        );
    }

    #[test]
    fn unarchive_confirmed_marks_only_does_not_drain() {
        // Symmetric to the archive version.
        let full = fs::read_to_string(file!()).expect("should read ui/mod.rs");
        let forbidden = format!("{}{}", "pending_unarch", "ives.retain");
        let handler_start = full
            .find("SupervisorEvent::UnarchiveConfirmed {")
            .expect("UnarchiveConfirmed handler must exist");
        // Find next top-level match arm — Error is the next sibling arm.
        let handler_end = full[handler_start..]
            .find("SupervisorEvent::Error")
            .map(|i| handler_start + i)
            .expect("Error arm must follow UnarchiveConfirmed in the match");
        let handler_body = &full[handler_start..handler_end];
        assert!(
            !handler_body.contains(&forbidden),
            "UnarchiveConfirmed handler must NOT drain pending_unarchives \
             directly. Drain belongs in the SessionsUpdated handler, \
             gated by both confirmation AND an independent scan \
             observation of the session in `active`."
        );
    }

    #[test]
    fn sessions_updated_snapshots_scan_views_before_filtering() {
        // The two-gate drain requires knowing the scan's ORIGINAL placement
        // of each session — captured BEFORE our pending filter moves
        // anything. If someone re-orders the code to build these sets
        // after the filter runs, the drain degenerates: our own moves
        // would satisfy gate (2), stale scans would drain the entry, and
        // the bounce-back regression returns.
        let full = fs::read_to_string(file!()).expect("should read ui/mod.rs");
        let sessions_updated_start = full
            .find("SupervisorEvent::SessionsUpdated {")
            .expect("SessionsUpdated handler must exist");
        let snap_hidden = full[sessions_updated_start..]
            .find("scan_hidden_keys");
        let snap_active = full[sessions_updated_start..]
            .find("scan_active_keys");
        let filter_apply = full[sessions_updated_start..]
            .find("if !self.pending_archives.is_empty()");
        assert!(
            snap_hidden.is_some() && snap_active.is_some(),
            "SessionsUpdated must snapshot both scan_hidden_keys and \
             scan_active_keys for the two-gate drain."
        );
        assert!(
            filter_apply.is_some(),
            "SessionsUpdated must still apply the pending_archives filter."
        );
        assert!(
            snap_hidden.unwrap() < filter_apply.unwrap()
                && snap_active.unwrap() < filter_apply.unwrap(),
            "scan_hidden_keys and scan_active_keys must be built BEFORE \
             the pending filter runs. Building them after would let the \
             filter's own moves satisfy the drain gate, defeating the \
             purpose."
        );
    }
}
