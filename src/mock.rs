//! Runtime loader for the optional **mock-data plugin**.
//!
//! The plugin (a `cdylib` named `mock_data.dll` / `libmock_data.so` /
//! `libmock_data.dylib`) is **never** checked in. Its source lives in
//! the gitignored `mock-plugin/` workspace member and is built locally
//! only when capturing demo GIFs.
//!
//! At startup `main.rs` calls [`MockPlugin::try_load`] which:
//! 1. Looks for `mock_data.{dll,so,dylib}` next to the running exe.
//! 2. Resolves `mock_data_get_json` (and the ABI version probe).
//! 3. Parses the embedded JSON dataset into in-memory structs.
//!
//! If the DLL is absent or any step fails, [`MockPlugin::try_load`] returns
//! `None` and the `--mock-data` flag is a silent no-op. End-users of a
//! distributed build will never observe a working `--mock-data` mode
//! because they never receive the DLL.

use std::collections::HashMap;
use std::ffi::CStr;
use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;

use crate::acp::AiSuggestion;
use crate::models::{
    Confidence, HealthState, InteractionState, PersistenceState, ProcessState, Session,
    SessionState,
};

/// ABI version this binary understands. Must match the value reported by
/// `mock_data_abi_version` in the loaded DLL — mismatches refuse the load.
const SUPPORTED_ABI: u32 = 1;

#[derive(Deserialize)]
struct MockFile {
    #[serde(default)]
    schema_version: Option<u32>,
    sessions: Vec<MockSession>,
    #[serde(default)]
    suggestions: Vec<MockSuggestion>,
}

#[derive(Deserialize)]
struct MockSession {
    provider: String,
    title: String,
    summary: String,
    state: String,
    #[serde(default)]
    group: Option<String>,
    age_hint: String,
}

#[derive(Deserialize)]
struct MockSuggestion {
    title_prefix: String,
    group: String,
    #[serde(default)]
    is_new: bool,
    #[serde(default)]
    score: f64,
    #[serde(default)]
    reason: String,
}

/// A loaded mock-data plugin. Owns the `libloading::Library` so the JSON
/// pointer returned by the DLL remains valid for the lifetime of this
/// struct. Holds the parsed dataset.
pub struct MockPlugin {
    // Held to keep the C string returned by `mock_data_get_json` alive.
    // Never accessed after construction — the JSON is copied into `data`.
    _lib: libloading::Library,
    data: MockFile,
}

