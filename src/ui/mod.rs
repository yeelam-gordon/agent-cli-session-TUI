use std::io;

use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{prelude::*, widgets::*};
use tokio::sync::mpsc;

use crate::models::{InteractionState, PersistenceState, ProcessState, Session};
use crate::supervisor::{SupervisorCommand, SupervisorEvent};
use crate::util::truncate_str_safe;

/// Which panel has focus (list-only layout).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    SessionList,
}

/// Which view is displayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    /// Normal: active sessions only.
    Active,
    /// Archive: archived + empty sessions.
    Hidden,
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
    status_message: String,
    should_quit: bool,
    provider_keys: Vec<String>,
    default_provider: String,
    search_active: bool,
    search_query: String,
    /// Providers that have reported in at least once. Once all are in, initial load is complete.
    seen_providers: std::collections::HashSet<String>,
    /// True once all providers have reported their first results.
    initial_load_complete: bool,
    /// True once user has manually pressed up/down. Prevents selection reset on refresh.
    user_navigated: bool,
    /// Sessions archived locally this cycle — filtered out until supervisor confirms.
    pending_archives: Vec<String>,
    /// Which filtered indices had a semantic match boost (for ✨ indicator).
    semantic_matches: std::collections::HashSet<usize>,
    /// Semantic plugin (shared with background indexer). Always use try_lock() — never block UI.
    semantic: std::sync::Arc<std::sync::Mutex<crate::search::SemanticPlugin>>,
    /// Last known semantic status (updated from try_lock, avoids blocking on status read).
    semantic_status_cache: crate::search::SemanticStatus,
}

