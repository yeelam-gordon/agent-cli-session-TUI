//! Remote JSON provider — OPT-IN.
//!
//! Instead of reading a local `state_dir`, this provider obtains its session
//! list by running a user-configured command that prints the same JSON an
//! `agent-session-tui --dump-json` run would produce (a JSON array of
//! `Session` objects), and deserializing it.
//!
//! This is what lets the TUI run on a HOST machine and list Copilot sessions
//! that actually live on a TARGET machine, reachable over a tunnel — e.g.:
//!
//! ```toml
//! [providers.copilot]
//! remote_list_cmd = ["devbox", "exec", "gordon-devbox1",
//!                    "agent-session-tui --dump-json 50"]
//! ```
//!
//! Only activated when `remote_list_cmd` is set for the provider. With it
//! unset (the default), the framework uses the normal local YAML provider and
//! behaviour is unchanged.
//! This module provides two OPT-IN remote providers:
//!
//! * [`RemoteJsonProvider`] — one-shot. Runs a command each refresh and parses
//!   the JSON array it prints (`agent-session-tui --dump-json`).
//! * [`RemoteStreamProvider`] — streaming (preferred). Spawns a command ONCE
//!   that stays connected and emits one compact NDJSON line per snapshot
//!   (`agent-session-tui --serve-json`); caches the latest, reconnects if the
//!   stream drops. Lower overhead and near-real-time over a single connection.
//!
//! ```toml
//! [providers.copilot]
//! # one-shot (fallback):
//! remote_list_cmd   = ["devbox", "exec", "gordon-devbox1", "agent-session-tui --dump-json 50"]
//! # streaming (preferred):
//! remote_stream_cmd = ["devbox", "exec", "gordon-devbox1", "agent-session-tui --serve-json 50 --interval 5"]
//! ```
//!
//! With neither set (the default), the framework uses the normal local YAML
//! provider and behaviour is unchanged.
//!
//! Liveness/process-matching is intentionally a no-op for both: the host cannot
//! see the target's PIDs, so sessions carry whatever state the target computed
//! (accurate, since the target runs its own discovery).

use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

use crate::models::{ProviderCapabilities, Session};
use crate::provider::Provider;

pub struct RemoteJsonProvider {
    key: String,
    name: String,
    list_cmd: Vec<String>,
    preserve_provider_name: bool,
}

impl RemoteJsonProvider {
    /// Per-provider remote: forces every session's `provider_name` to `key`.
    pub fn new(key: &str, list_cmd: Vec<String>) -> Self {
        Self {
            key: key.to_string(),
            name: format!("{key} (remote)"),
            list_cmd,
            preserve_provider_name: false,
        }
    }

    /// Whole-box remote (`--remote <box>`): the box's `--dump-json` already
    /// carries each session's real `provider_name` (copilot/claude/…), so keep
    /// it — that's what lets resume build the correct per-agent command.
    pub fn new_whole_box(key: &str, list_cmd: Vec<String>) -> Self {
        Self {
            key: key.to_string(),
            name: format!("{key} (remote)"),
            list_cmd,
            preserve_provider_name: true,
        }
    }
}