impl MockPlugin {
    /// Attempt to load the mock plugin from beside the running executable.
    /// Returns `None` when the DLL is absent, fails to load, has the wrong
    /// ABI, or returns malformed JSON. All failures are logged.
    pub fn try_load() -> Option<Self> {
        let dll_name = if cfg!(target_os = "windows") {
            "mock_data.dll"
        } else if cfg!(target_os = "macos") {
            "libmock_data.dylib"
        } else {
            "libmock_data.so"
        };

        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))?;
        let dll_path = exe_dir.join(dll_name);
        if !dll_path.exists() {
            return None;
        }

        crate::log::info(&format!("Loading mock-data plugin: {:?}", dll_path));

        let lib = match unsafe { libloading::Library::new(&dll_path) } {
            Ok(l) => l,
            Err(e) => {
                crate::log::warn(&format!("mock-data plugin load failed: {}", e));
                return None;
            }
        };

        let abi: u32 = unsafe {
            match lib.get::<libloading::Symbol<unsafe extern "C" fn() -> u32>>(
                b"mock_data_abi_version",
            ) {
                Ok(f) => f(),
                Err(_) => 0,
            }
        };
        if abi != SUPPORTED_ABI {
            crate::log::warn(&format!(
                "mock-data plugin ABI mismatch: plugin={}, expected={}",
                abi, SUPPORTED_ABI
            ));
            return None;
        }

        let json_ptr: *const std::ffi::c_char = unsafe {
            match lib.get::<libloading::Symbol<unsafe extern "C" fn() -> *const std::ffi::c_char>>(
                b"mock_data_get_json",
            ) {
                Ok(f) => f(),
                Err(e) => {
                    crate::log::warn(&format!(
                        "mock-data plugin missing mock_data_get_json: {}",
                        e
                    ));
                    return None;
                }
            }
        };
        if json_ptr.is_null() {
            crate::log::warn("mock-data plugin returned null JSON pointer");
            return None;
        }

        let json_str = match unsafe { CStr::from_ptr(json_ptr) }.to_str() {
            Ok(s) => s.to_owned(),
            Err(e) => {
                crate::log::warn(&format!("mock-data plugin JSON is not UTF-8: {}", e));
                return None;
            }
        };

        let data: MockFile = match serde_json::from_str(&json_str) {
            Ok(f) => f,
            Err(e) => {
                crate::log::warn(&format!("mock-data plugin JSON parse failed: {}", e));
                return None;
            }
        };

        if let Some(v) = data.schema_version {
            if v != 1 {
                crate::log::warn(&format!(
                    "mock-data plugin schema_version {} unsupported",
                    v
                ));
                return None;
            }
        }

        crate::log::info(&format!(
            "mock-data plugin loaded: {} sessions, {} suggestions",
            data.sessions.len(),
            data.suggestions.len()
        ));

        Some(Self { _lib: lib, data })
    }

    /// Build the curated mock session list. One mock row is rebound to a
    /// real local copilot session id so that pressing Enter on it actually
    /// resumes a terminal — necessary for the demo GIF.
    pub fn sessions(&self) -> Vec<Session> {
        let mut sessions = self.sessions_inner();

        if let Some((real_id, real_cwd)) = find_real_copilot_session() {
            if let Some(s) = sessions.iter_mut().find(|s| {
                s.provider_name == "copilot" && s.state.process == ProcessState::Exited
            }) {
                crate::log::info(&format!(
                    "Mock: wired real copilot session {} (cwd={}) into row '{}' — Enter/r will resume",
                    real_id, real_cwd, s.title
                ));
                s.provider_session_id = real_id;
                s.cwd = PathBuf::from(real_cwd);
            }
        } else {
            crate::log::warn(
                "Mock: no real copilot sessions found under ~/.copilot/session-state; \
                 resume in demo will be a no-op",
            );
        }

        sessions
    }

    fn sessions_inner(&self) -> Vec<Session> {
        let now = Utc::now();
        self.data
            .sessions
            .iter()
            .enumerate()
            .map(|(idx, m)| build_session(idx, m, now))
            .collect()
    }

    /// Pre-defined `(session_key, group_name)` pairs to seed the
    /// `GroupManager` in mock mode so the Grouped view shows memberships.
    pub fn group_assignments(&self) -> Vec<(String, String)> {
        self.data
            .sessions
            .iter()
            .enumerate()
            .filter_map(|(idx, m)| {
                m.group.as_ref().map(|g| {
                    let key = format!("{}:{}", m.provider, mock_session_id(idx));
                    (key, g.clone())
                })
            })
            .collect()
    }

    /// Pre-populated AI auto-suggestions for the demo flow. Each entry in
    /// the JSON `suggestions` array names a `title_prefix` to match against
    /// session titles; the first matching session in the dataset receives
    /// the suggestion.
    pub fn auto_suggestions(&self) -> HashMap<String, AiSuggestion> {
        let mut out = HashMap::new();
        for (idx, m) in self.data.sessions.iter().enumerate() {
            for sug in &self.data.suggestions {
                if m.title.starts_with(&sug.title_prefix) {
                    let key = format!("{}:{}", m.provider, mock_session_id(idx));
                    out.insert(
                        key.clone(),
                        AiSuggestion {
                            session: key,
                            group: sug.group.clone(),
                            is_new: sug.is_new,
                            score: sug.score,
                            reason: sug.reason.clone(),
                        },
                    );
                    break;
                }
            }
        }
        out
    }
}

// ---- helpers ---------------------------------------------------------------

/// Scan `~/.copilot/session-state/<UUID>/` for the most recently modified
/// session directory and return `(session_id, cwd)`.
fn find_real_copilot_session() -> Option<(String, String)> {
    let home = dirs::home_dir()?;
    let base = home.join(".copilot").join("session-state");
    let entries = std::fs::read_dir(&base).ok()?;

    let mut newest: Option<(std::time::SystemTime, std::path::PathBuf, String)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if name.len() != 36 || !name.contains('-') {
            continue;
        }
        let modified = entry.metadata().and_then(|m| m.modified()).ok()?;
        if newest
            .as_ref()
            .map(|(t, _, _)| modified > *t)
            .unwrap_or(true)
        {
            newest = Some((modified, path, name));
        }
    }

    let (_, path, name) = newest?;
    let cwd = parse_workspace_cwd(&path).unwrap_or_else(|| ".".to_string());
    Some((name, cwd))
}

