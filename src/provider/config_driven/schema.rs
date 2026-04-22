//! Serde schema for provider YAML files.
//!
//! Every provider is a `ProviderConfig` parsed from `providers/<name>.yaml`.
//! The shape is intentionally unified across all 5 agent CLIs — strategy
//! discriminators (`DiscoveryStrategy`, `CwdStrategy`, `ProcessMatchStrategy`)
//! pick the right behavior while keeping the surface identical.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderConfigFile {
    pub name: String,
    pub display_name: String,
    pub capabilities: CapabilitiesConfig,
    pub discovery: DiscoveryConfig,
    pub session_id: SessionIdConfig,
    pub cwd: CwdConfig,
    pub events: EventsConfig,
    pub fields: FieldsConfig,
    pub state_signals: StateSignalsConfig,
    pub process_match: ProcessMatchConfig,
    #[serde(default)]
    pub tab_title: Option<TabTitleConfig>,
    #[serde(default)]
    pub session_detail: Option<SessionDetailConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct CapabilitiesConfig {
    #[serde(default)]
    pub supports_resume: bool,
    #[serde(default)]
    pub supports_discovery: bool,
    #[serde(default)]
    pub supports_logs: bool,
    #[serde(default)]
    pub supports_wait_detection: bool,
    #[serde(default)]
    pub supports_kill: bool,
    #[serde(default)]
    pub supports_archive: bool,
    #[serde(default)]
    pub supports_summary_extraction: bool,
}

// ── Discovery ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DiscoveryConfig {
    /// Base directory. Supports `${HOME}` template expansion.
    pub base_dir: String,
    #[serde(flatten)]
    pub strategy: DiscoveryStrategy,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub enum DiscoveryStrategy {
    /// One directory per session (Copilot).
    DirPerSession {
        /// Optional metadata file in each dir (e.g. `workspace.yaml`).
        #[serde(default)]
        metadata_file: Option<String>,
        /// The JSONL/YAML events file inside each dir.
        events_file: String,
        /// Bytes to tail from `events_file` when reading (for speed on large logs).
        #[serde(default = "default_tail_bytes")]
        tail_bytes: usize,
        /// Lock-file pattern in the dir (optional, for mtime + process match).
        #[serde(default)]
        lockfile_pattern: Option<String>,
    },
    /// One file per session, flat glob (Claude/Qwen/Gemini).
    FilePerSession {
        /// Recursive glob relative to `base_dir` (e.g. `**/*.jsonl`).
        glob: String,
        /// Tail bytes to scan when extracting fields/state.
        #[serde(default = "default_tail_bytes")]
        tail_bytes: usize,
        /// Optional glob patterns whose matches are HIDDEN from the listing
        /// (e.g. Claude `subagents/*.jsonl`, Gemini `chats/<UUID>/*.jsonl`).
        #[serde(default)]
        hide_paths_glob: Vec<String>,
    },
    /// YYYY/MM/DD partitioned layout (Codex).
    DatePartitioned {
        /// Relative pattern, e.g. `{YYYY}/{MM}/{DD}/*.jsonl`.
        pattern: String,
        #[serde(default = "default_tail_bytes")]
        tail_bytes: usize,
    },
}

fn default_tail_bytes() -> usize {
    524_288 // 512KB
}

// ── Session ID extraction ────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum SessionIdConfig {
    /// Directory name (DirPerSession).
    Dirname,
    /// File stem (FilePerSession).
    FilenameStem,
    /// File stem passed through a regex (optional capture group 1).
    FilenameRegex { regex: String },
    /// A field in the first event.
    FirstEventField { field: String },
}

// ── CWD extraction ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum CwdConfig {
    /// A field in the metadata file (e.g. Copilot workspace.yaml's `cwd`).
    YamlField { path: String },
    /// A field in some event in the events file.
    EventField {
        /// Optional event-type filter. Empty → first event with a non-null value at `field`.
        #[serde(default)]
        event_type: Option<String>,
        /// The field path (dot notation).
        field: String,
    },
    /// Decode the directory name using a decoder.
    DirnameDecode {
        /// `drive_dash`: Windows dash-encoded path.
        decoder: String,
        /// For Claude: try shorter fallbacks when glob doesn't match.
        #[serde(default)]
        backtrack: bool,
    },
    /// Look up in a JSON config file by key (Gemini's reverse-path map).
    ConfigLookup {
        /// File path with `${HOME}` expansion (e.g. `${HOME}/.gemini/projects.json`).
        lookup_file: String,
        /// Where the CWD lives in the JSON (dot path, relative to the value for our key).
        /// Our key comes from the session's parent directory name.
        key_source: String, // "parent_dir_name" | "parent_parent_dir_name"
        /// Path inside each entry where the full CWD lives.
        value_path: String,
    },
}

// ── Events ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EventsConfig {
    #[serde(default = "default_format")]
    pub format: EventFormat,
    /// Expressions — an event is skipped if ANY filter evaluates true.
    #[serde(default)]
    pub filter_out: Vec<String>,
}

