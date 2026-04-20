//! Provider-agnostic test scenarios.
//!
//! These work with ANY Provider implementation. Plugin-specific tests
//! can call these + add their own custom scenarios.

use std::time::{Duration, Instant};

use super::{trunc, TestRunner};
use crate::models::*;
use crate::provider::Provider;

// ───────────────────────────────────────────────────────────────────────────
// Scenario: DISCOVER — scan sessions + processes + reconcile
// ───────────────────────────────────────────────────────────────────────────

pub fn discover(r: &mut TestRunner, p: &dyn Provider) {
    println!("\n▸ DISCOVER — scan existing {} sessions", r.provider_name);

    // Persisted sessions
    let t = Instant::now();
    let sessions = match p.discover_sessions() {
        Ok(s) => {
            r.record(
                "persisted_sessions",
                !s.is_empty(),
                &format!("{} sessions found", s.len()),
                t.elapsed(),
            );
            for s in s.iter().take(5) {
                println!(
                    "    {} {} — {}",
                    s.state.badge(),
                    &s.provider_session_id[..8.min(s.provider_session_id.len())],
                    trunc(&s.title, 50)
                );
            }
            if s.len() > 5 {
                println!("    ... and {} more", s.len() - 5);
            }
            s
        }
        Err(e) => {
            r.record("persisted_sessions", false, &format!("{e}"), t.elapsed());
            return;
        }
    };

    // Reconciliation (discover + match processes)
    let t = Instant::now();
    let mut sessions = sessions;
    if let Err(e) = p.match_processes(&mut sessions) {
        r.record("match_processes", false, &format!("{e}"), t.elapsed());
        return;
    }

    let running = sessions
        .iter()
        .filter(|s| s.state.process == ProcessState::Running)
        .count();
    let waiting = sessions
        .iter()
        .filter(|s| s.state.interaction == InteractionState::WaitingInput)
        .count();
    let busy = sessions
        .iter()
        .filter(|s| s.state.interaction == InteractionState::Busy)
        .count();
    let orphan = sessions
        .iter()
        .filter(|s| s.state.health == HealthState::Orphaned)
        .count();

    r.record(
        "match_processes",
        true,
        &format!("{running} running ({busy} busy, {waiting} waiting), {orphan} orphaned"),
        t.elapsed(),
    );

    // Show running sessions
    for s in sessions
        .iter()
        .filter(|s| s.state.process == ProcessState::Running)
    {
        println!(
            "    {} {} {} [{:?}] PID={:?} — {}",
            s.state.badge(),
            s.state.label(),
            &s.provider_session_id[..8.min(s.provider_session_id.len())],
            s.state.confidence,
            s.pid,
            trunc(&s.title, 40)
        );
    }

    // Assertions
    let running_with_pid = sessions
        .iter()
        .filter(|s| s.state.process == ProcessState::Running && s.pid.is_some())
        .count();
    r.record(
        "running_has_pid",
        running == running_with_pid,
        &format!("{running_with_pid}/{running} running sessions have PID"),
        Duration::ZERO,
    );

    let waiting_confident = sessions
        .iter()
        .filter(|s| {
            s.state.interaction == InteractionState::WaitingInput
                && s.state.confidence >= Confidence::Medium
        })
        .count();
    r.record(
        "waiting_confidence",
        waiting == 0 || waiting_confident > 0,
        &format!("{waiting_confident}/{waiting} waiting have ≥Medium confidence"),
        Duration::ZERO,
    );
}

// ───────────────────────────────────────────────────────────────────────────
// Scenario: GRACEFUL — verify clean exits are Resumable
// ───────────────────────────────────────────────────────────────────────────

pub fn graceful(r: &mut TestRunner, p: &dyn Provider) {
    println!("\n▸ GRACEFUL — verify cleanly-exited sessions");

    let t = Instant::now();
    let mut sessions = p.discover_sessions().unwrap_or_default();
    let _ = p.match_processes(&mut sessions);

    let clean: Vec<_> = sessions
        .iter()
        .filter(|s| {
            s.state.process != ProcessState::Running
                && s.state.persistence == PersistenceState::Resumable
                && s.state.health == HealthState::Clean
        })
        .collect();

    r.record(
        "clean_resumable",
        !clean.is_empty(),
        &format!("{} cleanly-exited resumable sessions", clean.len()),
        t.elapsed(),
    );

    for s in clean.iter().take(3) {
        println!(
            "    {} {} — {}",
            s.state.badge(),
            &s.provider_session_id[..8.min(s.provider_session_id.len())],
            trunc(&s.title, 40)
        );
    }

    let orphaned: Vec<_> = sessions
        .iter()
        .filter(|s| s.state.health == HealthState::Orphaned)
        .collect();

    r.record(
        "orphaned_count",
        true,
        &format!("{} orphaned sessions", orphaned.len()),
        Duration::ZERO,
    );

    let orphan_with_pid = orphaned.iter().filter(|s| s.pid.is_some()).count();
    r.record(
        "orphaned_no_pid",
        orphan_with_pid == 0,
        &format!(
            "{orphan_with_pid}/{} orphaned have PID (should be 0)",
            orphaned.len()
        ),
        Duration::ZERO,
    );

    let with_summary = clean.iter().filter(|s| !s.summary.is_empty()).count();
    r.record(
        "resumable_has_summary",
        clean.is_empty() || with_summary > 0,
        &format!("{with_summary}/{} resumable have summaries", clean.len()),
        Duration::ZERO,
    );
}

// ───────────────────────────────────────────────────────────────────────────
// Scenario: LAUNCH — start a session, poll for state transitions
// ───────────────────────────────────────────────────────────────────────────

