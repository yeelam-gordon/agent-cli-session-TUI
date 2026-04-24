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
    /// Optional — defaults to all-false. Only `supports_discovery` is
    /// checked at runtime (supervisor uses it to gate scanning).
    #[serde(default)]
    pub capabilities: CapabilitiesConfig,
    pub discovery: DiscoveryConfig,
    pub session_id: SessionIdConfig,
    pub cwd: CwdConfig,
    /// Optional — defaults to JSONL format with no filters.
    #[serde(default)]
    pub events: EventsConfig,
    pub fields: FieldsConfig,
    pub state_signals: StateSignalsConfig,
    pub process_match: ProcessMatchConfig,
    #[serde(default)]
    pub tab_title: Option<TabTitleConfig>,
    #[serde(default)]
    pub session_detail: Option<SessionDetailConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CapabilitiesConfig {
    #[serde(default)]
    pub supports_resume: bool,
    /// Defaults to true — the only capability flag actually checked at runtime.
    #[serde(default = "default_true")]
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

impl Default for CapabilitiesConfig {
    fn default() -> Self {
        Self {
            supports_resume: false,
            supports_discovery: true, // only flag checked at runtime
            supports_logs: false,
            supports_wait_detection: false,
            supports_kill: false,
            supports_archive: false,
            supports_summary_extraction: false,
        }
    }
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

impl Default for EventsConfig {
    fn default() -> Self {
        Self { format: EventFormat::Jsonl, filter_out: Vec::new() }
    }
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
    /// Summary extraction spec. Optional — when absent, Session.summary is
    /// left empty (suitable for list-only TUI with no detail pane).
    #[serde(default)]
    pub summary: Option<FieldSpec>,
    /// Created-at timestamp. Optional — when absent, falls back to empty string.
    #[serde(default)]
    pub created_at: Option<TimestampSpec>,
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
    /// user interaction" behavior). Defaults to true.
    #[serde(default = "default_true")]
    pub discard_if_empty: bool,
}

fn default_true() -> bool { true }

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
        /// Optional substring that must NOT appear in the cmdline. Used to
        /// disambiguate co-tenant agents (e.g. Claude's cmdline can mention
        /// `claude` when another tool like `copilot` is also running under
        /// `node.exe`; set `not_contains: "copilot"` to skip those).
        #[serde(default)]
        not_contains: Option<String>,
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
    /// Look for `--flag <session_id>` in cmdline. Accepts either a single
    /// flag (`flag: "--session-id"`) or a list tried in priority order
    /// (`flag: ["--session-id", "--continue", "--resume"]`).
    Flag { flag: FlagSpec },
    /// Any cmdline arg must equal the session UUID literally.
    PositionalUuid,
    /// Any cmdline substring contains the session ID.
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
    /// A constant sentinel — the supervisor will substring-match any terminal
    /// tab whose title *contains* this value. Used for Claude Code, which sets
    /// its own tab title (e.g. `"✳ …"`) that we can only detect by prefix.
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

// ═══════════════════════════════════════════════════════════════════════════
// V3 YAML schema types (deserialization-only) + translation to ProviderConfigFile
// ═══════════════════════════════════════════════════════════════════════════

/// Top-level v3 provider YAML.
#[derive(Debug, Clone, Deserialize)]
pub struct ProviderConfigV3 {
    pub name: String,
    pub display_name: String,
    /// Optional `files:` section declaring source files.
    #[serde(default)]
    pub files: V3Files,
    pub discovery: V3Discovery,
    #[serde(default)]
    pub events: V3Events,
    pub extract: V3Extract,
    pub process: V3Process,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct V3Files {
    #[serde(default)]
    pub metadata: Option<String>,
    #[serde(default)]
    pub events: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct V3Discovery {
    pub base_dir: String,
    pub strategy: String,
    #[serde(default)]
    pub glob: Option<String>,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub hide_paths_glob: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct V3Events {
    #[serde(default)]
    pub filter_out: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct V3Extract {
    pub session_id: V3SessionId,
    pub cwd: V3Cwd,
    pub title: V3FieldSpec,
    pub updated_at: V3TimestampSpec,
    pub state: V3State,
    #[serde(default)]
    pub tab_title: Option<V3TabTitle>,
}

// ── session_id ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct V3SessionId {
    pub from: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub strategy: Option<String>,
}

// ── cwd ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct V3Cwd {
    pub from: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub decoder: Option<String>,
    #[serde(default)]
    pub backtrack: bool,
    #[serde(default)]
    pub from_parent: bool,
    #[serde(default)]
    pub r#where: Option<String>,
    // config_file fields
    #[serde(default)]
    pub lookup_file: Option<String>,
    #[serde(default)]
    pub key_source: Option<String>,
    #[serde(default)]
    pub container_path: Option<String>,
}

// ── field specs (title etc.) ─────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct V3FieldSpec {
    pub from: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub r#where: Option<String>,
    #[serde(default)]
    pub transforms: Vec<String>,
    #[serde(default)]
    pub fallback: Vec<V3FieldSpec>,
    #[serde(default)]
    pub strategy: Option<String>,
}

// ── timestamp ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct V3TimestampSpec {
    pub from: String,
    #[serde(default)]
    pub strategy: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
}

// ── state signals ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct V3State {
    pub from: String,
    #[serde(default)]
    pub last_event_map: BTreeMap<String, String>,
    #[serde(default)]
    pub event_predicates: Vec<V3EventPredicate>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct V3EventPredicate {
    pub r#where: String,
    pub value: String,
}

// ── tab title ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct V3TabTitle {
    pub from: String,
    #[serde(default)]
    pub strategy: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub r#where: Option<String>,
    #[serde(default)]
    pub iterate_path: Option<String>,
    #[serde(default)]
    pub tool_name_path: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub args_path: Option<String>,
    #[serde(default)]
    pub arg_field: Option<String>,
}

// ── process match ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct V3Process {
    #[serde(default)]
    pub executable: Option<String>,
    #[serde(default)]
    pub script_contains: Option<String>,
    pub r#match: V3ProcessMatch,
    #[serde(default)]
    pub fallback: Option<V3ProcessFallback>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct V3ProcessMatch {
    pub field: String,
    pub on: String,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub pid_regex: Option<String>,
    #[serde(default)]
    pub flag: Option<V3FlagValue>,
}

/// Accepts both `flag: "--x"` and `flag: ["--x", "--y"]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum V3FlagValue {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct V3ProcessFallback {
    #[serde(default)]
    pub recently_active_secs: Option<u64>,
}