fn parse_workspace_cwd(session_dir: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(session_dir.join("workspace.yaml")).ok()?;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("cwd:") {
            let v = rest.trim().trim_matches('"').trim_matches('\'');
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn build_session(idx: usize, m: &MockSession, now: DateTime<Utc>) -> Session {
    let updated = now - parse_age_hint(&m.age_hint);
    let created = updated - Duration::hours(2);
    let state = parse_state(&m.state);
    let pid = if matches!(state.process, ProcessState::Running) {
        Some(99_000 + idx as u32)
    } else {
        None
    };

    Session {
        id: format!("mock-{:04}", idx),
        provider_session_id: mock_session_id(idx),
        provider_name: m.provider.clone(),
        cwd: PathBuf::from("/mock/demo"),
        title: m.title.clone(),
        tab_title: None,
        summary: m.summary.clone(),
        state,
        pid,
        created_at: created.to_rfc3339(),
        updated_at: updated.to_rfc3339(),
        state_dir: None,
    }
}

fn mock_session_id(idx: usize) -> String {
    format!("{:08x}-mock-{:04}", 0xDEC0DE00u32.wrapping_add(idx as u32), idx)
}

fn parse_state(s: &str) -> SessionState {
    match s {
        "running" => SessionState {
            process: ProcessState::Running,
            interaction: InteractionState::Busy,
            persistence: PersistenceState::Resumable,
            health: HealthState::Clean,
            confidence: Confidence::High,
            reason: "mock running".into(),
        },
        "waiting" => SessionState {
            process: ProcessState::Running,
            interaction: InteractionState::WaitingInput,
            persistence: PersistenceState::Resumable,
            health: HealthState::Clean,
            confidence: Confidence::High,
            reason: "mock waiting for input".into(),
        },
        _ => SessionState {
            process: ProcessState::Exited,
            interaction: InteractionState::Idle,
            persistence: PersistenceState::Resumable,
            health: HealthState::Clean,
            confidence: Confidence::High,
            reason: "mock resumable".into(),
        },
    }
}

fn parse_age_hint(s: &str) -> Duration {
    let s = s.trim().to_lowercase();
    let s = s.strip_suffix(" ago").unwrap_or(&s);

    match s {
        "yesterday" => return Duration::hours(24),
        "last week" => return Duration::days(7),
        "weekend" => return Duration::days(2),
        _ => {}
    }

    let mut total = Duration::zero();
    let mut found = false;
    for part in s.split_whitespace() {
        let mut digits_end = 0;
        for (i, c) in part.char_indices() {
            if c.is_ascii_digit() {
                digits_end = i + c.len_utf8();
            } else {
                break;
            }
        }
        if digits_end == 0 {
            continue;
        }
        let n: i64 = match part[..digits_end].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let unit = &part[digits_end..];
        let dur = match unit {
            "m" => Duration::minutes(n),
            "h" => Duration::hours(n),
            "d" => Duration::days(n),
            "w" => Duration::weeks(n),
            _ => continue,
        };
        total += dur;
        found = true;
    }

    if found { total } else { Duration::hours(1) }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Small inline fixture — the real dataset lives in the gitignored
    // plugin crate, so unit tests use a minimal in-tree sample that
    // exercises the parsing & state-derivation code paths.
    const FIXTURE_JSON: &str = r#"{
      "schema_version": 1,
      "sessions": [
        {"provider":"copilot","title":"R1","summary":"s","state":"running","age_hint":"5m ago"},
        {"provider":"claude","title":"W1","summary":"s","state":"waiting","group":"web","age_hint":"2h ago"},
        {"provider":"codex","title":"Z1","summary":"s","state":"resumable","age_hint":"yesterday"}
      ],
      "suggestions": [
        {"title_prefix":"W1","group":"web-frontend","is_new":true,"score":0.8,"reason":"x"}
      ]
    }"#;

    fn fixture_plugin() -> MockFile {
        serde_json::from_str(FIXTURE_JSON).expect("fixture JSON parses")
    }

    #[test]
    fn parse_age_hint_basic() {
        assert_eq!(parse_age_hint("5m ago"), Duration::minutes(5));
        assert_eq!(parse_age_hint("2h ago"), Duration::hours(2));
        assert_eq!(parse_age_hint("3d ago"), Duration::days(3));
        assert_eq!(parse_age_hint("1w ago"), Duration::weeks(1));
        assert_eq!(parse_age_hint("yesterday"), Duration::hours(24));
        assert_eq!(parse_age_hint("last week"), Duration::days(7));
        assert_eq!(
            parse_age_hint("1h 15m ago"),
            Duration::hours(1) + Duration::minutes(15)
        );
    }

    #[test]
    fn build_session_maps_state() {
        let f = fixture_plugin();
        let now = Utc::now();
        let s0 = build_session(0, &f.sessions[0], now);
        assert!(matches!(s0.state.process, ProcessState::Running));
        assert!(matches!(s0.state.interaction, InteractionState::Busy));
        let s1 = build_session(1, &f.sessions[1], now);
        assert!(matches!(s1.state.interaction, InteractionState::WaitingInput));
        let s2 = build_session(2, &f.sessions[2], now);
        assert!(matches!(s2.state.process, ProcessState::Exited));
    }

    #[test]
    fn mock_session_id_is_stable() {
        let a = mock_session_id(0);
        let b = mock_session_id(0);
        assert_eq!(a, b);
        assert!(a.contains("mock-0000"));
    }

    #[test]
    fn try_load_returns_none_when_dll_absent() {
        // No plugin DLL exists next to the test binary, so this must
        // return None — proving the `--mock-data` flag is a no-op for
        // any distributed build.
        assert!(MockPlugin::try_load().is_none());
    }
}
