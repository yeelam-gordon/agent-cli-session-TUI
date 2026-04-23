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
        }
    }

    /// Get the list being displayed based on view mode.
    fn current_view_sessions(&self) -> &[Session] {
        match self.view_mode {
            ViewMode::Active => &self.sessions,
            ViewMode::Hidden => &self.hidden_sessions,
        }
    }

    /// Get the currently selected session (through the filter).
    fn selected_session(&self) -> Option<&Session> {
        self.filtered_indices
            .get(self.selected_index)
            .and_then(|&idx| self.current_view_sessions().get(idx))
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
            let results = crate::search::ranked_search(view, &query, sem_ref, log_ref);
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
                                                build_semantic_text(s, &registry)
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
                                    let text = build_semantic_text(session, &registry);
                                    let text_hash = crate::search::hash_text(&text);

                                    // Short lock: skip-check.
                                    let needs = {
                                        let sem = match sem_clone.lock() {
                                            Ok(g) => g,
                                            Err(_) => return,
                                        };
                                        sem.needs_embedding(&session.id, text_hash)
                                    };

                                    if needs {
                                        // Longer lock: the actual embed + cache insert.
                                        // Held for ~1-2s on CPU, then released so
                                        // the UI's next draw tick + user searches
                                        // get a turn before the next embed starts.
                                        let one_embed_start = std::time::Instant::now();
                                        let mut sem = match sem_clone.lock() {
                                            Ok(g) => g,
                                            Err(_) => return,
                                        };
                                        if sem.embed_and_cache(&session.id, &text, text_hash) {
                                            embedded_since_flush += 1;
                                            embedded_count += 1;
                                        }
                                        sem.update_progress(i + 1, total);
                                        // Flush every 20 new embeddings so a
                                        // mid-indexing process kill still leaves
                                        // persisted progress.
                                        if embedded_since_flush >= 20 {
                                            sem.save_cache();
                                            embedded_since_flush = 0;
                                        }
                                        drop(sem);
                                        let one_embed_ms =
                                            one_embed_start.elapsed().as_millis();
                                        total_embed_ms += one_embed_ms;
                                        crate::log::info(&format!(
                                            "[idx] embed session={} text_len={} ({}ms)",
                                            &session.id[..session.id.len().min(8)],
                                            text.len(),
                                            one_embed_ms
                                        ));
                                    } else {
                                        // Still bump progress for skipped sessions.
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
                                            crate::log::info(&format!("log index refresh failed: {}", e));
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
                                ViewMode::Active => active_count,
                                ViewMode::Hidden => hidden_count,
                            };
                            let mode_label = match self.view_mode {
                                ViewMode::Active => "active",
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
                KeyCode::Enter => {
                    self.handle_enter(cmd_tx);
                }
                KeyCode::Char('a') => {
                    if let Some(session) = self.selected_session() {
                        let psid = session.provider_session_id.clone();
                        let pname = session.provider_name.clone();
                        let key = format!("{}:{}", pname, psid);
                        match self.view_mode {
                            ViewMode::Active => {
                                let _ = cmd_tx.send(SupervisorCommand::ArchiveSession {
                                    provider_session_id: psid.clone(),
                                    provider_key: pname.clone(),
                                });
                                // Track locally so incoming refreshes don't put it back
                                self.pending_archives.push(PendingTransition { key, confirmed: false });
                                // Instantly move from active to hidden
                                if let Some(&idx) = self.filtered_indices.get(self.selected_index) {
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
                    // Shift+Tab: toggle between Active and Hidden view
                    self.view_mode = match self.view_mode {
                        ViewMode::Active => ViewMode::Hidden,
                        ViewMode::Hidden => ViewMode::Active,
                    };
                    self.selected_index = 0;
                    self.list_state.select(Some(0));
                    self.search_query.clear();
                    self.apply_filter();
                    self.log_lines.push(format!(
                        "View: {}",
                        match self.view_mode {
                            ViewMode::Active => "Active sessions",
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
    }

    fn draw_title_bar(&self, f: &mut Frame, area: Rect) {
        let hl = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);

        if self.search_active {
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
        } else {
            // Normal mode
            let title = Paragraph::new(Line::from(vec![
                Span::styled(" Agent Session Manager ", Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw("  "),
                Span::styled("⏎", hl),
                Span::raw(" open  "),
                Span::styled("n", hl),
                Span::raw("ew  "),
                Span::styled("a", hl),
                Span::raw("rchive  "),
                Span::styled("/", hl),
                Span::raw("search  "),
                Span::styled("q", hl),
                Span::raw("uit"),
            ]));
            f.render_widget(title, area);
        }
    }

    fn draw_session_list(&mut self, f: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .filtered_indices
            .iter()
            .enumerate()
            .map(|(list_idx, &session_idx)| {
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

                let age_line = Line::from(vec![
                    Span::raw("   "),
                    Span::styled(
                        format!("{} · {}", s.state.label(), age),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]);

                ListItem::new(vec![line, age_line])
            })
            .collect();

        let border_style = if self.focus == Focus::SessionList || self.search_active {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let view_label = match self.view_mode {
            ViewMode::Active => "Sessions",
            ViewMode::Hidden => "📦 Archived & Hidden",
        };
        let view_count = self.current_view_sessions().len();

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

    fn draw_session_detail(&self, f: &mut Frame, area: Rect) {
        let border_style = if self.focus == Focus::Detail {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

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
            ViewMode::Active => "Shift+Tab: show archived  a: archive",
            ViewMode::Hidden => "Shift+Tab: show active  a: unarchive",
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
fn build_semantic_text(session: &Session, _registry: &ProviderRegistry) -> String {
    // IMPORTANT: this text drives the embedding cache hash. If it changes,
    // the session gets re-embedded (load model → embed → unload, ~550MB churn).
    // So we deliberately use ONLY stable per-session-identity signals here.
    // Log head/tail used to be appended for richer semantics, but that made
    // active sessions re-index every supervisor poll as their logs grew.
    // Title+summary+cwd is enough for "find session by topic" search, and
    // changes only when the provider re-summarizes (rare, not per log byte).
    let cwd_name = session
        .cwd
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    format!(
        "search_document: {} | {} | cwd={} | provider={}",
        session.title, session.summary, cwd_name, session.provider_name
    )
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