// ═══════════════════════════════════════════════════════════════════════════
// Translation: V3 → ProviderConfigFile
// ═══════════════════════════════════════════════════════════════════════════

impl TryFrom<ProviderConfigV3> for ProviderConfigFile {
    type Error = anyhow::Error;

    fn try_from(v3: ProviderConfigV3) -> Result<Self, Self::Error> {
        Ok(ProviderConfigFile {
            name: v3.name,
            display_name: v3.display_name,
            capabilities: CapabilitiesConfig::default(),
            discovery: translate_discovery(&v3.discovery, &v3.files)?,
            session_id: translate_session_id(&v3.extract.session_id)?,
            cwd: translate_cwd(&v3.extract.cwd)?,
            events: EventsConfig {
                format: EventFormat::Jsonl,
                filter_out: v3.events.filter_out,
            },
            fields: translate_fields(&v3.extract)?,
            state_signals: translate_state(&v3.extract.state),
            process_match: translate_process(&v3.process)?,
            tab_title: v3.extract.tab_title.map(translate_tab_title).transpose()?,
            session_detail: None,
        })
    }
}

fn translate_discovery(d: &V3Discovery, files: &V3Files) -> Result<DiscoveryConfig, anyhow::Error> {
    let strategy = match d.strategy.as_str() {
        "dir_per_session" => DiscoveryStrategy::DirPerSession {
            metadata_file: files.metadata.clone(),
            events_file: files
                .events
                .clone()
                .unwrap_or_else(|| "events.jsonl".to_string()),
            tail_bytes: default_tail_bytes(),
            lockfile_pattern: None, // populated from process match later if needed
        },
        "file_per_session" => DiscoveryStrategy::FilePerSession {
            glob: d.glob.clone().unwrap_or_else(|| "**/*.jsonl".to_string()),
            tail_bytes: default_tail_bytes(),
            hide_paths_glob: d.hide_paths_glob.clone(),
        },
        "date_partitioned" => DiscoveryStrategy::DatePartitioned {
            pattern: d
                .pattern
                .clone()
                .unwrap_or_else(|| "{YYYY}/{MM}/{DD}/*.jsonl".to_string()),
            tail_bytes: default_tail_bytes(),
        },
        other => anyhow::bail!("unknown v3 discovery strategy: {other}"),
    };
    Ok(DiscoveryConfig {
        base_dir: d.base_dir.clone(),
        strategy,
    })
}

