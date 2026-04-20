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
use unicode_width::UnicodeWidthStr;

use crate::models::{InteractionState, PersistenceState, ProcessState, Session};
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
    /// Sessions archived locally this cycle — filtered out until supervisor confirms.
    pending_archives: Vec<String>,
}

impl App {
    pub fn new(provider_keys: Vec<String>, default_provider: String, log_max_lines: usize) -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
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
            status_message: String::new(),
            should_quit: false,
            default_provider,
            provider_keys,
            detail_scroll: 0,
            search_active: false,
            search_query: String::new(),
            log_max_lines,
            pending_archives: Vec::new(),
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
    fn apply_filter(&mut self) {
        let view = self.current_view_sessions();
        let query = self.search_query.to_lowercase();
        if query.is_empty() {
            self.filtered_indices = (0..view.len()).collect();
        } else {
            self.filtered_indices = view
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    s.title.to_lowercase().contains(&query)
                        || s.summary.to_lowercase().contains(&query)
                        || s.provider_session_id.to_lowercase().contains(&query)
                        || s.cwd.to_string_lossy().to_lowercase().contains(&query)
                        || s.provider_name.to_lowercase().contains(&query)
                })
                .map(|(i, _)| i)
                .collect();
        }
        if self.selected_index >= self.filtered_indices.len() && !self.filtered_indices.is_empty() {
            self.selected_index = 0;
        }
        self.list_state.select(Some(self.selected_index));
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
            // Draw
            terminal.draw(|f| {
                self.draw(f);
            })?;

            // Handle events (non-blocking with timeout)
            if event::poll(tick_rate)? {
                match event::read()? {
                    Event::Key(key) => {
                        // Only handle Press to avoid double/triple on Windows
                        if key.kind == KeyEventKind::Press {
                            self.handle_key(key, &cmd_tx);
                        }
                    }
                    _ => {}
                }
            }

            // Drain supervisor events
            while let Ok(ev) = event_rx.try_recv() {
                match ev {
                    SupervisorEvent::SessionsUpdated { mut active, mut hidden } => {
                        // Filter out sessions that were just archived locally
                        // (supervisor may not have processed the command yet)
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
                            // Clear pending once supervisor confirms (session is in hidden list)
                            self.pending_archives.retain(|k| {
                                !hidden.iter().any(|s| format!("{}:{}", s.provider_name, s.provider_session_id) == *k)
                            });
                        }

                        let active_count = active.len();
                        let hidden_count = hidden.len();

                        // Preserve selection
                        let prev_selected_id = self
                            .selected_session()
                            .map(|s| (s.provider_name.clone(), s.provider_session_id.clone()));

                        self.sessions = active;
                        self.hidden_sessions = hidden;
                        self.apply_filter();

                        // Restore selection
                        if let Some((prev_provider, prev_id)) = prev_selected_id {
                            let view = self.current_view_sessions();
                            if let Some(pos) = self.filtered_indices.iter().position(|&idx| {
                                let s = &view[idx];
                                s.provider_name == prev_provider && s.provider_session_id == prev_id
                            }) {
                                self.selected_index = pos;
                                self.list_state.select(Some(pos));
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

    fn handle_key(&mut self, key: KeyEvent, cmd_tx: &mpsc::UnboundedSender<SupervisorCommand>) {
        // Search mode handles its own keys
        if self.search_active {
            match key.code {
                KeyCode::Esc => {
                    self.search_active = false;
                    self.search_query.clear();
                    self.apply_filter();
                }
                KeyCode::Enter => {
                    // Exit search and resume the selected session
                    self.search_active = false;
                    if let Some(session) = self.selected_session() {
                        let psid = session.provider_session_id.clone();
                        let pname = session.provider_name.clone();
                        let title = session.title.clone();
                        let scwd = session.cwd.to_string_lossy().to_string();
                        let _ = cmd_tx.send(SupervisorCommand::ResumeSession {
                            provider_session_id: psid.clone(),
                            provider_key: pname,
                            session_cwd: scwd,
                        });
                        self.status_message = format!(
                            "▶ Resuming: {} ({})",
                            title, &psid[..8.min(psid.len())]
                        );
                        self.log_lines.push(format!(
                            "Resuming: {} ({})",
                            title, &psid[..8.min(psid.len())]
                        ));
                    }
                }
                KeyCode::Up => {
                    // Navigate results while still in search mode
                    if self.selected_index > 0 {
                        self.selected_index -= 1;
                        self.list_state.select(Some(self.selected_index));
                    }
                }
                KeyCode::Down => {
                    if self.selected_index + 1 < self.filtered_indices.len() {
                        self.selected_index += 1;
                        self.list_state.select(Some(self.selected_index));
                    }
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

        // Global shortcuts
        match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('c')) | (_, KeyCode::Char('q')) => {
                self.should_quit = true;
                return;
            }
            _ => {}
        }

        match self.focus {
            Focus::SessionList => match key.code {
                KeyCode::Esc => {
                    // Clear search filter if one is active
                    if !self.search_query.is_empty() {
                        self.search_query.clear();
                        self.apply_filter();
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if self.selected_index > 0 {
                        self.selected_index -= 1;
                        self.list_state.select(Some(self.selected_index));
                        self.detail_scroll = 0;
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if self.selected_index + 1 < self.filtered_indices.len() {
                        self.selected_index += 1;
                        self.list_state.select(Some(self.selected_index));
                        self.detail_scroll = 0;
                    }
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
                    if let Some(session) = self.selected_session() {
                        let psid = session.provider_session_id.clone();
                        let pname = session.provider_name.clone();
                        let title = session.title.clone();
                        let scwd = session.cwd.to_string_lossy().to_string();
                        let _ = cmd_tx.send(SupervisorCommand::ResumeSession {
                            provider_session_id: psid.clone(),
                            provider_key: pname,
                            session_cwd: scwd,
                        });
                        self.status_message = format!(
                            "▶ Resuming: {} ({})",
                            title,
                            &psid[..8.min(psid.len())]
                        );
                        self.log_lines.push(format!(
                            "Resuming: {} ({})",
                            title,
                            &psid[..8.min(psid.len())]
                        ));
                    }
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
                        self.log_lines
                            .push(format!("Archived: {}", &psid[..8.min(psid.len())]));
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
                    self.detail_scroll += 1;
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
                KeyCode::Down => {
                    if self.log_scroll + 1 < self.log_lines.len() {
                        self.log_scroll += 1;
                    }
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
        let title = if self.search_active {
            Paragraph::new(Line::from(vec![
                Span::styled(" Agent Session Manager ", Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw("  ↑↓ browse  "),
                Span::styled("⏎", hl),
                Span::raw(" open  "),
                Span::styled("Esc", hl),
                Span::raw(" cancel"),
            ]))
        } else {
            Paragraph::new(Line::from(vec![
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
            ]))
        };
        f.render_widget(title, area);
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
                let short_id = &s.provider_session_id[..8.min(s.provider_session_id.len())];

                let title_display = if s.title.is_empty() {
                    short_id.to_string()
                } else {
                    truncate_str_safe(&s.title, 25)
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
                for summary_line in session.summary.lines().take(15) {
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

            // Pad every line with trailing spaces to fill the panel width.
            // Without Wrap, lines longer than inner_width are truncated by ratatui
            // but we still need padding for shorter lines to overwrite stale content.
            // Tabs and zero-width chars are replaced to ensure accurate width calc.
            let inner_width = area.width.saturating_sub(2) as usize; // minus borders
            let inner_height = area.height.saturating_sub(2) as usize;
            for line in &mut lines {
                // Replace tabs with spaces in each span (tabs have width 0 but render wider)
                for span in &mut line.spans {
                    if span.content.contains('\t') {
                        span.content = span.content.replace('\t', "    ").into();
                    }
                }
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
            // Pad remaining rows with full-width space lines
            while lines.len() < inner_height {
                lines.push(Line::from(" ".repeat(inner_width)));
            }

            let detail = Paragraph::new(lines)
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
            ViewMode::Active => "Shift+Tab: show archived",
            ViewMode::Hidden => "Shift+Tab: show active",
        };
        let status = Paragraph::new(Line::from(vec![
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
}