impl Provider for RemoteJsonProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn key(&self) -> &str {
        &self.key
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_resume: true,
            supports_discovery: true,
            supports_logs: false,
            supports_wait_detection: false,
            supports_kill: false,
            supports_archive: true,
            supports_summary_extraction: true,
        }
    }

    fn discover_sessions(&self) -> Result<Vec<Session>> {
        // Full discovery for remote providers is intentionally bounded into
        // pages. We keep fetching pages until one comes back short.
        const PAGE: usize = 50;
        let mut all = Vec::new();
        let mut offset = 0usize;
        loop {
            let page = self.discover_sessions_paged(offset, PAGE)?;
            if page.sessions.is_empty() {
                break;
            }
            let got = page.sessions.len();
            all.extend(page.sessions);
            if got < PAGE {
                break;
            }
            offset = offset.saturating_add(PAGE);
        }
        Ok(all)
    }

    fn discover_sessions_paged(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<crate::provider::PagedSessions> {
        let cmd = rewrite_dump_json_command(&self.list_cmd, offset, limit);
        let output = std::process::Command::new(&cmd[0])
            .args(&cmd[1..])
            .output()
            .with_context(|| format!("running remote list command {:?}", cmd))?;

        if !output.status.success() {
            anyhow::bail!(
                "remote list command exited {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json = extract_json_array(&stdout)
            .ok_or_else(|| anyhow::anyhow!("no JSON array found in remote output"))?;

        let mut sessions: Vec<Session> = serde_json::from_str(&json)
            .with_context(|| "parsing remote --dump-json output into sessions")?;

        if !self.preserve_provider_name {
            for s in &mut sessions {
                s.provider_name = self.key.clone();
            }
        }

        let got = sessions.len();
        Ok(crate::provider::PagedSessions {
            sessions,
            total: offset.saturating_add(got),
            has_more: got == limit,
        })
    }

    fn match_processes(&self, _sessions: &mut [Session]) -> Result<()> {
        // Remote: we cannot see the target's OS processes. Leave state as-is
        // (Resumable), which is the correct default for cross-machine resume.
        Ok(())
    }
}

/// Rewrites a remote list command that contains `am --dump-json` (or direct
/// `agent-session-tui --dump-json`) to ask for a specific offset/limit page.
///
/// The current Gordon setup wraps the real remote command inside the *last*
/// argument passed to the host-side PowerShell `-Command`, e.g.:
///   & '...\\devbox.ps1' exec g0 'am --dump-json 50'
/// We keep the wrapper intact and replace only the inner dump-json invocation.
fn rewrite_dump_json_command(list_cmd: &[String], offset: usize, limit: usize) -> Vec<String> {
    let mut out = list_cmd.to_vec();
    if let Some(last) = out.last_mut() {
        if let Some(rewritten) = rewrite_dump_json_fragment(last, offset, limit) {
            *last = rewritten;
        }
    }
    out
}

fn rewrite_dump_json_fragment(src: &str, offset: usize, limit: usize) -> Option<String> {
    for marker in ["am --dump-json", "agent-session-tui --dump-json"] {
        if let Some(start) = src.find(marker) {
            let after = &src[start..];
            let end_rel = after.find('\'').unwrap_or(after.len());
            let before = &src[..start];
            let after_tail = &after[end_rel..];
            let replacement = format!("{marker} --limit {limit} --offset {offset}");
            return Some(format!("{before}{replacement}{after_tail}"));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// RemoteStreamProvider — persistent streaming (preferred remote mode)
// ---------------------------------------------------------------------------

/// Keeps a single long-running connection to a target-side
/// `agent-session-tui --serve-json` and caches the latest snapshot it streams.
///
/// A background thread spawns the command, reads it line-by-line (each line is
/// one compact JSON snapshot), and updates a shared cache. `discover_sessions`
/// just returns the cached snapshot — non-blocking. If the stream ends (tunnel
/// drop, target restart), the thread reconnects after a short backoff.
pub struct RemoteStreamProvider {
    key: String,
    name: String,
    latest: Arc<Mutex<Vec<Session>>>,
    child: Arc<Mutex<Option<std::process::Child>>>,
    stop: Arc<AtomicBool>,
}

impl RemoteStreamProvider {
    /// Per-provider remote: forces every streamed session's `provider_name` to
    /// `key` (used when `remote_stream_cmd` is set on one provider).
    pub fn new(key: &str, stream_cmd: Vec<String>) -> Self {
        Self::build(key, stream_cmd, false)
    }

    /// Whole-box remote (`--remote <box>`): the stream already carries each
    /// session's real `provider_name` (copilot/claude/…), so preserve it —
    /// that's what lets resume pick the correct per-agent command on the box.
    pub fn new_whole_box(key: &str, stream_cmd: Vec<String>) -> Self {
        Self::build(key, stream_cmd, true)
    }

    fn build(key: &str, stream_cmd: Vec<String>, preserve_provider_name: bool) -> Self {
        let latest: Arc<Mutex<Vec<Session>>> = Arc::new(Mutex::new(Vec::new()));
        let child: Arc<Mutex<Option<std::process::Child>>> = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));

        if !stream_cmd.is_empty() {
            let key_owned = key.to_string();
            let latest_c = latest.clone();
            let child_c = child.clone();
            let stop_c = stop.clone();
            std::thread::Builder::new()
                .name(format!("remote-stream-{key}"))
                .spawn(move || {
                    stream_loop(
                        key_owned,
                        stream_cmd,
                        latest_c,
                        child_c,
                        stop_c,
                        preserve_provider_name,
                    )
                })
                .ok();
        }

        Self {
            key: key.to_string(),
            name: format!("{key} (remote-stream)"),
            latest,
            child,
            stop,
        }
    }
}

impl Drop for RemoteStreamProvider {
    fn drop(&mut self) {
        // Best-effort: stop the reader loop and kill the current child so we
        // don't orphan the tunnel connection when the TUI exits.
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(mut slot) = self.child.lock() {
            if let Some(mut c) = slot.take() {
                let _ = c.kill();
            }
        }
    }
}

impl Provider for RemoteStreamProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn key(&self) -> &str {
        &self.key
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_resume: true,
            supports_discovery: true,
            supports_logs: false,
            supports_wait_detection: false,
            supports_kill: false,
            supports_archive: true,
            supports_summary_extraction: true,
        }
    }

    fn discover_sessions(&self) -> Result<Vec<Session>> {
        Ok(self.latest.lock().map(|g| g.clone()).unwrap_or_default())
    }

    fn match_processes(&self, _sessions: &mut [Session]) -> Result<()> {
        Ok(())
    }
}

/// Reconnecting reader loop for [`RemoteStreamProvider`]. Runs on a background
/// thread until `stop` is set.
fn stream_loop(
    key: String,
    cmd: Vec<String>,
    latest: Arc<Mutex<Vec<Session>>>,
    child_slot: Arc<Mutex<Option<std::process::Child>>>,
    stop: Arc<AtomicBool>,
    preserve_provider_name: bool,
) {
    use std::process::{Command, Stdio};

    while !stop.load(Ordering::Relaxed) {
        let mut child = match Command::new(&cmd[0])
            .args(&cmd[1..])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                crate::log::warn(&format!("remote-stream {key}: spawn failed: {e}"));
                std::thread::sleep(std::time::Duration::from_secs(3));
                continue;
            }
        };

        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => {
                let _ = child.kill();
                std::thread::sleep(std::time::Duration::from_secs(3));
                continue;
            }
        };
        // Hand the child to the shared slot so Drop can kill it on shutdown.
        if let Ok(mut slot) = child_slot.lock() {
            *slot = Some(child);
        }

        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let line = match line {
                Ok(l) => l,
                Err(_) => break, // stream error → reconnect
            };
            if let Some(json) = extract_json_array(&line) {
                match serde_json::from_str::<Vec<Session>>(&json) {
                    Ok(mut sessions) => {
                        if !preserve_provider_name {
                            for s in &mut sessions {
                                s.provider_name = key.clone();
                            }
                        }
                        if let Ok(mut g) = latest.lock() {
                            *g = sessions;
                        }
                    }
                    Err(e) => {
                        crate::log::warn(&format!("remote-stream {key}: bad snapshot: {e}"));
                    }
                }
            }
        }

        // Stream ended — reap the child and back off before reconnecting.
        if let Ok(mut slot) = child_slot.lock() {
            if let Some(mut c) = slot.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
        if stop.load(Ordering::Relaxed) {
            break;
        }
        crate::log::info(&format!(
            "remote-stream {key}: disconnected, retrying in 2s"
        ));
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

/// Extract the first top-level JSON array from a string that may be wrapped in
/// unrelated launcher noise (e.g. a `[devbox] connecting...` status line, which
/// itself contains brackets). Returns the balanced `[ ... ]` substring, or None.
///
/// Strategy: find the first `[` that plausibly starts a JSON array (next
/// non-whitespace char looks like JSON), then scan forward with string- and
/// depth-awareness to the matching close bracket.
fn extract_json_array(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' && looks_like_json_array_start(&bytes[i..]) {
            if let Some(end) = scan_balanced_array(&bytes[i..]) {
                return Some(s[i..i + end].to_string());
            }
        }
        i += 1;
    }
    None
}

