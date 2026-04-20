//! Shared process discovery for all providers.
//!
//! Uses WMI (Win32_Process) to reliably get process command lines on Windows.
//! sysinfo often can't read command-line args due to access restrictions,
//! but WMI uses a privileged code path that works consistently.
//!
//! Both Claude and Copilot providers should use this module instead of
//! calling sysinfo directly for process discovery.

use std::collections::HashMap;

/// A discovered OS process with its full command line.
#[derive(Debug, Clone)]
pub struct ProcessEntry {
    pub pid: u32,
    pub name: String,
    pub command_line: String,
}

/// Discover processes whose name matches the filter (case-insensitive substring).
/// Returns a map of PID → ProcessEntry for easy lookup.
///
/// On Windows, uses WMI for reliable command-line reading.
/// On other platforms, falls back to sysinfo.
pub fn discover_processes(name_filter: &str) -> HashMap<u32, ProcessEntry> {
    #[cfg(windows)]
    {
        discover_via_wmi(name_filter)
    }
    #[cfg(not(windows))]
    {
        discover_via_sysinfo(name_filter)
    }
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
// Windows: WMI-based discovery (reliable command-line reading)
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn discover_via_wmi(name_filter: &str) -> HashMap<u32, ProcessEntry> {
    let mut result = HashMap::new();

    // Use PowerShell + WMI to get process info with full command lines.
    // This is proven reliable on Windows where sysinfo can't read args.
    let ps_script = format!(
        "Get-CimInstance Win32_Process | \
         Where-Object {{ $_.Name -like '*{}*' -or $_.CommandLine -like '*{}*' }} | \
         Select-Object ProcessId, Name, CommandLine | \
         ConvertTo-Json -Compress",
        name_filter, name_filter
    );

    let output = std::process::Command::new("pwsh")
        .args(["-NoProfile", "-Command", &ps_script])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => {
            crate::log::warn("WMI process discovery failed, falling back to sysinfo");
            return discover_via_sysinfo(name_filter);
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stdout = stdout.trim();

    if stdout.is_empty() {
        return result;
    }

    // PowerShell returns a single object (not array) when there's only one match
    let entries: Vec<serde_json::Value> = if stdout.starts_with('[') {
        serde_json::from_str(stdout).unwrap_or_default()
    } else {
        match serde_json::from_str::<serde_json::Value>(stdout) {
            Ok(val) => vec![val],
            Err(_) => Vec::new(),
        }
    };

    for entry in entries {
        let pid = entry.get("ProcessId").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let name = entry
            .get("Name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let cmd = entry
            .get("CommandLine")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if pid != 0 {
            result.insert(
                pid,
                ProcessEntry {
                    pid,
                    name,
                    command_line: cmd,
                },
            );
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Fallback: sysinfo-based discovery
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn discover_via_sysinfo(name_filter: &str) -> HashMap<u32, ProcessEntry> {
    use sysinfo::System;

    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let filter_lower = name_filter.to_lowercase();
    let mut result = HashMap::new();

    for (pid, process) in sys.processes() {
        let name = process.name().to_string_lossy().to_lowercase();
        let cmd_args: Vec<String> = process
            .cmd()
            .iter()
            .map(|s| s.to_string_lossy().to_string())
            .collect();
        let cmd_line = cmd_args.join(" ");

        if name.contains(&filter_lower) || cmd_line.to_lowercase().contains(&filter_lower) {
            result.insert(
                pid.as_u32(),
                ProcessEntry {
                    pid: pid.as_u32(),
                    name: name.to_string(),
                    command_line: cmd_line,
                },
            );
        }
    }

    result
}