fn default_format() -> EventFormat {
    EventFormat::Jsonl
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EventFormat {
    Jsonl,
    /// Some Gemini files are a single JSON array rather than line-delimited.
    JsonArray,
}

// ── Fields (title / summary / timestamps) ────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FieldsConfig {
    pub title: FieldSpec,
    pub summary: FieldSpec,
    pub created_at: TimestampSpec,
    pub updated_at: TimestampSpec,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FieldSpec {
    /// `first_matching_event`, `last_matching_event`, `metadata_field`, `joined_events`.
    pub strategy: String,
    /// Optional predicate expression to filter events before picking.
    #[serde(default)]
    pub r#where: Option<String>,
    /// Dot-path + `//` alt expression for the value.
    pub path: String,
    /// Optional transforms applied in order.
    #[serde(default)]
    pub transforms: Vec<String>,
    /// For `joined_events` strategy — join with this separator.
    #[serde(default)]
    pub join: Option<String>,
    /// For `joined_events` strategy — hard cap on result length.
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TimestampSpec {
    /// `metadata_field`, `event_field`, `file_mtime`, `first_event_field`, `last_event_field`.
    pub strategy: String,
    #[serde(default)]
    pub path: Option<String>,
    /// Fallback chain: additional strategies tried if the primary returns None.
    #[serde(default)]
    pub fallback: Vec<TimestampSpec>,
}

// ── State signals ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StateSignalsConfig {
    /// Map from event-type match to StateSignals deltas. First match wins when scanning
    /// from most-recent event backward.
    #[serde(default)]
    pub last_event_map: BTreeMap<String, StateSignalDelta>,
    /// Seconds of inactivity before the session is considered idle.
    #[serde(default = "default_idle_secs")]
    pub idle_threshold_seconds: u64,
    /// Optional predicate that means "has unfinished turn".
    #[serde(default)]
    pub unfinished_turn_when: Option<String>,
    /// Optional predicate that means "recent tool activity".
    #[serde(default)]
    pub recent_tool_activity_when: Option<String>,
}

fn default_idle_secs() -> u64 {
    1800
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct StateSignalDelta {
    /// `busy` | `waiting_input` | `idle` | `unknown`.
    #[serde(default)]
    pub interaction: Option<String>,
    /// `running` | `exited` | `stale_lock` | `missing`.
    #[serde(default)]
    pub process: Option<String>,
}

// ── Process match ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub enum ProcessMatchConfig {
    /// Lock-file based (Copilot).
    Lockfile {
        /// Glob inside the session directory (e.g. `inuse.*.lock`).
        lockfile_pattern: String,
        /// Regex with one capture group extracting the PID from filename.
        pid_extract_regex: String,
    },
    /// Command-line arg based (Claude, Codex, Qwen).
    Cmdline {
        /// Process executable (basename, e.g. `claude`, `codex`, `node`).
        executable: String,
        /// Optional substring that must appear in the cmdline (e.g. `qwen.js`).
        #[serde(default)]
        script_contains: Option<String>,
        /// How to match session ID against cmdline: a flag, or a positional UUID.
        #[serde(flatten)]
        id_match: CmdlineIdMatch,
        /// When no process matches by session ID, fall back to "recently-active"
        /// heuristic within this many seconds of the last event.
        #[serde(default)]
        recently_active_secs: Option<u64>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "id_match_kind", rename_all = "snake_case")]
pub enum CmdlineIdMatch {
    /// Look for `--flag <session_id>` in cmdline.
    Flag { flag: String },
    /// Any cmdline arg must equal the session UUID literally.
    PositionalUuid,
    /// Any cmdline substring contains the session ID.
    Contains,
}

// ── Tab title ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub enum TabTitleConfig {
    /// Copilot: latest `report_intent` tool call's `intent` arg.
    FromToolCall {
        tool_name: String,
        arg_field: String,
        /// jq-ish event selector (e.g. `type == "assistant.tool_call"`).
        r#where: String,
        /// Path to tool name inside the matched event (or, with `iterate_path`,
        /// inside each element of the iterated array).
        tool_name_path: String,
        /// Path to args inside the matched event (or, with `iterate_path`,
        /// inside each element).
        args_path: String,
        /// Optional path to an ARRAY inside the event. When set, we iterate the
        /// array; each element is treated as a tool call (tool_name_path and
        /// args_path resolve relative to the element).
        #[serde(default)]
        iterate_path: Option<String>,
    },
    /// Use the value of some field in the latest matching event.
    FromField {
        r#where: String,
        path: String,
    },
    /// No tab title support.
    None,
}

// ── Session detail (plan items etc.) ─────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct SessionDetailConfig {
    /// Expression that produces a list of plan items (title + done bool).
    #[serde(default)]
    pub plan_items_where: Option<String>,
    #[serde(default)]
    pub plan_item_title_path: Option<String>,
    #[serde(default)]
    pub plan_item_done_path: Option<String>,
}

// ── Template expansion ───────────────────────────────────────────────────────

/// Expand `${HOME}`, `${CACHE_DIR}`, and `${CONFIG_DIR}` in a path string.
pub fn expand_path(s: &str) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let cache = dirs::cache_dir().unwrap_or_else(|| home.join(".cache"));
    let config = dirs::config_dir().unwrap_or_else(|| home.join(".config"));
    let expanded = s
        .replace("${HOME}", &home.to_string_lossy())
        .replace("${CACHE_DIR}", &cache.to_string_lossy())
        .replace("${CONFIG_DIR}", &config.to_string_lossy());
    PathBuf::from(expanded)
}
