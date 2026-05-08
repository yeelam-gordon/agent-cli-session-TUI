//! Shared process discovery for all providers.
//!
//! Uses WMI (Win32_Process) to reliably get process command lines on Windows.
//! sysinfo often can't read command-line args due to access restrictions,
//! but WMI uses a privileged code path that works consistently.
//!
//! A global snapshot is cached so that multiple providers calling
//! `discover_processes` during the same scan cycle share ONE WMI query
//! instead of spawning 6 concurrent PowerShell processes (which can hang).

use std::collections::HashMap;
use std::sync::Mutex;

/// A discovered OS process with its full command line.
#[derive(Debug, Clone)]
pub struct ProcessEntry {
    #[allow(dead_code)]
    pub name: String,
    pub command_line: String,
}

/// Time-aware process cache with WMI cooldown.
struct ProcessCacheState {
    /// Cached process snapshot.
    snapshot: HashMap<u32, ProcessEntry>,
    /// When the snapshot was captured.
    captured_at: std::time::Instant,
    /// Whether the snapshot came from WMI (has cmdlines) or sysinfo-only.
    /// Kept on the struct as a diagnostic signal for future logging — read
    /// path may be added later, hence the `dead_code` allow.
    #[allow(dead_code)]
    has_wmi_data: bool,
    /// When the last WMI timeout occurred (for backoff).
    last_wmi_timeout: Option<std::time::Instant>,
}

static PROCESS_CACHE: Mutex<Option<ProcessCacheState>> = Mutex::new(None);

/// How long a successful snapshot is reused before refreshing.
const CACHE_TTL_SECS: u64 = 30;
/// How long to avoid WMI after a timeout (backoff).
#[cfg(windows)]
const WMI_BACKOFF_SECS: u64 = 300; // 5 minutes

/// Known provider executables for the combined WMI fallback query.
#[cfg(windows)]
const KNOWN_EXECUTABLES: &[&str] = &["copilot", "claude", "codex", "qwen", "gemini", "kimi", "node", "agency"];

/// WMI timeout in seconds. If WMI doesn't respond in this time, fall back to sysinfo.
#[cfg(windows)]
const WMI_TIMEOUT_SECS: u64 = 3;

/// Invalidate the global process cache so the next `discover_processes` call
/// re-queries the OS. Call this at the START of each scan cycle.
///
/// Respects TTL: only actually clears if the cache is older than CACHE_TTL_SECS.
/// WMI backoff state is always preserved.
pub fn invalidate_process_cache() {
    if let Ok(mut guard) = PROCESS_CACHE.lock() {
        if let Some(ref state) = *guard {
            if state.captured_at.elapsed().as_secs() < CACHE_TTL_SECS {
                return; // Still fresh — keep it
            }
        }
        // Preserve WMI backoff timestamp across invalidations
        let wmi_timeout = guard.as_ref().and_then(|s| s.last_wmi_timeout);
        if wmi_timeout.is_some() {
            if let Some(ref mut state) = *guard {
                // Clear snapshot but keep backoff
                state.snapshot.clear();
                state.captured_at = std::time::Instant::now() - std::time::Duration::from_secs(CACHE_TTL_SECS + 1);
            } else {
                *guard = None;
            }
        } else {
            *guard = None;
        }
    }
}

/// Discover processes whose name matches the filter (case-insensitive substring).
/// Returns a map of PID → ProcessEntry for easy lookup.
///
/// Strategy: sysinfo first (fast, ~150ms). If sysinfo returns empty command
/// lines (common on Windows), try WMI with a timeout — but respects a 5-minute
/// cooldown after WMI timeouts to avoid crashing the WMI service.
pub fn discover_processes(name_filter: &str) -> HashMap<u32, ProcessEntry> {
    let all_procs = {
        let mut guard = PROCESS_CACHE.lock().unwrap_or_else(|e| e.into_inner());

        // Return cached if still fresh
        let use_cached = guard.as_ref()
            .map(|s| !s.snapshot.is_empty() && s.captured_at.elapsed().as_secs() < CACHE_TTL_SECS)
            .unwrap_or(false);
        if use_cached {
            guard.as_ref().unwrap().snapshot.clone()
        } else {
            // Check WMI backoff state — only meaningful on Windows where WMI
            // is actually invoked. On non-Windows the variable would be unused.
            #[cfg(windows)]
            let wmi_backed_off = guard.as_ref()
                .and_then(|s| s.last_wmi_timeout)
                .map(|t| t.elapsed().as_secs() < WMI_BACKOFF_SECS)
                .unwrap_or(false);

            let start = std::time::Instant::now();
            let snapshot = discover_all_sysinfo();
            crate::log::info(&format!(
                "Process snapshot: {} processes in {:?} (sysinfo)",
                snapshot.len(), start.elapsed()
            ));

            #[cfg(windows)]
            let (snapshot, has_wmi, wmi_timed_out) = {
                let has_cmdlines = snapshot.values()
                    .take(20)
                    .filter(|e| !e.command_line.is_empty())
                    .count();
                if has_cmdlines < 5 && !snapshot.is_empty() && !wmi_backed_off {
                    crate::log::info(&format!(
                        "sysinfo cmdlines mostly empty, trying WMI ({}s timeout)",
                        WMI_TIMEOUT_SECS
                    ));
                    let wmi_start = std::time::Instant::now();
                    match discover_wmi_with_timeout() {
                        Some(wmi_result) => {
                            crate::log::info(&format!(
                                "WMI OK: {} processes in {:?}",
                                wmi_result.len(), wmi_start.elapsed()
                            ));
                            (wmi_result, true, false)
                        }
                        None => {
                            crate::log::warn(&format!(
                                "WMI timed out ({}s), backing off for {}s",
                                WMI_TIMEOUT_SECS, WMI_BACKOFF_SECS
                            ));
                            (snapshot, false, true)
                        }
                    }
                } else {
                    if wmi_backed_off {
                        crate::log::info("WMI in cooldown, using sysinfo name-only");
                    }
                    (snapshot, false, false)
                }
            };

            #[cfg(not(windows))]
            let (snapshot, has_wmi, wmi_timed_out) = (snapshot, false, false);

            let prev_wmi_timeout = guard.as_ref().and_then(|s| s.last_wmi_timeout);
            *guard = Some(ProcessCacheState {
                snapshot: snapshot.clone(),
                captured_at: std::time::Instant::now(),
                has_wmi_data: has_wmi,
                last_wmi_timeout: if wmi_timed_out {
                    Some(std::time::Instant::now())
                } else if has_wmi {
                    None // WMI succeeded — clear any previous backoff
                } else {
                    prev_wmi_timeout // Preserve existing backoff
                },
            });
            snapshot
        }
    };

    // Filter by name
    let filter_lower = name_filter.to_lowercase();
    all_procs
        .into_iter()
        .filter(|(_, entry)| {
            entry.name.to_lowercase().contains(&filter_lower)
                || entry.command_line.to_lowercase().contains(&filter_lower)
        })
        .collect()
}