fn translate_session_id(sid: &V3SessionId) -> Result<SessionIdConfig, anyhow::Error> {
    match sid.from.as_str() {
        "dirname" => Ok(SessionIdConfig::Dirname),
        "filename_stem" => Ok(SessionIdConfig::FilenameStem),
        "events" => {
            if sid.strategy.as_deref() == Some("first_event_field") {
                let field = sid
                    .path
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("v3 session_id from events needs `path`"))?;
                Ok(SessionIdConfig::FirstEventField { field })
            } else {
                anyhow::bail!("v3 session_id from events: unknown strategy {:?}", sid.strategy)
            }
        }
        other => anyhow::bail!("unknown v3 session_id.from: {other}"),
    }
}

fn translate_cwd(cwd: &V3Cwd) -> Result<CwdConfig, anyhow::Error> {
    match cwd.from.as_str() {
        "metadata" => Ok(CwdConfig::YamlField {
            path: cwd.path.clone().unwrap_or_else(|| "cwd".to_string()),
        }),
        "events" => Ok(CwdConfig::EventField {
            event_type: cwd.r#where.as_ref().and_then(|w| extract_type_eq(w)),
            field: cwd
                .path
                .clone()
                .ok_or_else(|| anyhow::anyhow!("v3 cwd from events needs `path`"))?,
        }),
        "dirname" => Ok(CwdConfig::DirnameDecode {
            decoder: cwd
                .decoder
                .clone()
                .unwrap_or_else(|| "drive_dash".to_string()),
            backtrack: cwd.backtrack,
            from_parent: cwd.from_parent,
        }),
        "config_file" => Ok(CwdConfig::ConfigReverseLookup {
            lookup_file: cwd
                .lookup_file
                .clone()
                .ok_or_else(|| anyhow::anyhow!("v3 cwd config_file needs `lookup_file`"))?,
            key_source: cwd
                .key_source
                .clone()
                .unwrap_or_else(|| "parent_dir_name".to_string()),
            container_path: cwd.container_path.clone().unwrap_or_default(),
        }),
        other => anyhow::bail!("unknown v3 cwd.from: {other}"),
    }
}

/// Extract the value from `type == "X"` pattern used in where clauses.
fn extract_type_eq(expr: &str) -> Option<String> {
    let expr = expr.trim();
    // Match: type == "value"
    if let Some(rest) = expr.strip_prefix("type ==") {
        let rest = rest.trim().trim_matches('"').trim_matches('\'');
        return Some(rest.to_string());
    }
    None
}

fn translate_v3_field(f: &V3FieldSpec) -> FieldSpec {
    let strategy = match f.from.as_str() {
        "metadata" => "metadata_field".to_string(),
        "events" => "first_matching_event".to_string(),
        _ => f.strategy.clone().unwrap_or_else(|| "first_matching_event".to_string()),
    };
    FieldSpec {
        strategy,
        r#where: f.r#where.clone(),
        path: f.path.clone().unwrap_or_default(),
        transforms: f.transforms.clone(),
        join: None,
        limit: None,
        nth: None,
        fallback: f.fallback.iter().map(translate_v3_field).collect(),
    }
}

fn translate_fields(extract: &V3Extract) -> Result<FieldsConfig, anyhow::Error> {
    Ok(FieldsConfig {
        title: translate_v3_field(&extract.title),
        summary: None,
        created_at: None,
        updated_at: translate_timestamp(&extract.updated_at),
        summary_parts: Vec::new(),
        discard_if_empty: true,
    })
}

fn translate_timestamp(ts: &V3TimestampSpec) -> TimestampSpec {
    let strategy = ts
        .strategy
        .clone()
        .unwrap_or_else(|| "file_mtime".to_string());
    TimestampSpec {
        strategy,
        path: ts.path.clone(),
        fallback: Vec::new(),
    }
}

/// Translate v3 shorthand interaction values to internal model values.
fn translate_interaction(v: &str) -> String {
    match v {
        "waiting" => "waiting_input".to_string(),
        "busy" => "busy".to_string(),
        other => other.to_string(),
    }
}