impl App {
    pub fn new(provider_keys: Vec<String>, default_provider: String) -> Self {
        let mut list_state = ListState::default();
        // No selection until all providers report in
        list_state.select(None);

        // Semantic plugin loaded in background thread (never blocks UI)
        let semantic = std::sync::Arc::new(std::sync::Mutex::new(crate::search::SemanticPlugin::new()));
        {
            let sem_clone = semantic.clone();
            std::thread::spawn(move || {
                let cache_dir = dirs::data_local_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join("agent-session-tui")
                    .join("models");
                std::fs::create_dir_all(&cache_dir).ok();
                if let Ok(mut plugin) = sem_clone.lock() {
                    plugin.try_load(&cache_dir.to_string_lossy());
                }
            });
        }

        let provider_count = provider_keys.len();
        Self {
            sessions: Vec::new(),
            hidden_sessions: Vec::new(),
            filtered_indices: Vec::new(),
            selected_index: 0,
            list_state,
            focus: Focus::SessionList,
            view_mode: ViewMode::Active,
            status_message: format!("Loading {} providers...", provider_count),
            should_quit: false,
            default_provider,
            provider_keys,
            search_active: false,
            search_query: String::new(),
            seen_providers: std::collections::HashSet::new(),
            initial_load_complete: false,
            user_navigated: false,
            pending_archives: Vec::new(),
            semantic_matches: std::collections::HashSet::new(),
            semantic,
            semantic_status_cache: crate::search::SemanticStatus::Unavailable,
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
            let sem = if query.len() >= 5 {
                self.semantic.try_lock().ok()
            } else {
                None
            };
            let sem_ref = sem.as_deref();
            let view = self.current_view_sessions();
            let results = crate::search::ranked_search(view, &query, sem_ref);
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

        let tick_rate = std::time::Duration::from_millis(100);

        loop {
            // Update semantic status (try_lock: never blocks if indexer holds the lock)
            if let Ok(sem) = self.semantic.try_lock() {
                self.semantic_status_cache = sem.status().clone();
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
                    SupervisorEvent::SessionsUpdated { mut active, mut hidden } => {
                        // Filter out sessions that were just archived locally
                        if !self.pending_archives.is_empty() {
                            let mut moved = Vec::new();
                            active.retain(|s| {
                                let key = format!("{}:{}", s.provider_name, s.provider_session_id);
                                if self.pending_archives.contains(&key) {
                                    moved.push(s.clone());
                                    false
                                } else {
                                    true
                                }
                            });
                            hidden.extend(moved);
                            self.pending_archives.retain(|k| {
                                !hidden.iter().any(|s| format!("{}:{}", s.provider_name, s.provider_session_id) == *k)
                            });
                        }

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

                        // Track which providers have reported in
                        for s in &active {
                            self.seen_providers.insert(s.provider_name.clone());
                        }
                        for s in &hidden {
                            self.seen_providers.insert(s.provider_name.clone());
                        }
                        let all_providers_in = self.provider_keys.iter()
                            .all(|k| self.seen_providers.contains(k));

                        // First time all providers report → complete initial load
                        if all_providers_in && !self.initial_load_complete {
                            self.initial_load_complete = true;
                        }

                        if data_changed {
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

                            if set_changed || !self.search_active {
                                self.apply_filter();

                                if self.initial_load_complete && !self.user_navigated {
                                    self.selected_index = 0;
                                    self.list_state.select(Some(0));
                                } else if let Some((prev_provider, prev_id)) = &prev_selected_id {
                                    // User navigated → restore their position
                                    let view = self.current_view_sessions();
                                    if let Some(pos) = self.filtered_indices.iter().position(|&idx| {
                                        let s = &view[idx];
                                        &s.provider_name == prev_provider && &s.provider_session_id == prev_id
                                    }) {
                                        self.selected_index = pos;
                                        self.list_state.select(Some(pos));
                                    }
                                } else if !self.initial_load_complete {
                                    // Still loading — no selection
                                    self.list_state.select(None);
                                }
                            }

                            // Background semantic indexing
                            let sem_clone = self.semantic.clone();
                            let all_sessions: Vec<Session> = self.sessions.clone();
                            std::thread::spawn(move || {
                                if let Ok(mut sem) = sem_clone.lock() {
                                    if sem.lib.is_some() {
                                        sem.index_sessions(&all_sessions);
                                    }
                                }
                            });
                        } else {
                            // Data unchanged but check initial load completion
                            if all_providers_in && self.list_state.selected().is_none() {
                                self.selected_index = 0;
                                self.list_state.select(Some(0));
                            }
                        }

                        let now = chrono::Local::now().format("%H:%M:%S");
                        let shown = self.filtered_indices.len();
                        let total = match self.view_mode {
                            ViewMode::Active => active_count,
                            ViewMode::Hidden => hidden_count,
                        };
                        let mode_label = match self.view_mode {
                            ViewMode::Active => "active",
                            ViewMode::Hidden => "hidden",
                        };
                        self.status_message = format!(
                            "{}/{} {} · {} hidden · refreshed {}",
                            shown, total, mode_label, hidden_count, now
                        );

                        // Trigger background semantic indexing for new/changed sessions
                        let sem_clone = self.semantic.clone();
                        let all_sessions: Vec<Session> = self.sessions.clone();
                        std::thread::spawn(move || {
                            if let Ok(mut sem) = sem_clone.lock() {
                                if sem.lib.is_some() {
                                    sem.index_sessions(&all_sessions);
                                }
                            }
                        });
                    }
                    SupervisorEvent::Error(e) => {
                        self.status_message = format!("Error: {}", e);
                    }
                }
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
                KeyCode::Up
                    if self.selected_index > 0 => {
                        self.selected_index -= 1;
                        self.list_state.select(Some(self.selected_index));
                        self.user_navigated = true;
                }
                KeyCode::Down
                    if self.selected_index + 1 < self.filtered_indices.len() => {
                        self.selected_index += 1;
                        self.list_state.select(Some(self.selected_index));
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
                        self.user_navigated = true;
                }
                KeyCode::Down | KeyCode::Char('j')
                    if self.selected_index + 1 < self.filtered_indices.len() => {
                        self.selected_index += 1;
                        self.list_state.select(Some(self.selected_index));
                        self.user_navigated = true;
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
                        self.status_message = format!("Launching new {} session...", key);
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
                        let _ = cmd_tx.send(SupervisorCommand::ArchiveSession {
                            provider_session_id: psid.clone(),
                            provider_key: pname.clone(),
                        });
                        // Track locally so incoming refreshes don't put it back
                        self.pending_archives.push(key);
                        // Instantly move from active to hidden
                        if self.view_mode == ViewMode::Active {
                            if let Some(&idx) = self.filtered_indices.get(self.selected_index) {
                                if idx < self.sessions.len() {
                                    let removed = self.sessions.remove(idx);
                                    self.hidden_sessions.insert(0, removed);
                                    self.apply_filter();
                                }
                            }
                        }
                        self.status_message = format!("Archived: {}", crate::util::short_id(&psid, 8));
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
                    self.apply_filter();
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
                Constraint::Min(10),   // Session list
                Constraint::Length(1), // Status bar
            ])
            .split(f.area());

        self.draw_title_bar(f, chunks[0]);
        self.draw_session_list(f, chunks[1]);
        self.draw_status_bar(f, chunks[2]);
    }

    fn draw_title_bar(&self, f: &mut Frame, area: Rect) {
        let hl = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);

        if self.search_active {
            let title = Paragraph::new(Line::from(vec![
                Span::styled(" Search ", Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::raw("  "),
                Span::styled("⏎", hl),
                Span::raw(" open  "),
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
        // Title can use most of the row width. Reserve space for:
        //   badge(3) + provider(6) + spaces(2) + state/age suffix(~20) + border(2)
        let title_max = (area.width as usize).saturating_sub(35).max(20);
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
                    truncate_str_safe(&s.title, title_max)
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
                    Span::raw(" "),
                    Span::styled(
                        format!("{} · {}", s.state.label(), age),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]);

                ListItem::new(vec![line])
            })
            .collect();

        let border_style = Style::default().fg(Color::Cyan);

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

    fn draw_status_bar(&self, f: &mut Frame, area: Rect) {
        let view_hint = match self.view_mode {
            ViewMode::Active => "Shift+Tab: show archived",
            ViewMode::Hidden => "Shift+Tab: show active",
        };
        let sem_indicator = match &self.semantic_status_cache {
            crate::search::SemanticStatus::Ready { count } => Span::styled(
                format!("🧠 {} ", count),
                Style::default().fg(Color::Green),
            ),
            crate::search::SemanticStatus::Indexing { done, total } => Span::styled(
                format!("⏳ {}/{} ", done, total),
                Style::default().fg(Color::Yellow),
            ),
            crate::search::SemanticStatus::Failed(_) => Span::styled("⚠ Semantic failed ", Style::default().fg(Color::Red)),
            crate::search::SemanticStatus::Unavailable => Span::raw(""),
        };
        let status = Paragraph::new(Line::from(vec![
            sem_indicator,
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

#[allow(dead_code)]
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
#[cfg(test)]
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
        let mut app = App::new(vec!["copilot".into()], "copilot".into());
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
        let app = App::new(vec!["copilot".into()], "copilot".into());
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

    fn ui_source() -> String {
        fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/ui/mod.rs"))
            .expect("should read ui/mod.rs")
    }

    fn code_section() -> String {
        let src = ui_source();
        src.split("#[cfg(test)]").next().unwrap_or(&src).to_string()
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
    fn no_clear_widget() {
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
}
