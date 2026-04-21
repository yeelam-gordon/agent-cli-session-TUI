#![allow(dead_code)]

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
        active: Vec<Session>,
        hidden: Vec<Session>,
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
    FocusSession {
        tab_title: Option<String>,
        title: String,
        provider_session_id: String,
    },
    Shutdown,
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
        // Initial scan
        if let Err(e) = self.scan_and_notify(&event_tx) {
            crate::log::error(&format!("Initial scan failed: {}", e));
            let _ = event_tx.send(SupervisorEvent::Error(e.to_string()));
        }

        let mut interval = tokio::time::interval(self.poll_interval);

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let scan_start = std::time::Instant::now();
                    if let Err(e) = self.scan_and_notify(&event_tx) {
                        crate::log::warn(&format!("Scan error: {}", e));
                        let _ = event_tx.send(SupervisorEvent::Error(e.to_string()));
                    }
                    crate::log::info(&format!("Scan cycle: {:?}", scan_start.elapsed()));
                }
                Some(cmd) = cmd_rx.recv() => {
                    let cmd_start = std::time::Instant::now();
                    match cmd {
                        SupervisorCommand::Shutdown => break,
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
                            self.handle_archive(&provider_key, &provider_session_id, &event_tx);
                            let _ = self.scan_and_notify(&event_tx);
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

    fn scan_and_notify(&self, event_tx: &mpsc::UnboundedSender<SupervisorEvent>) -> Result<()> {
        let providers: Vec<_> = self.registry.providers().iter()
            .filter(|p| p.capabilities().supports_discovery)
            .collect();

        let archive = self.archive.lock().ok();
        let mut all_active: Vec<Session> = Vec::new();
        let mut all_hidden: Vec<Session> = Vec::new();

        // Scan providers in parallel, sending progressive updates as each completes
        std::thread::scope(|s| {
            let (tx, rx) = std::sync::mpsc::channel::<Vec<Session>>();

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
                    let _ = tx.send(sessions);
                });
            }
            drop(tx); // Close sender so rx iterator ends when all threads finish

            // Process results as each provider completes — send progressive updates
            for sessions in rx {
                for s in sessions {
                    let is_archived = archive
                        .as_ref()
                        .map(|a| a.is_archived(&s.provider_name, &s.provider_session_id))
                        .unwrap_or(false);
                    if is_archived {
                        all_hidden.push(s);
                    } else {
                        all_active.push(s);
                    }
                }

                // Sort and send partial update — UI renders immediately
                all_active.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
                all_hidden.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
                let _ = event_tx.send(SupervisorEvent::SessionsUpdated {
                    active: all_active.clone(),
                    hidden: all_hidden.clone(),
                });
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
        _event_tx: &mpsc::UnboundedSender<SupervisorEvent>,
    ) {
        if let Ok(mut archive) = self.archive.lock() {
            let _ = archive.archive(provider_key, provider_session_id);
        }
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