pub fn launch(r: &mut TestRunner, p: &dyn Provider, config: &crate::config::ProviderConfig) {
    println!("\n▸ LAUNCH — start session, poll for Running→Busy→Waiting");

    let cwd = std::env::current_dir().unwrap_or_default();
    let mut cmd = vec![config.command.clone()];
    cmd.extend(config.default_args.iter().cloned());
    r.record(
        "build_command",
        true,
        &format!("{}", cmd.join(" ")),
        Duration::ZERO,
    );

    let before: Vec<String> = p
        .discover_sessions()
        .unwrap_or_default()
        .iter()
        .map(|s| s.provider_session_id.clone())
        .collect();

    println!("    Launching: {}", cmd.join(" "));
    #[cfg(windows)]
    {
        let cmd_str = cmd.join(" ");
        let _ = std::process::Command::new("wt")
            .args([
                "-w",
                "0",
                "new-tab",
                "--startingDirectory",
                &cwd.to_string_lossy(),
                "cmd",
                "/k",
                &cmd_str,
            ])
            .spawn();
    }

    println!("    Polling every 2s for 60s...");
    let start = Instant::now();
    let mut saw_running = false;
    let mut saw_busy = false;
    let mut saw_waiting = false;
    let mut transitions: Vec<(u64, String)> = Vec::new();

    while start.elapsed() < Duration::from_secs(60) {
        std::thread::sleep(Duration::from_secs(2));

        let mut sessions = p.discover_sessions().unwrap_or_default();
        let _ = p.match_processes(&mut sessions);

        let target = sessions
            .iter()
            .filter(|s| !before.contains(&s.provider_session_id))
            .filter(|s| s.state.process == ProcessState::Running)
            .max_by_key(|s| s.updated_at.clone())
            .or_else(|| {
                sessions
                    .iter()
                    .filter(|s| s.state.process == ProcessState::Running)
                    .max_by_key(|s| s.updated_at.clone())
            });

        if let Some(s) = target {
            let st = format!(
                "{} {} [{:?}]",
                s.state.badge(),
                s.state.label(),
                s.state.confidence
            );
            if transitions.last().map(|(_, t)| t != &st).unwrap_or(true) {
                let e = start.elapsed().as_secs();
                println!("    +{e}s: {st}");
                transitions.push((e, st));
            }
            if s.state.process == ProcessState::Running {
                saw_running = true;
            }
            if s.state.interaction == InteractionState::Busy {
                saw_busy = true;
            }
            if s.state.interaction == InteractionState::WaitingInput {
                saw_waiting = true;
            }
        }
        if saw_running && saw_waiting {
            break;
        }
    }

    r.record(
        "detect_running",
        saw_running,
        if saw_running {
            "Detected Running"
        } else {
            "Never saw Running"
        },
        start.elapsed(),
    );
    r.record(
        "detect_busy",
        saw_busy,
        if saw_busy {
            "Detected Busy"
        } else {
            "Busy not observed (may be too fast)"
        },
        Duration::ZERO,
    );
    r.record(
        "detect_waiting",
        saw_waiting,
        if saw_waiting {
            "Detected WaitingInput"
        } else {
            "WaitingInput not detected"
        },
        Duration::ZERO,
    );
}

// ───────────────────────────────────────────────────────────────────────────
// Scenario: KILL — kill a process, verify state change
// ───────────────────────────────────────────────────────────────────────────

pub fn kill(r: &mut TestRunner, p: &dyn Provider) {
    println!("\n▸ KILL — kill running process, verify state transition");

    let mut sessions = p.discover_sessions().unwrap_or_default();
    let _ = p.match_processes(&mut sessions);

    let target = sessions
        .iter()
        .find(|s| s.state.process == ProcessState::Running && s.pid.is_some());

    let Some(s) = target else {
        r.record(
            "find_target",
            false,
            "No running session with PID",
            Duration::ZERO,
        );
        return;
    };

    let pid = s.pid.unwrap();
    let sid = s.provider_session_id.clone();
    println!(
        "    Target: {} PID={} — {}",
        &sid[..8.min(sid.len())],
        pid,
        trunc(&s.title, 40)
    );
    println!("    ⚠ Will kill PID {pid}. Press Enter or Ctrl+C.");
    let mut buf = String::new();
    let _ = std::io::stdin().read_line(&mut buf);

    let t = Instant::now();
    let killed = {
        let mut sys = sysinfo::System::new();
        sys.refresh_processes(
            sysinfo::ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(pid)]),
            true,
        );
        sys.process(sysinfo::Pid::from_u32(pid))
            .map(|p| p.kill())
            .unwrap_or(false)
    };
    r.record("kill_process", killed, &format!("PID {pid}"), t.elapsed());
    if !killed {
        return;
    }

    println!("    Waiting 5s...");
    std::thread::sleep(Duration::from_secs(5));

    let t = Instant::now();
    let mut sessions = p.discover_sessions().unwrap_or_default();
    let _ = p.match_processes(&mut sessions);

    match sessions.iter().find(|s| s.provider_session_id == sid) {
        Some(s) => {
            let not_running = s.state.process != ProcessState::Running;
            println!(
                "    After: {} {} {:?} {:?}",
                s.state.badge(),
                s.state.label(),
                s.state.health,
                s.state.persistence
            );
            r.record(
                "not_running_after_kill",
                not_running,
                &format!("{:?}", s.state.process),
                t.elapsed(),
            );
            r.record(
                "is_resumable_or_orphaned",
                s.state.persistence == PersistenceState::Resumable
                    || s.state.health == HealthState::Orphaned,
                &format!(
                    "persistence={:?} health={:?}",
                    s.state.persistence, s.state.health
                ),
                Duration::ZERO,
            );
        }
        None => r.record("session_exists", false, "Gone after kill!", t.elapsed()),
    }
}
