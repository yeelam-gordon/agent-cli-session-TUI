//! Serde schema for provider YAML files.
//!
//! Every provider is a `ProviderConfig` parsed from `providers/<name>.yaml`.
//! The shape is intentionally unified across all 5 agent CLIs — strategy
//! discriminators (`DiscoveryStrategy`, `CwdStrategy`, `LivenessStrategy`)
//! pick the right behavior while keeping the surface identical.
//!
//! Status detection is split into orthogonal dimensions, each declarative:
//!
//!   * `state_signals`      — what the agent is doing now (busy/waiting/idle),
//!     inferred from the JSONL event stream.
//!   * `liveness_detection` — is a live OS process attached? ordered strategy
//!     chain, first match wins (lockfile / cmdline uuid /
//!     tab-title / recently-active).
//!   * resumability         — implicit today: any session whose file exists and
//!     is not archived is resumable. Will be lifted into
//!     YAML when a provider needs different behavior.

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
    pub liveness_detection: LivenessDetectionConfig,
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
        /// For Claude: try shorter fallbacks by probing disk when glob doesn't match.
        #[serde(default)]
        backtrack: bool,
        /// When `true` and the candidate path points at a file (FilePerSession),
        /// decode the *parent* directory name instead of the file name.
        #[serde(default)]
        from_parent: bool,
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
    /// Reverse-lookup in a JSON map whose KEYS are CWDs and VALUES are project
    /// names (Gemini's `~/.gemini/projects.json`). The session's ancestor
    /// directory name matches a VALUE; we scan the map for the entry whose
    /// value matches and return its KEY as the cwd.
    ConfigReverseLookup {
        /// File path with `${HOME}` expansion.
        lookup_file: String,
        /// Our key comes from the session's parent directory name.
        /// "parent_dir_name" | "parent_parent_dir_name"
        key_source: String,
        /// Dot path from the JSON root to the map object whose entries are
        /// `<cwd>: <project-name>` (e.g. `"projects"`). If empty, the root
        /// object itself is treated as the map.
        #[serde(default)]
        container_path: String,
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
    /// Optional labeled parts appended to the primary `summary` value.
    /// When present, the final Session.summary is composed by appending each
    /// resolved part below the primary summary. Lets provider YAMLs build the
    /// 4-section "First message / Last user message / Previous response /
    /// Last <Provider> response" block the legacy hand-written providers
    /// produced.
    #[serde(default)]
    pub summary_parts: Vec<SummaryPart>,
    /// If true, sessions with no extractable title AND no summary content
    /// are dropped entirely (matches legacy main's "skip sessions with zero
    /// user interaction" behavior). When false (default), empty sessions
    /// surface with a fallback "<Provider> session" title.
    #[serde(default)]
    pub discard_if_empty: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SummaryPart {
    /// Optional name so later parts can reference via `skip_if_same_as`.
    #[serde(default)]
    pub name: Option<String>,
    /// Label rendered above the extracted value (e.g. "--- First message ---").
    pub label: String,
    /// Extraction spec for this part's value.
    pub spec: FieldSpec,
    /// If this part's resolved value equals the named earlier part's value,
    /// skip rendering it. Used to avoid "last user message == first user
    /// message" duplication.
    #[serde(default)]
    pub skip_if_same_as: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FieldSpec {
    /// `first_matching_event`, `last_matching_event`,
    /// `nth_from_end_matching_event`, `metadata_field`, `joined_events`.
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
    /// For `nth_from_end_matching_event` — 1-based index from end.
    /// n=1 behaves like `last_matching_event`; n=2 is "second-to-last".
    #[serde(default)]
    pub nth: Option<usize>,
    /// Additional specs tried in order if the primary strategy returns
    /// nothing. Each fallback runs independently (same event/metadata
    /// context) and may itself specify transforms, `where`, etc.
    #[serde(default)]
    pub fallback: Vec<FieldSpec>,
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
    /// Ordered list of `(where-predicate → delta)` pairs. Used when the
    /// relevant discriminator is nested (e.g. Codex's `payload.type`) rather
    /// than the outer `type`. Evaluated after `last_event_map`: we scan events
    /// most-recent-first and, for each event, check predicates in order — the
    /// first matching (event, predicate) pair wins.
    #[serde(default)]
    pub event_predicates: Vec<EventPredicate>,
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EventPredicate {
    /// Expression evaluated against each scanned event.
    pub r#where: String,
    #[serde(flatten)]
    pub delta: StateSignalDelta,
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

// ── Liveness detection ───────────────────────────────────────────────────────

/// How to link a session to a live OS process. Strategies are tried in order;
/// FIRST MATCH wins. If none match, the session has no live process.
///
/// Typical per-provider chains:
///   * copilot → [ lockfile ]
///   * claude  → [ cmdline_flag_uuid, recently_active ]
///   * codex   → [ cmdline_positional_uuid, recently_active ]
///   * qwen    → [ cmdline_positional_uuid, tab_title_match, recently_active ]
///   * gemini  → [ recently_active ]   (no UUID on cmdline; tab title is generic)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LivenessDetectionConfig {
    /// WMI-level executable basename filter (applied to cmdline-* strategies).
    /// Omit when no cmdline strategy is used (copilot).
    #[serde(default)]
    pub executable: Option<String>,
    /// Substring that must appear in the cmdline (e.g. `qwen.js`, `gemini.js`).
    #[serde(default)]
    pub script_contains: Option<String>,
    /// Substring that must NOT appear in the cmdline. Used to disambiguate
    /// co-tenant agents (e.g. `not_contains: copilot` for Claude).
    #[serde(default)]
    pub not_contains: Option<String>,
    /// Ordered strategy chain. First matching strategy claims the session.
    pub strategies: Vec<LivenessStrategy>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub enum LivenessStrategy {
    /// Session directory contains a process-specific lock file (copilot).
    Lockfile {
        /// Glob inside the session directory (e.g. `inuse.*.lock`).
        lockfile_pattern: String,
        /// Regex with one capture group extracting the PID from filename.
        pid_extract_regex: String,
    },
    /// A matching process has the session UUID as the value of a `--flag`.
    /// Accepts a single flag or an ordered list tried in priority order.
    CmdlineFlagUuid { flag: FlagSpec },
    /// A matching process has the session UUID as a positional (non-flag) arg.
    CmdlinePositionalUuid,
    /// A matching process's cmdline contains the session UUID as a substring.
    CmdlineContains,
    /// A Windows Terminal tab is currently displaying this session's computed
    /// title (from `fields.title`). Used when the agent doesn't expose session
    /// id on cmdline AND has no lock file (qwen without `--resume`).
    ///
    /// Does NOT set a PID — only sets `process = Running`. Kill actions won't
    /// work for sessions claimed via this strategy.
    TabTitleMatch {
        #[serde(default)]
        fuzzy: FuzzyMatch,
        /// Minimum title length required to attempt a match. Titles shorter
        /// than this are skipped (to avoid generic matches like "test"
        /// colliding with unrelated tabs). Defaults to 4.
        #[serde(default = "default_min_title_len")]
        min_title_len: usize,
    },
    /// Last-resort fallback: pair any unclaimed matching process with this
    /// session if its events were updated within the window. Low specificity —
    /// use only as the final strategy in the chain.
    RecentlyActive { within_secs: u64 },
}

fn default_min_title_len() -> usize {
    4
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FuzzyMatch {
    /// Tab title must equal the session title exactly.
    Exact,
    /// Session title must be a prefix of the tab title (e.g. tab may carry
    /// a streaming indicator prefix like `✳ Foo…`). This is still a contains
    /// check on the trimmed/normalized tab — see matcher implementation.
    #[default]
    Prefix,
    /// Tab title contains the session title as any substring.
    Contains,
}

/// Either a single flag or a list of fallback flags (tried in order).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum FlagSpec {
    One(String),
    Many(Vec<String>),
}

impl FlagSpec {
    pub fn as_slice(&self) -> Vec<&str> {
        match self {
            FlagSpec::One(s) => vec![s.as_str()],
            FlagSpec::Many(v) => v.iter().map(|s| s.as_str()).collect(),
        }
    }
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
    /// Reuse the already-extracted session title (from `fields.title`) as the
    /// tab title. Used for Claude Code, which sets its own terminal tab title
    /// to `"✳ <first words of the first user message>"` — the same source
    /// `fields.title` extracts, so session.title is a substring of the real
    /// tab title and matches uniquely (unlike a shared sentinel).
    FromTitle,
    /// A constant sentinel — the supervisor will substring-match any terminal
    /// tab whose title *contains* this value. Used as a fallback when a more
    /// specific per-session string isn't available.
    Literal { value: String },
    /// Use the basename of the session's `cwd`. Used by Codex, which has no
    /// in-band tab title signal — the terminal title is set by the launcher
    /// from the working directory's folder name.
    CwdBasename,
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
    // Expand leading `~/` or `~\` (tilde) to the user's home directory.
    let expanded = if let Some(rest) = expanded
        .strip_prefix("~/")
        .or_else(|| expanded.strip_prefix("~\\"))
    {
        home.join(rest).to_string_lossy().into_owned()
    } else if expanded == "~" {
        home.to_string_lossy().into_owned()
    } else {
        expanded
    };
    PathBuf::from(expanded)
}

#[cfg(test)]
mod expand_path_tests {
    use super::*;

    #[test]
    fn expands_tilde_prefix() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(expand_path("~/foo/bar"), home.join("foo/bar"));
        assert_eq!(expand_path("~\\foo\\bar"), home.join("foo\\bar"));
        assert_eq!(expand_path("~"), home);
    }

    #[test]
    fn does_not_expand_embedded_tilde() {
        assert_eq!(expand_path("/tmp/~something"), PathBuf::from("/tmp/~something"));
    }

    #[test]
    fn expands_home_token() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(
            expand_path("${HOME}/foo"),
            PathBuf::from(format!("{}/foo", home.to_string_lossy()))
        );
    }
}