/// Extract a flag's value from a command line string.
/// E.g., `extract_flag_value(cmd, "--session-id")` returns the value after `--session-id`.
pub fn extract_flag_value(command_line: &str, flag: &str) -> Option<String> {
    let parts: Vec<&str> = command_line.split_whitespace().collect();
    parts
        .windows(2)
        .find(|w| w[0].eq_ignore_ascii_case(flag))
        .map(|w| w[1].trim_matches('"').to_string())
}

// ---------------------------------------------------------------------------
// Windows: WMI-based discovery with timeout (reliable command-line reading)
// ---------------------------------------------------------------------------

/// Run a combined WMI query for all known provider executables with a timeout.
/// Returns None if WMI hangs or fails within the timeout.
#[cfg(windows)]
fn discover_wmi_with_timeout() -> Option<HashMap<u32, ProcessEntry>> {
    use std::io::Read;

    // Build combined filter for all known executables
    let conditions: Vec<String> = KNOWN_EXECUTABLES.iter()
        .flat_map(|exe| vec![
            format!("$_.Name -like '*{}*'", exe),
            format!("$_.CommandLine -like '*{}*'", exe),
        ])
        .collect();
    let where_clause = conditions.join(" -or ");

    let ps_script = format!(
        "Get-CimInstance Win32_Process | \
         Where-Object {{ {} }} | \
         Select-Object ProcessId, Name, CommandLine | \
         ConvertTo-Json -Compress",
        where_clause
    );

    let mut child = std::process::Command::new("pwsh")
        .args(["-NoProfile", "-Command", &ps_script])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(WMI_TIMEOUT_SECS);

    // Poll until done or timeout
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                break;
            }
            Ok(None) => {
                if std::time::Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }

    let mut stdout_str = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut stdout_str);
    }
    let stdout_str = stdout_str.trim();
    if stdout_str.is_empty() {
        return Some(HashMap::new());
    }

    let entries: Vec<serde_json::Value> = if stdout_str.starts_with('[') {
        serde_json::from_str(stdout_str).unwrap_or_default()
    } else {
        match serde_json::from_str::<serde_json::Value>(stdout_str) {
            Ok(val) => vec![val],
            Err(_) => Vec::new(),
        }
    };

    let mut result = HashMap::new();
    for entry in entries {
        let pid = entry.get("ProcessId").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let name = entry.get("Name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let cmd = entry.get("CommandLine").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if pid != 0 {
            result.insert(pid, ProcessEntry { name, command_line: cmd });
        }
    }
    Some(result)
}

#[cfg(not(windows))]
#[allow(dead_code)]
fn discover_wmi_with_timeout() -> Option<HashMap<u32, ProcessEntry>> {
    None
}

// ---------------------------------------------------------------------------
// Fallback: sysinfo-based discovery
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn discover_all_sysinfo() -> HashMap<u32, ProcessEntry> {
    use sysinfo::System;

    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let mut result = HashMap::new();

    for (pid, process) in sys.processes() {
        let name = process.name().to_string_lossy().to_string();
        let cmd_args: Vec<String> = process
            .cmd()
            .iter()
            .map(|s| s.to_string_lossy().to_string())
            .collect();
        let cmd_line = cmd_args.join(" ");

        result.insert(
            pid.as_u32(),
            ProcessEntry {
                name,
                command_line: cmd_line,
            },
        );
    }

    result
}
