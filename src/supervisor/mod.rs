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
    /// Full session list refresh: (active_sessions, hidden_sessions).
    /// Hidden = archived + empty (filtered out during discovery).
    SessionsUpdated {
        active: Vec<Session>,
        hidden: Vec<Session>,
    },
    /// A single session's state changed.
    SessionStateChanged { provider_session_id: String },
    /// Error during background work.
    Error(String),
}

/// Commands from the TUI to the supervisor.
#[derive(Debug)]
pub enum SupervisorCommand {
    /// Force a full scan+reconcile now.
    Refresh,
    /// Launch a new session.
    NewSession {
        provider_key: String,
        cwd: String,
    },
    /// Resume an existing session.
    ResumeSession {
        provider_session_id: String,
        provider_key: String,
        /// The session's original working directory (needed by CLIs like Claude
        /// that tie sessions to a specific directory).
        session_cwd: String,
    },
    /// Kill a running session's process.
    KillSession {
        provider_session_id: String,
        provider_key: String,
    },
    /// Archive a session.
    ArchiveSession {
        provider_session_id: String,
        provider_key: String,
    },
    /// Shut down the supervisor.
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
                    if let Err(e) = self.scan_and_notify(&event_tx) {
                        crate::log::warn(&format!("Scan error: {}", e));
                        let _ = event_tx.send(SupervisorEvent::Error(e.to_string()));
                    }
                }
                Some(cmd) = cmd_rx.recv() => {
                    match cmd {
                        SupervisorCommand::Shutdown => break,
                        SupervisorCommand::Refresh => {
                            if let Err(e) = self.scan_and_notify(&event_tx) {
                                let _ = event_tx.send(SupervisorEvent::Error(e.to_string()));
                            }
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
                            self.handle_archive(&provider_key, &provider_session_id, &event_tx);
                        }
                    }
                }
            }
        }
    }

    fn scan_and_notify(
        &self,
        event_tx: &mpsc::UnboundedSender<SupervisorEvent>,
    ) -> Result<()> {
        let mut active_sessions = Vec::new();
        let mut hidden_sessions = Vec::new();

        for provider in self.registry.providers() {
            if !provider.capabilities().supports_discovery {
                continue;
            }

            let mut sessions = provider.discover_sessions().unwrap_or_default();
            let _ = provider.match_processes(&mut sessions);

            // Split into active vs hidden (archived)
            let archive = self.archive.lock().ok();
            for s in sessions {
                let is_archived = archive.as_ref()
                    .map(|a| a.is_archived(&s.provider_name, &s.provider_session_id))
                    .unwrap_or(false);
                if is_archived {
                    hidden_sessions.push(s);
                } else {
                    active_sessions.push(s);
                }
            }
        }

        // Also discover empty/filtered sessions for the hidden view
        // (providers already skip these, but we can mark archived ones)

        active_sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        hidden_sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        let _ = event_tx.send(SupervisorEvent::SessionsUpdated {
            active: active_sessions,
            hidden: hidden_sessions,
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
            let _ = event_tx.send(SupervisorEvent::Error(
                format!("Provider '{}' not in config", provider_key),
            ));
            return;
        };

        let effective_cwd = config.startup_dir.as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| cwd.to_string());
        let launch_method = config.launch_method.as_str();
        let wt_profile = config.wt_profile.as_deref();

        let cmd = Self::build_new_command(config);
        crate::log::info(&format!("Launching new {}: {:?} in {}", provider_key, cmd, effective_cwd));
        if let Err(e) = launch_in_terminal(&cmd, &effective_cwd, launch_method, wt_profile) {
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
            let _ = event_tx.send(SupervisorEvent::Error(
                format!("Provider '{}' not in config", provider_key),
            ));
            return;
        };

        // Use the session's original CWD (critical for CLIs like Claude that
        // tie sessions to directories). Fall back to config startup_dir, then ".".
        let effective_cwd = if !session_cwd.is_empty() && session_cwd != "." {
            session_cwd.to_string()
        } else {
            config.startup_dir.as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| ".".to_string())
        };
        let launch_method = config.launch_method.as_str();
        let wt_profile = config.wt_profile.as_deref();

        let cmd = Self::build_resume_command(config, provider_session_id);
        crate::log::info(&format!("Resuming {} session {} in {:?}: {:?}",
            provider_key, provider_session_id, effective_cwd, cmd));
        if let Err(e) = launch_in_terminal(&cmd, &effective_cwd, launch_method, wt_profile) {
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

/// Launch a command in a new terminal window using the configured method.
fn launch_in_terminal(cmd: &[String], cwd: &str, launch_method: &str, wt_profile: Option<&str>) -> Result<()> {
    let cmd_str = cmd.join(" ");

    #[cfg(windows)]
    {
        match launch_method {
            "wt" => {
                // Windows Terminal: new tab in the EXISTING window.
                // `-w 0` targets the most recently active WT window
                // (without it, wt opens a brand new window when called
                // from a process outside Windows Terminal).
                let mut args = vec!["-w".to_string(), "0".to_string(), "new-tab".to_string()];
                if let Some(profile) = wt_profile {
                    args.push("--profile".to_string());
                    args.push(profile.to_string());
                }
                args.push("--startingDirectory".to_string());
                args.push(cwd.to_string());
                args.push("cmd".to_string());
                args.push("/k".to_string());
                args.push(cmd_str.clone());

                let result = std::process::Command::new("wt")
                    .args(&args)
                    .spawn();

                match result {
                    Ok(_) => Ok(()),
                    Err(_) => {
                        // Fallback to cmd if wt not available
                        launch_in_terminal(cmd, cwd, "cmd", None)
                    }
                }
            }
            "pwsh" => {
                std::process::Command::new("pwsh")
                    .args(["-NoExit", "-Command", &cmd_str])
                    .current_dir(cwd)
                    .spawn()?;
                Ok(())
            }
            _ => {
                // "cmd" or any other fallback
                std::process::Command::new("cmd")
                    .args(["/c", "start", "cmd", "/k", &cmd_str])
                    .current_dir(cwd)
                    .spawn()?;
                Ok(())
            }
        }
    }

    #[cfg(not(windows))]
    {
        let _ = (launch_method, wt_profile); // suppress unused warnings
        let shell_cmd = format!("cd {} && {}", cwd, cmd_str);
        std::process::Command::new("sh")
            .args(["-c", &format!("xterm -e '{}' &", shell_cmd)])
            .spawn()?;
        Ok(())
    }
}