fn translate_state(state: &V3State) -> StateSignalsConfig {
    let last_event_map = state
        .last_event_map
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                StateSignalDelta {
                    interaction: Some(translate_interaction(v)),
                    process: None,
                },
            )
        })
        .collect();

    let event_predicates = state
        .event_predicates
        .iter()
        .map(|p| EventPredicate {
            r#where: p.r#where.clone(),
            delta: StateSignalDelta {
                interaction: Some(translate_interaction(&p.value)),
                process: None,
            },
        })
        .collect();

    StateSignalsConfig {
        last_event_map,
        event_predicates,
        idle_threshold_seconds: default_idle_secs(),
        unfinished_turn_when: None,
        recent_tool_activity_when: None,
    }
}

fn translate_tab_title(tt: V3TabTitle) -> Result<TabTitleConfig, anyhow::Error> {
    match tt.from.as_str() {
        "events" => {
            match tt.strategy.as_deref() {
                Some("from_tool_call") => Ok(TabTitleConfig::FromToolCall {
                    tool_name: tt.tool_name.unwrap_or_default(),
                    arg_field: tt.arg_field.unwrap_or_default(),
                    r#where: tt.r#where.unwrap_or_default(),
                    tool_name_path: tt.tool_name_path.unwrap_or_default(),
                    args_path: tt.args_path.unwrap_or_default(),
                    iterate_path: tt.iterate_path,
                }),
                _ => anyhow::bail!("v3 tab_title from events: unknown strategy {:?}", tt.strategy),
            }
        }
        "literal" => Ok(TabTitleConfig::Literal {
            value: tt.value.unwrap_or_default(),
        }),
        "cwd" => {
            match tt.strategy.as_deref() {
                Some("cwd_basename") | None => Ok(TabTitleConfig::CwdBasename),
                other => anyhow::bail!("v3 tab_title from cwd: unknown strategy {other:?}"),
            }
        }
        other => anyhow::bail!("unknown v3 tab_title.from: {other}"),
    }
}

fn translate_process(p: &V3Process) -> Result<ProcessMatchConfig, anyhow::Error> {
    let recently_active_secs = p
        .fallback
        .as_ref()
        .and_then(|f| f.recently_active_secs);

    match p.r#match.on.as_str() {
        "lockfile" => Ok(ProcessMatchConfig::Lockfile {
            lockfile_pattern: p
                .r#match
                .pattern
                .clone()
                .unwrap_or_else(|| "inuse.*.lock".to_string()),
            pid_extract_regex: p
                .r#match
                .pid_regex
                .clone()
                .unwrap_or_else(|| r"inuse\.(\d+)\.lock".to_string()),
        }),
        "flag" => {
            let flag = match &p.r#match.flag {
                Some(V3FlagValue::One(s)) => FlagSpec::One(s.clone()),
                Some(V3FlagValue::Many(v)) => FlagSpec::Many(v.clone()),
                None => anyhow::bail!("v3 process match on flag: missing `flag` field"),
            };
            Ok(ProcessMatchConfig::Cmdline {
                executable: p.executable.clone().unwrap_or_default(),
                script_contains: p.script_contains.clone(),
                not_contains: None,
                id_match: CmdlineIdMatch::Flag { flag },
                recently_active_secs,
            })
        }
        "positional_arg" => Ok(ProcessMatchConfig::Cmdline {
            executable: p.executable.clone().unwrap_or_default(),
            script_contains: p.script_contains.clone(),
            not_contains: None,
            id_match: CmdlineIdMatch::PositionalUuid,
            recently_active_secs,
        }),
        "contains" => Ok(ProcessMatchConfig::Cmdline {
            executable: p.executable.clone().unwrap_or_default(),
            script_contains: p.script_contains.clone(),
            not_contains: None,
            id_match: CmdlineIdMatch::Contains,
            recently_active_secs,
        }),
        other => anyhow::bail!("unknown v3 process.match.on: {other}"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// V3 detection helper
// ═══════════════════════════════════════════════════════════════════════════

/// Returns true if the YAML text looks like a v3 provider config
/// (has top-level `extract:` key, which old format never has).
pub fn is_v3_yaml(text: &str) -> bool {
    // Quick heuristic: v3 always has `extract:` at the top level.
    // The old format uses `session_id:`, `cwd:`, `fields:`, `state_signals:`.
    text.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed == "extract:" || trimmed.starts_with("extract:")
    })
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