/// True if, after the opening `[`, the next non-whitespace byte is one that can
/// legally begin a JSON value (or an immediate `]` for an empty array). This
/// rejects noise like `[devbox]` where `[` is followed by a letter.
fn looks_like_json_array_start(bytes: &[u8]) -> bool {
    let mut j = 1;
    while j < bytes.len() && (bytes[j] as char).is_whitespace() {
        j += 1;
    }
    if j >= bytes.len() {
        return false;
    }
    matches!(
        bytes[j],
        b'{' | b'"' | b']' | b'-' | b'0'..=b'9' | b't' | b'f' | b'n'
    )
}

/// Given a slice starting at `[`, return the index just past the matching `]`
/// (i.e. exclusive end), scanning with brace depth and JSON string awareness so
/// brackets inside strings don't throw off the balance. None if unbalanced.
fn scan_balanced_array(bytes: &[u8]) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (idx, &b) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'[' | b'{' => depth += 1,
            b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx + 1);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_array_amid_noise() {
        // Leading `[devbox]` status line contains brackets but must be skipped.
        let raw = "[devbox] connecting...\n[ {\"x\":1} ]\nbye\n";
        let got = extract_json_array(raw).unwrap();
        assert_eq!(got, "[ {\"x\":1} ]");
    }

    #[test]
    fn handles_brackets_inside_strings() {
        // A `]` inside a JSON string value must not end the array early.
        let raw = "noise [{\"title\":\"a] b [c\"}] tail";
        let got = extract_json_array(raw).unwrap();
        assert_eq!(got, "[{\"title\":\"a] b [c\"}]");
    }

    #[test]
    fn handles_nested_arrays() {
        let raw = "[{\"tags\":[1,2,3]}]";
        assert_eq!(extract_json_array(raw).unwrap(), raw);
    }

    #[test]
    fn empty_array() {
        assert_eq!(extract_json_array("[devbox] x\n[]\n").unwrap(), "[]");
    }

    #[test]
    fn none_when_no_array() {
        assert!(extract_json_array("no json here").is_none());
    }

    #[test]
    fn stream_provider_empty_cmd_is_noop() {
        // Empty command → no reader thread, discover returns empty immediately.
        let p = RemoteStreamProvider::new("copilot", vec![]);
        assert_eq!(p.key(), "copilot");
        assert!(p.discover_sessions().unwrap().is_empty());
        assert!(p.capabilities().supports_resume);
    }

    #[test]
    fn rewrite_dump_json_fragment_updates_wrapped_am_command() {
        let src = "& 'C:\\x\\devbox.ps1' exec g0 'am --dump-json 50'";
        let got = rewrite_dump_json_fragment(src, 20, 10).unwrap();
        assert_eq!(
            got,
            "& 'C:\\x\\devbox.ps1' exec g0 'am --dump-json --limit 10 --offset 20'"
        );
    }

    #[test]
    fn rewrite_dump_json_fragment_updates_direct_agent_session_tui() {
        let src = "agent-session-tui --dump-json 50";
        let got = rewrite_dump_json_fragment(src, 0, 20).unwrap();
        assert_eq!(got, "agent-session-tui --dump-json --limit 20 --offset 0");
    }
}
