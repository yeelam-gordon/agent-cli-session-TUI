//! `ConfigDrivenProvider` — the single `Provider` implementation backed by a
//! YAML file. One YAML per agent CLI.
//!
//! Strategy dispatch is inline in this module (small, local enums; no dyn-Trait
//! zoo). Expression evaluation goes through `eval::Expr`.
//!
//! File layout:
//!   - `schema`  — serde types
//!   - `eval`    — expression evaluator (dot-paths + // alt + equality + bool)
//!   - `mod`     — the Provider trait impl (this file)

pub mod eval;
pub mod schema;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::config::ProviderConfig as AppProviderConfig;
use crate::models::{
    ActivitySource, Confidence, HealthState, InteractionState, PersistenceState,
    ProcessState, ProviderCapabilities, Session, SessionState, StateSignals,
};
use crate::process_info::{discover_processes, extract_flag_value};
use crate::provider::{Provider, SessionDetail};
use crate::util::truncate_str_safe;

use eval::{Expr, ExprCache};
use schema::*;

// ─────────────────────────────────────────────────────────────────────────────
// ConfigDrivenProvider
// ─────────────────────────────────────────────────────────────────────────────

pub struct ConfigDrivenProvider {
    cfg: ProviderConfigFile,
    app_cfg: AppProviderConfig,
    base_dir: PathBuf,
    cache: Mutex<ExprCache>,
}

impl ConfigDrivenProvider {
    /// Load a provider from a YAML file on disk.
    pub fn load_from_yaml(path: &Path, app_cfg: &AppProviderConfig) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading provider YAML: {path:?}"))?;
        let cfg: ProviderConfigFile = serde_yaml::from_str(&text)
            .with_context(|| format!("parsing provider YAML: {path:?}"))?;
        Self::from_config(cfg, app_cfg)
    }

    pub fn from_config(cfg: ProviderConfigFile, app_cfg: &AppProviderConfig) -> Result<Self> {
        let base_dir = app_cfg
            .state_dir
            .clone()
            .unwrap_or_else(|| expand_path(&cfg.discovery.base_dir));
        Ok(Self {
            cfg,
            app_cfg: app_cfg.clone(),
            base_dir,
            cache: Mutex::new(ExprCache::new()),
        })
    }

    fn expr(&self, src: &str) -> Expr {
        let mut guard = self.cache.lock().unwrap();
        match guard.get(src) {
            Ok(e) => e.clone(),
            Err(_) => {
                // Failure shouldn't happen at runtime if the YAML is valid —
                // cache a never-true expression to avoid re-parsing.
                Expr::parse("null").unwrap()
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Provider trait impl
// ─────────────────────────────────────────────────────────────────────────────

impl Provider for ConfigDrivenProvider {
    fn name(&self) -> &str {
        &self.cfg.display_name
    }
    fn key(&self) -> &str {
        &self.cfg.name
    }
    fn capabilities(&self) -> ProviderCapabilities {
        let c = &self.cfg.capabilities;
        ProviderCapabilities {
            supports_resume: c.supports_resume,
            supports_discovery: c.supports_discovery,
            supports_logs: c.supports_logs,
            supports_wait_detection: c.supports_wait_detection,
            supports_kill: c.supports_kill,
            supports_archive: c.supports_archive,
            supports_summary_extraction: c.supports_summary_extraction,
        }
    }

    fn discover_sessions(&self) -> Result<Vec<Session>> {
        let candidates = list_candidates(&self.cfg.discovery, &self.base_dir)?;
        let mut out = Vec::with_capacity(candidates.len());
        for cand in candidates {
            match parse_session(self, &cand) {
                Ok(Some(s)) => out.push(s),
                Ok(None) => {} // filtered
                Err(_e) => { /* ignore one bad session */ }
            }
        }
        // most recent first
        out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(out)
    }

    fn match_processes(&self, sessions: &mut [Session]) -> Result<()> {
        match_processes_dispatch(self, sessions)
    }

    fn activity_sources(&self, session: &Session) -> Result<Vec<ActivitySource>> {
        if let Some(dir) = &session.state_dir {
            match &self.cfg.discovery.strategy {
                DiscoveryStrategy::DirPerSession { events_file, .. } => {
                    Ok(vec![ActivitySource::EventStream(dir.join(events_file))])
                }
                _ => Ok(vec![ActivitySource::EventStream(dir.clone())]),
            }
        } else {
            Ok(vec![])
        }
    }

    fn session_detail(&self, session: &Session) -> Result<SessionDetail> {
        Ok(SessionDetail {
            title: Some(session.title.clone()),
            summary: Some(session.summary.clone()),
            plan_items: vec![],
        })
    }

    fn tab_title(&self, session: &Session) -> Option<String> {
        let tab = self.cfg.tab_title.as_ref()?;
        if let TabTitleConfig::CwdBasename = tab {
            return session
                .cwd
                .file_name()
                .map(|n| n.to_string_lossy().to_string());
        }
        let events = read_session_events(self, session.state_dir.as_deref()?).ok()?;
        extract_tab_title(self, tab, &events)
    }

    fn infer_state(&self, signals: &StateSignals) -> SessionState {
        // Start with the default inference
        let mut st = crate::provider::default_state_inference(signals);
        // If the YAML last_event_map explicitly set `forced_interaction` via
        // build_state_signals → apply_state_delta, honor it (overrides Unknown/etc).
        if let Some(forced) = signals.forced_interaction.as_deref() {
            st.interaction = match forced {
                "busy" => InteractionState::Busy,
                "waiting_input" => InteractionState::WaitingInput,
                "idle" => InteractionState::Idle,
                _ => st.interaction,
            };
            st.confidence = Confidence::High;
        }
        st
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Session candidate — internal
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Candidate {
    session_id: String,
    /// For DirPerSession: the directory. For FilePerSession: the file.
    path: PathBuf,
    /// Pre-read metadata (YAML for Copilot).
    metadata: Option<Value>,
    /// Raw (unfiltered) events list.
    events: Vec<Value>,
    /// File mtime of the events source (best-effort for `file_mtime` strategy).
    file_mtime: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Discovery dispatch
// ─────────────────────────────────────────────────────────────────────────────

fn list_candidates(
    disc: &DiscoveryConfig,
    base_dir: &Path,
) -> Result<Vec<Candidate>> {
    match &disc.strategy {
        DiscoveryStrategy::DirPerSession {
            metadata_file,
            events_file,
            tail_bytes,
            lockfile_pattern: _,
        } => list_dir_per_session(base_dir, metadata_file.as_deref(), events_file, *tail_bytes),
        DiscoveryStrategy::FilePerSession {
            glob,
            tail_bytes,
            hide_paths_glob,
        } => list_file_per_session(base_dir, glob, *tail_bytes, hide_paths_glob),
        DiscoveryStrategy::DatePartitioned { pattern, tail_bytes } => {
            list_date_partitioned(base_dir, pattern, *tail_bytes)
        }
    }
}

fn list_dir_per_session(
    base: &Path,
    metadata_file: Option<&str>,
    events_file: &str,
    tail_bytes: usize,
) -> Result<Vec<Candidate>> {
    if !base.exists() {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(base)?.flatten() {
        if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            continue;
        }
        let dir = entry.path();
        let session_id = dir.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
        if session_id.is_empty() { continue; }

        // metadata
        let metadata = metadata_file.and_then(|name| {
            let p = dir.join(name);
            let text = std::fs::read_to_string(&p).ok()?;
            serde_yaml::from_str::<Value>(&text).ok()
        });

        // events — tail read
        let events_path = dir.join(events_file);
        let events = read_jsonl_tail(&events_path, tail_bytes).unwrap_or_default();
        let file_mtime = file_mtime_rfc3339(&events_path);

        out.push(Candidate {
            session_id,
            path: dir,
            metadata,
            events,
            file_mtime,
        });
    }
    Ok(out)
}

fn list_file_per_session(
    base: &Path,
    glob_pat: &str,
    tail_bytes: usize,
    hide_globs: &[String],
) -> Result<Vec<Candidate>> {
    if !base.exists() {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    collect_files_recursive(base, base, glob_pat, hide_globs, &mut out, tail_bytes);
    Ok(out)
}

fn collect_files_recursive(
    root: &Path,
    dir: &Path,
    glob_pat: &str,
    hide_globs: &[String],
    out: &mut Vec<Candidate>,
    tail_bytes: usize,
) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let p = entry.path();
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if ft.is_dir() {
            collect_files_recursive(root, &p, glob_pat, hide_globs, out, tail_bytes);
            continue;
        }
        if !matches_simple_glob(&p, root, glob_pat) {
            continue;
        }
        if hide_globs.iter().any(|pat| matches_simple_glob(&p, root, pat)) {
            continue;
        }
        let session_id = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if session_id.is_empty() { continue; }
        let events = read_jsonl_tail(&p, tail_bytes).unwrap_or_default();
        let file_mtime = file_mtime_rfc3339(&p);
        out.push(Candidate {
            session_id,
            path: p,
            metadata: None,
            events,
            file_mtime,
        });
    }
}

fn list_date_partitioned(
    base: &Path,
    _pattern: &str,
    tail_bytes: usize,
) -> Result<Vec<Candidate>> {
    // For Codex: YYYY/MM/DD/*.jsonl. We just walk 3 levels deep.
    if !base.exists() {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    for yr in std::fs::read_dir(base)?.flatten() {
        if !yr.file_type().map(|f| f.is_dir()).unwrap_or(false) { continue; }
        for mo in std::fs::read_dir(yr.path()).ok().into_iter().flatten().flatten() {
            if !mo.file_type().map(|f| f.is_dir()).unwrap_or(false) { continue; }
            for day in std::fs::read_dir(mo.path()).ok().into_iter().flatten().flatten() {
                if !day.file_type().map(|f| f.is_dir()).unwrap_or(false) { continue; }
                for file in std::fs::read_dir(day.path()).ok().into_iter().flatten().flatten() {
                    let p = file.path();
                    if p.extension().and_then(|e| e.to_str()) != Some("jsonl") { continue; }
                    let session_id = p
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .map(|s| {
                            // Codex files are `rollout-<iso>-<uuid>.jsonl` — take UUID
                            s.rsplit('-').take(5).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("-")
                        })
                        .unwrap_or_default();
                    if session_id.is_empty() { continue; }
                    let events = read_jsonl_tail(&p, tail_bytes).unwrap_or_default();
                    let file_mtime = file_mtime_rfc3339(&p);
                    out.push(Candidate {
                        session_id,
                        path: p,
                        metadata: None,
                        events,
                        file_mtime,
                    });
                }
            }
        }
    }
    Ok(out)
}

/// Very small glob: supports `**`, `*`, and literals. Paths are normalized to `/`.
fn matches_simple_glob(path: &Path, base: &Path, pat: &str) -> bool {
    let rel = path.strip_prefix(base).unwrap_or(path);
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    // Translate glob to simple regex-ish matcher
    let mut re = String::from("^");
    let mut chars = pat.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    // optional trailing slash
                    if chars.peek() == Some(&'/') { chars.next(); }
                    re.push_str(".*");
                } else {
                    re.push_str("[^/]*");
                }
            }
            '?' => re.push('.'),
            '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' | '\\' => {
                re.push('\\');
                re.push(c);
            }
            c => re.push(c),
        }
    }
    re.push('$');
    regex_match(&re, &rel_str)
}

// ─────────────────────────────────────────────────────────────────────────────
// Event parsing
// ─────────────────────────────────────────────────────────────────────────────

fn read_jsonl_tail(path: &Path, tail_bytes: usize) -> Result<Vec<Value>> {
    let metadata = std::fs::metadata(path)?;
    let len = metadata.len() as usize;
    let start = len.saturating_sub(tail_bytes);
    let text = if start == 0 {
        std::fs::read_to_string(path)?
    } else {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = std::fs::File::open(path)?;
        f.seek(SeekFrom::Start(start as u64))?;
        let mut s = String::new();
        f.read_to_string(&mut s)?;
        // drop partial first line
        if let Some(pos) = s.find('\n') { s.drain(..=pos); }
        s
    };
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            out.push(v);
        }
    }
    Ok(out)
}

fn file_mtime_rfc3339(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let t = meta.modified().ok()?;
    let dt: chrono::DateTime<chrono::Local> = t.into();
    Some(dt.to_rfc3339())
}

// ─────────────────────────────────────────────────────────────────────────────
// Session assembly
// ─────────────────────────────────────────────────────────────────────────────

fn parse_session(prov: &ConfigDrivenProvider, cand: &Candidate) -> Result<Option<Session>> {
    let cfg = &prov.cfg;

    // Apply filter_out
    let filters: Vec<Expr> = cfg.events.filter_out.iter().map(|s| prov.expr(s)).collect();
    let kept: Vec<&Value> = cand
        .events
        .iter()
        .filter(|ev| !filters.iter().any(|f| f.eval_bool(ev)))
        .collect();

    if kept.is_empty() && cand.metadata.is_none() {
        return Ok(None);
    }

    // session id
    let session_id = match &cfg.session_id {
        SessionIdConfig::Dirname | SessionIdConfig::FilenameStem => cand.session_id.clone(),
        SessionIdConfig::FilenameRegex { regex } => {
            regex_capture1(regex, &cand.session_id).unwrap_or_else(|| cand.session_id.clone())
        }
        SessionIdConfig::FirstEventField { field } => kept
            .first()
            .and_then(|ev| prov.expr(field).eval_str(ev))
            .unwrap_or_else(|| cand.session_id.clone()),
    };

    // cwd
    let cwd = resolve_cwd(prov, cand, &kept)?.unwrap_or_else(|| PathBuf::from("."));

    // title
    let title = extract_field(prov, &cand.metadata, &kept, &cfg.fields.title)
        .unwrap_or_else(|| format!("{} session", cfg.display_name));
    let title = truncate_str_safe(&title, 120);

    // summary
    let summary = extract_field(prov, &cand.metadata, &kept, &cfg.fields.summary)
        .unwrap_or_default();

    // timestamps
    let created_at = extract_timestamp(prov, &cand.metadata, &kept, &cfg.fields.created_at, cand.file_mtime.as_deref())
        .unwrap_or_else(|| cand.file_mtime.clone().unwrap_or_default());
    let updated_at = extract_timestamp(prov, &cand.metadata, &kept, &cfg.fields.updated_at, cand.file_mtime.as_deref())
        .unwrap_or_else(|| cand.file_mtime.clone().unwrap_or_default());

    // state — default Resumable unless a process matches later
    let state = SessionState {
        process: ProcessState::Missing,
        interaction: InteractionState::Unknown,
        persistence: PersistenceState::Resumable,
        health: HealthState::Clean,
        confidence: Confidence::Low,
        reason: "initial discovery".into(),
    };

    Ok(Some(Session {
        id: uuid_like(&session_id),
        provider_session_id: session_id,
        provider_name: cfg.name.clone(),
        cwd,
        title,
        tab_title: None,
        summary,
        state,
        pid: None,
        created_at,
        updated_at,
        state_dir: Some(cand.path.clone()),
    }))
}

/// Simple hash-based UUID-ish ID (stable for a given session id).
fn uuid_like(seed: &str) -> String {
    // just prepend "internal-" — we don't actually need uniqueness across providers here
    // because Session.id is used as a map key scoped with provider_name.
    format!("cd-{seed}")
}

// ─────────────────────────────────────────────────────────────────────────────
// CWD resolution
// ─────────────────────────────────────────────────────────────────────────────

fn resolve_cwd(
    prov: &ConfigDrivenProvider,
    cand: &Candidate,
    events: &[&Value],
) -> Result<Option<PathBuf>> {
    match &prov.cfg.cwd {
        CwdConfig::YamlField { path } => {
            let Some(meta) = &cand.metadata else { return Ok(None) };
            let e = prov.expr(path);
            Ok(e.eval_str(meta).map(PathBuf::from))
        }
        CwdConfig::EventField { event_type, field } => {
            let e = prov.expr(field);
            let type_filter = event_type.as_ref().map(|t| prov.expr(&format!("type == \"{t}\"")));
            for ev in events {
                let matches = match &type_filter {
                    Some(tf) => tf.eval_bool(ev),
                    None => true,
                };
                if matches {
                    if let Some(s) = e.eval_str(ev) {
                        if !s.is_empty() {
                            return Ok(Some(PathBuf::from(s)));
                        }
                    }
                }
            }
            Ok(None)
        }
        CwdConfig::DirnameDecode { decoder, backtrack, from_parent } => {
            // For FilePerSession, the encoded CWD lives in the PARENT directory name.
            // For DirPerSession, the session dir itself carries the encoded CWD.
            let name_src: Option<&Path> = if *from_parent {
                cand.path.parent()
            } else {
                Some(cand.path.as_path())
            };
            let name = name_src
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .unwrap_or("");
            Ok(Some(decode_cwd_name(name, decoder, *backtrack)))
        }
        CwdConfig::ConfigLookup { lookup_file, key_source, value_path } => {
            let p = expand_path(lookup_file);
            let text = std::fs::read_to_string(&p).unwrap_or_default();
            let json: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            let key = resolve_key_source(key_source, &cand.path);
            if let Some(entry) = json.get(&key) {
                let expr = prov.expr(value_path);
                return Ok(expr.eval_str(entry).map(PathBuf::from));
            }
            Ok(None)
        }
        CwdConfig::ConfigReverseLookup { lookup_file, key_source, container_path } => {
            let p = expand_path(lookup_file);
            let text = std::fs::read_to_string(&p).unwrap_or_default();
            let json: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            let key = resolve_key_source(key_source, &cand.path);
            // Navigate to the container object that holds the <cwd>: <name> map.
            let mut container = &json;
            if !container_path.is_empty() {
                for seg in container_path.split('.') {
                    container = match container.get(seg) {
                        Some(c) => c,
                        None => return Ok(None),
                    };
                }
            }
            let map = match container.as_object() {
                Some(m) => m,
                None => return Ok(None),
            };
            // Find the key whose value equals our lookup name.
            for (cwd_str, name_val) in map.iter() {
                if name_val.as_str() == Some(key.as_str()) {
                    return Ok(Some(PathBuf::from(cwd_str)));
                }
            }
            Ok(None)
        }
    }
}

/// Resolve a `key_source` string (e.g. "parent_dir_name",
/// "parent_parent_dir_name") against a session candidate's path.
/// Literal strings fall through unchanged.
fn resolve_key_source(key_source: &str, path: &Path) -> String {
    match key_source {
        "parent_dir_name" => path
            .parent()
            .and_then(|p| p.file_name().and_then(|s| s.to_str()))
            .unwrap_or("")
            .to_string(),
        "parent_parent_dir_name" => path
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.file_name().and_then(|s| s.to_str()))
            .unwrap_or("")
            .to_string(),
        other => other.to_string(),
    }
}

/// Decode CWD from a Windows dash-encoded directory name.
/// Example: `D--Demo--agent-session-tui` → `D:\Demo\agent-session-tui`.
///
/// When `backtrack` is true, hyphens in the remainder are ambiguous (they could
/// be path separators or literal hyphens). We try greedy prefixes against disk
/// to find the longest prefix that exists as a directory, then recurse. This
/// matches the legacy Claude behavior for paths like `C--Users-a2a-cli` where
/// the leaf could be `a2a\cli` or `a2a-cli`.
fn decode_cwd_name(name: &str, decoder: &str, backtrack: bool) -> PathBuf {
    match decoder {
        "drive_dash" => {
            if backtrack {
                if let Some(p) = drive_dash_backtrack(name) {
                    return p;
                }
            }
            // Naive: leading "X--" → "X:\", internal "--" → "\", single '-' stays
            let mut out = String::new();
            let bytes = name.as_bytes();
            let mut i = 0usize;
            if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b'-' && bytes[2] == b'-' {
                out.push(bytes[0] as char);
                out.push(':');
                out.push('\\');
                i = 3;
            }
            while i < bytes.len() {
                if i + 1 < bytes.len() && bytes[i] == b'-' && bytes[i + 1] == b'-' {
                    out.push('\\');
                    i += 2;
                } else {
                    out.push(bytes[i] as char);
                    i += 1;
                }
            }
            PathBuf::from(out)
        }
        _ => PathBuf::from(name),
    }
}

/// Backtracking decoder for Claude-style lossy dash-encoded paths.
/// Splits drive prefix `X--…`, then tries progressively-longer hyphen groupings
/// for each segment, probing disk existence to disambiguate.
fn drive_dash_backtrack(encoded: &str) -> Option<PathBuf> {
    let (drive, remainder) = match encoded.find("--") {
        Some(pos) => {
            let drive = format!("{}:\\", &encoded[..pos]);
            let rest = if pos + 2 < encoded.len() { &encoded[pos + 2..] } else { "" };
            (drive, rest)
        }
        None => return None,
    };
    if remainder.is_empty() {
        return Some(PathBuf::from(drive));
    }
    let segments: Vec<&str> = remainder.split('-').collect();

    fn go(base: &Path, segs: &[&str], idx: usize) -> Option<PathBuf> {
        if idx >= segs.len() {
            return Some(base.to_path_buf());
        }
        let mut combined = segs[idx].to_string();
        for end in idx + 1..=segs.len() {
            let candidate = base.join(&combined);
            if end == segs.len() {
                if candidate.exists() {
                    return Some(candidate);
                }
            } else if candidate.is_dir() {
                if let Some(result) = go(&candidate, segs, end) {
                    return Some(result);
                }
            }
            if end < segs.len() {
                combined = format!("{}-{}", combined, segs[end]);
            }
        }
        // No disk match; consume the rest as a literal hyphenated leaf.
        let mut fallback = segs[idx].to_string();
        for s in &segs[idx + 1..] {
            fallback = format!("{}-{}", fallback, s);
        }
        Some(base.join(fallback))
    }

    let drive_path = PathBuf::from(&drive);
    go(&drive_path, &segments, 0)
}

// ─────────────────────────────────────────────────────────────────────────────
// Field extraction
// ─────────────────────────────────────────────────────────────────────────────

fn extract_field(
    prov: &ConfigDrivenProvider,
    meta: &Option<Value>,
    events: &[&Value],
    spec: &FieldSpec,
) -> Option<String> {
    let path = prov.expr(&spec.path);
    let predicate = spec.r#where.as_ref().map(|s| prov.expr(s));

    let raw = match spec.strategy.as_str() {
        "metadata_field" => meta.as_ref().and_then(|m| path.eval_str(m)),
        "first_matching_event" => events.iter().find(|ev| match &predicate {
            Some(p) => p.eval_bool(ev),
            None => true,
        }).and_then(|ev| path.eval_str(ev)),
        "last_matching_event" => events.iter().rev().find(|ev| match &predicate {
            Some(p) => p.eval_bool(ev),
            None => true,
        }).and_then(|ev| path.eval_str(ev)),
        "joined_events" => {
            let sep = spec.join.as_deref().unwrap_or("\n");
            let parts: Vec<String> = events
                .iter()
                .filter(|ev| match &predicate {
                    Some(p) => p.eval_bool(ev),
                    None => true,
                })
                .filter_map(|ev| path.eval_str(ev))
                .collect();
            if parts.is_empty() { None } else { Some(parts.join(sep)) }
        }
        _ => None,
    };

    raw.map(|s| apply_transforms(&s, &spec.transforms, spec.limit))
}

fn apply_transforms(input: &str, transforms: &[String], limit: Option<usize>) -> String {
    let mut s = input.to_string();
    for t in transforms {
        let (name, arg) = match t.split_once(':') {
            Some((n, a)) => (n, Some(a)),
            None => (t.as_str(), None),
        };
        s = match name {
            "trim" => s.trim().to_string(),
            "first_line" => s.lines().next().unwrap_or("").to_string(),
            "strip_newlines" => s.replace(['\n', '\r'], " "),
            "truncate" => {
                let n = arg.and_then(|a| a.parse::<usize>().ok()).unwrap_or(60);
                truncate_str_safe(&s, n)
            }
            _ => s,
        };
    }
    if let Some(l) = limit {
        s = truncate_str_safe(&s, l);
    }
    s
}

fn extract_timestamp(
    prov: &ConfigDrivenProvider,
    meta: &Option<Value>,
    events: &[&Value],
    spec: &TimestampSpec,
    file_mtime: Option<&str>,
) -> Option<String> {
    let result = match spec.strategy.as_str() {
        "metadata_field" => {
            let path = prov.expr(spec.path.as_deref()?);
            meta.as_ref().and_then(|m| path.eval_str(m))
        }
        "first_event_field" => {
            let path = prov.expr(spec.path.as_deref()?);
            events.first().and_then(|ev| path.eval_str(ev))
        }
        "last_event_field" => {
            let path = prov.expr(spec.path.as_deref()?);
            events.last().and_then(|ev| path.eval_str(ev))
        }
        "file_mtime" => file_mtime.map(|s| s.to_string()),
        _ => None,
    };
    if result.is_some() {
        return result;
    }
    for fb in &spec.fallback {
        if let Some(v) = extract_timestamp(prov, meta, events, fb, file_mtime) {
            return Some(v);
        }
    }
    None
}

// ─────────────────────────────────────────────────────────────────────────────
// Process matching
// ─────────────────────────────────────────────────────────────────────────────

fn match_processes_dispatch(
    prov: &ConfigDrivenProvider,
    sessions: &mut [Session],
) -> Result<()> {
    match &prov.cfg.process_match {
        ProcessMatchConfig::Lockfile { lockfile_pattern, pid_extract_regex } => {
            match_lockfile(sessions, lockfile_pattern, pid_extract_regex);
        }
        ProcessMatchConfig::Cmdline { executable, script_contains, not_contains, id_match, recently_active_secs } => {
            match_cmdline(sessions, executable, script_contains.as_deref(), not_contains.as_deref(), id_match, *recently_active_secs);
        }
    }

    // Build state signals from events, then run default_state_inference
    for s in sessions.iter_mut() {
        let Some(dir) = s.state_dir.as_deref() else { continue };
        let events = read_session_events(prov, dir).unwrap_or_default();
        let signals = build_state_signals(prov, &events, s);
        s.state = prov.infer_state(&signals);
    }
    Ok(())
}

fn match_lockfile(sessions: &mut [Session], pattern: &str, pid_regex: &str) {
    for s in sessions.iter_mut() {
        let Some(dir) = s.state_dir.as_deref() else { continue };
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        let mut best_pid: Option<u32> = None;
        let mut best_mtime: Option<std::time::SystemTime> = None;
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !matches_simple_glob(&e.path(), dir, pattern) {
                continue;
            }
            let Some(pid) = regex_capture1(pid_regex, &name).and_then(|s| s.parse::<u32>().ok()) else { continue };
            let mtime = e.metadata().ok().and_then(|m| m.modified().ok());
            if best_mtime.is_none() || mtime > best_mtime {
                best_mtime = mtime;
                best_pid = Some(pid);
            }
        }
        if let Some(pid) = best_pid {
            let alive = process_is_alive(pid);
            s.pid = Some(pid);
            // Store state signals via side-channel below in build_state_signals.
            if alive {
                s.state.process = ProcessState::Running;
            }
        }
    }
}

fn match_cmdline(
    sessions: &mut [Session],
    executable: &str,
    script_contains: Option<&str>,
    not_contains: Option<&str>,
    id_match: &CmdlineIdMatch,
    recently_active_secs: Option<u64>,
) {
    let procs = discover_processes(executable);
    use std::collections::HashSet;
    let mut claimed: HashSet<u32> = HashSet::new();

    let cmd_passes_filters = |cmd: &str| -> bool {
        if let Some(req) = script_contains {
            if !cmd.contains(req) { return false; }
        }
        if let Some(forbidden) = not_contains {
            if cmd.contains(forbidden) { return false; }
        }
        true
    };

    // Direct ID match pass
    for s in sessions.iter_mut() {
        let mut found: Option<u32> = None;
        for (pid, p) in &procs {
            if claimed.contains(pid) { continue; }
            let cmd = &p.command_line;
            if !cmd_passes_filters(cmd) { continue; }
            let matched = match id_match {
                CmdlineIdMatch::Flag { flag } => {
                    flag.as_slice().iter().any(|f|
                        extract_flag_value(cmd, f).as_deref() == Some(s.provider_session_id.as_str())
                    )
                }
                CmdlineIdMatch::PositionalUuid => {
                    cmd.split_whitespace().any(|a| a == s.provider_session_id)
                }
                CmdlineIdMatch::Contains => cmd.contains(&s.provider_session_id),
            };
            if matched {
                found = Some(*pid);
                break;
            }
        }
        if let Some(pid) = found {
            claimed.insert(pid);
            s.pid = Some(pid);
            s.state.process = ProcessState::Running;
        }
    }

    // Recently-active fallback — for sessions still unmatched whose events are fresh
    if let Some(window) = recently_active_secs {
        for s in sessions.iter_mut() {
            if s.pid.is_some() { continue; }
            let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(&s.updated_at) else { continue };
            let delta = chrono::Utc::now().signed_duration_since(parsed.to_utc());
            if delta.num_seconds() > window as i64 { continue; }
            for (pid, p) in &procs {
                if claimed.contains(pid) { continue; }
                let cmd = &p.command_line;
                if !cmd_passes_filters(cmd) { continue; }
                claimed.insert(*pid);
                s.pid = Some(*pid);
                s.state.process = ProcessState::Running;
                break;
            }
        }
    }
}

fn process_is_alive(pid: u32) -> bool {
    use sysinfo::System;
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    sys.process(sysinfo::Pid::from_u32(pid)).is_some()
}

fn read_session_events(prov: &ConfigDrivenProvider, state_dir: &Path) -> Result<Vec<Value>> {
    match &prov.cfg.discovery.strategy {
        DiscoveryStrategy::DirPerSession { events_file, tail_bytes, .. } => {
            read_jsonl_tail(&state_dir.join(events_file), *tail_bytes)
        }
        DiscoveryStrategy::FilePerSession { tail_bytes, .. }
        | DiscoveryStrategy::DatePartitioned { tail_bytes, .. } => {
            read_jsonl_tail(state_dir, *tail_bytes)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// State signals
// ─────────────────────────────────────────────────────────────────────────────

fn build_state_signals(
    prov: &ConfigDrivenProvider,
    events: &[Value],
    session: &Session,
) -> StateSignals {
    let cfg = &prov.cfg.state_signals;
    let mut signals = StateSignals::default();
    signals.pid = session.pid;
    signals.process_alive = session.pid.map(process_is_alive);

    // last_event_age
    if let Some(last) = events.last() {
        let ts_paths = ["timestamp", "startTime", "payload.timestamp", "data.startTime"];
        let mut age: Option<u64> = None;
        for p in ts_paths.iter() {
            if let Some(s) = prov.expr(p).eval_str(last) {
                if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&s) {
                    let d = chrono::Utc::now().signed_duration_since(dt.to_utc());
                    age = Some(d.num_seconds().max(0) as u64);
                    break;
                }
            }
        }
        signals.last_event_age_secs = age;
    }

    // last_event_map — match the latest event type against the map
    if !cfg.last_event_map.is_empty() {
        for ev in events.iter().rev() {
            let ty = prov.expr("type").eval_str(ev).unwrap_or_default();
            if let Some(delta) = cfg.last_event_map.get(&ty) {
                apply_state_delta(&mut signals, delta, session);
                break;
            }
        }
    }

    // event_predicates — ordered list of (where-predicate → delta) pairs.
    // Scan events most-recent-first; for each event, check predicates in
    // order. The first (event, predicate) match wins. Only applied when
    // last_event_map did not already force an interaction.
    if !cfg.event_predicates.is_empty() && signals.forced_interaction.is_none() {
        let compiled: Vec<(Expr, &StateSignalDelta)> = cfg
            .event_predicates
            .iter()
            .map(|p| (prov.expr(&p.r#where), &p.delta))
            .collect();
        'outer: for ev in events.iter().rev() {
            for (expr, delta) in compiled.iter() {
                if expr.eval_bool(ev) {
                    apply_state_delta(&mut signals, delta, session);
                    break 'outer;
                }
            }
        }
    }

    if let Some(expr_src) = &cfg.unfinished_turn_when {
        let e = prov.expr(expr_src);
        signals.has_unfinished_turn = Some(events.iter().any(|ev| e.eval_bool(ev)));
    }
    if let Some(expr_src) = &cfg.recent_tool_activity_when {
        let e = prov.expr(expr_src);
        signals.recent_tool_activity = Some(events.iter().rev().take(20).any(|ev| e.eval_bool(ev)));
    }

    // lockfile-strategy sets lock_file_* via scan
    if let ProcessMatchConfig::Lockfile { lockfile_pattern, pid_extract_regex } = &prov.cfg.process_match {
        if let Some(dir) = &session.state_dir {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for e in entries.flatten() {
                    let name = e.file_name().to_string_lossy().to_string();
                    if !matches_simple_glob(&e.path(), dir, lockfile_pattern) { continue; }
                    signals.lock_file_exists = Some(true);
                    if let Some(pid) = regex_capture1(pid_extract_regex, &name).and_then(|s| s.parse::<u32>().ok()) {
                        signals.lock_file_pid = Some(pid);
                    }
                }
            }
        }
    }

    signals
}

fn apply_state_delta(signals: &mut StateSignals, delta: &StateSignalDelta, _session: &Session) {
    if let Some(i) = &delta.interaction {
        signals.forced_interaction = Some(i.clone());
        match i.as_str() {
            "busy" => signals.has_unfinished_turn = Some(true),
            "waiting_input" => signals.has_unfinished_turn = Some(false),
            "idle" => { signals.has_unfinished_turn = Some(false); signals.recent_tool_activity = Some(false); }
            _ => {}
        }
    }
    if let Some(p) = &delta.process {
        match p.as_str() {
            "exited" => signals.process_alive = Some(false),
            "running" => signals.process_alive = Some(true),
            _ => {}
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tab title
// ─────────────────────────────────────────────────────────────────────────────

fn extract_tab_title(
    prov: &ConfigDrivenProvider,
    cfg: &TabTitleConfig,
    events: &[Value],
) -> Option<String> {
    match cfg {
        TabTitleConfig::None => None,
        TabTitleConfig::Literal { value } => Some(value.clone()),
        // Handled in the provider's `tab_title()` method where `session.cwd`
        // is available; reaching here means no events will help.
        TabTitleConfig::CwdBasename => None,
        TabTitleConfig::FromField { r#where, path } => {
            let w = prov.expr(r#where);
            let p = prov.expr(path);
            events.iter().rev().find(|ev| w.eval_bool(ev)).and_then(|ev| p.eval_str(ev))
        }
        TabTitleConfig::FromToolCall {
            tool_name, r#where, tool_name_path, args_path, arg_field, iterate_path,
        } => {
            let w = prov.expr(r#where);
            let tnp = prov.expr(tool_name_path);
            let ap = prov.expr(args_path);
            let af = prov.expr(arg_field);
            let ip = iterate_path.as_ref().map(|p| prov.expr(p));
            for ev in events.iter().rev() {
                if !w.eval_bool(ev) { continue; }
                // Build the list of "tool call" values to scan.
                let tool_calls: Vec<Value> = if let Some(ref ipe) = ip {
                    match ipe.eval(ev) {
                        Value::Array(arr) => arr,
                        _ => continue,
                    }
                } else {
                    vec![ev.clone()]
                };
                for tc in tool_calls.iter().rev() {
                    if tnp.eval_str(tc).as_deref() != Some(tool_name) { continue; }
                    let args_value = ap.eval(tc);
                    let candidate = af.eval(&args_value);
                    if let Some(s) = match candidate {
                        Value::String(s) => Some(s),
                        Value::Null => None,
                        v => Some(v.to_string()),
                    } {
                        return Some(s);
                    }
                }
            }
            None
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tiny regex helpers (uses Rust's std? no — we can't depend on `regex` without
// adding it; rely on a hand-rolled limited matcher since patterns are tame)
// ─────────────────────────────────────────────────────────────────────────────

/// Very small regex matcher: supports `.`, `*`, `+`, `?`, `[]`, `^`, `$`, `\\d`,
/// `\\.`, character classes `[abc]` / `[a-z]`, and capture groups `()`.
/// This is deliberately minimal — YAML authors should keep patterns simple.
fn regex_match(pat: &str, input: &str) -> bool {
    // Quick & dirty: leverage std's built-in pattern functions where possible;
    // otherwise implement a scanner. We reuse the crate's existing approach:
    // translate our limited grammar to a character-by-character NFA walk.
    tiny_regex::matches(pat, input)
}

fn regex_capture1(pat: &str, input: &str) -> Option<String> {
    tiny_regex::capture1(pat, input)
}

// `expand_path` is already in scope via `use schema::*;`

// ── Tiny regex — extracted to keep mod top clean ────────────────────────────
mod tiny_regex {
    //! Ultra-small regex: enough for lockfile pid extraction and simple globs.
    //! Supports: literals, `.`, `*`, `+`, `?`, character classes `[...]`,
    //! escaped `\\.`, `\\d`, anchors `^` `$`, one `()` capture group.
    //!
    //! Not robust for general regex but sufficient for our patterns.

    pub fn matches(pat: &str, input: &str) -> bool {
        do_match(pat, input, false).is_some()
    }
    pub fn capture1(pat: &str, input: &str) -> Option<String> {
        do_match(pat, input, true)
    }

    fn do_match(pat: &str, input: &str, want_cap: bool) -> Option<String> {
        // Compile to nodes
        let mut nodes = Vec::new();
        let mut cap_start = None;
        let mut cap_end = None;
        let bytes = pat.as_bytes();
        let mut i = 0usize;
        let mut anchored_start = false;
        let mut anchored_end = false;
        if i < bytes.len() && bytes[i] == b'^' { anchored_start = true; i += 1; }
        while i < bytes.len() {
            let c = bytes[i];
            if c == b'$' && i + 1 == bytes.len() { anchored_end = true; break; }
            if c == b'(' {
                cap_start = Some(nodes.len());
                i += 1;
                continue;
            }
            if c == b')' {
                cap_end = Some(nodes.len());
                i += 1;
                continue;
            }
            let (node, consumed) = parse_atom(&bytes[i..])?;
            i += consumed;
            // quantifier?
            let (q_node, q_consumed) = apply_quant(node, &bytes[i..]);
            i += q_consumed;
            nodes.push(q_node);
        }

        // try matching at each starting position
        for start in 0..=input.len() {
            if anchored_start && start != 0 { break; }
            if let Some((end, cap)) = match_nodes(&nodes, input, start, cap_start, cap_end) {
                if anchored_end && end != input.len() { continue; }
                if want_cap {
                    return Some(cap.unwrap_or_else(|| input[start..end].to_string()));
                } else {
                    return Some(String::new());
                }
            }
        }
        None
    }

    #[derive(Debug, Clone)]
    enum Node {
        Lit(u8),
        Any,        // .
        Digit,      // \d
        Class(Vec<(u8, u8)>, bool), // ranges, negate
        Rep(Box<Node>, usize, Option<usize>), // min, max
    }

    fn parse_atom(b: &[u8]) -> Option<(Node, usize)> {
        if b.is_empty() { return None; }
        match b[0] {
            b'.' => Some((Node::Any, 1)),
            b'\\' => {
                if b.len() < 2 { return None; }
                match b[1] {
                    b'd' => Some((Node::Digit, 2)),
                    c => Some((Node::Lit(c), 2)),
                }
            }
            b'[' => {
                let mut j = 1;
                let mut negate = false;
                if j < b.len() && b[j] == b'^' { negate = true; j += 1; }
                let mut ranges = Vec::new();
                while j < b.len() && b[j] != b']' {
                    let lo = b[j];
                    if j + 2 < b.len() && b[j + 1] == b'-' && b[j + 2] != b']' {
                        let hi = b[j + 2];
                        ranges.push((lo, hi));
                        j += 3;
                    } else {
                        ranges.push((lo, lo));
                        j += 1;
                    }
                }
                if j >= b.len() { return None; }
                Some((Node::Class(ranges, negate), j + 1))
            }
            c => Some((Node::Lit(c), 1)),
        }
    }

    fn apply_quant(node: Node, b: &[u8]) -> (Node, usize) {
        if b.is_empty() { return (node, 0); }
        match b[0] {
            b'*' => (Node::Rep(Box::new(node), 0, None), 1),
            b'+' => (Node::Rep(Box::new(node), 1, None), 1),
            b'?' => (Node::Rep(Box::new(node), 0, Some(1)), 1),
            _ => (node, 0),
        }
    }

    fn node_matches_byte(node: &Node, b: u8) -> bool {
        match node {
            Node::Lit(x) => *x == b,
            Node::Any => true,
            Node::Digit => b.is_ascii_digit(),
            Node::Class(ranges, neg) => {
                let hit = ranges.iter().any(|(lo, hi)| b >= *lo && b <= *hi);
                if *neg { !hit } else { hit }
            }
            _ => false,
        }
    }

    fn match_nodes(
        nodes: &[Node],
        input: &str,
        start: usize,
        cap_start: Option<usize>,
        cap_end: Option<usize>,
    ) -> Option<(usize, Option<String>)> {
        let bytes = input.as_bytes();
        match_rec(nodes, 0, bytes, start, cap_start, cap_end, None, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn match_rec(
        nodes: &[Node],
        ni: usize,
        input: &[u8],
        pos: usize,
        cap_start: Option<usize>,
        cap_end: Option<usize>,
        cap_start_pos: Option<usize>,
        cap_end_pos: Option<usize>,
    ) -> Option<(usize, Option<String>)> {
        let cap_start_pos = if Some(ni) == cap_start { Some(pos) } else { cap_start_pos };
        let cap_end_pos = if Some(ni) == cap_end { Some(pos) } else { cap_end_pos };

        if ni == nodes.len() {
            let cap = match (cap_start_pos, cap_end_pos) {
                (Some(s), Some(e)) if e >= s => Some(std::str::from_utf8(&input[s..e]).ok()?.to_string()),
                _ => None,
            };
            return Some((pos, cap));
        }
        match &nodes[ni] {
            Node::Rep(inner, min, max) => {
                let mut count = 0usize;
                let mut p = pos;
                while count < *min {
                    if p >= input.len() { return None; }
                    if !node_matches_byte(inner, input[p]) { return None; }
                    p += 1;
                    count += 1;
                }
                // greedy — try longest, backtrack
                let mut positions = vec![p];
                while p < input.len() {
                    let hit_max = max.map(|m| count >= m).unwrap_or(false);
                    if hit_max { break; }
                    if !node_matches_byte(inner, input[p]) { break; }
                    p += 1;
                    count += 1;
                    positions.push(p);
                }
                for try_p in positions.into_iter().rev() {
                    if let Some(r) = match_rec(nodes, ni + 1, input, try_p, cap_start, cap_end, cap_start_pos, cap_end_pos) {
                        return Some(r);
                    }
                }
                None
            }
            other => {
                if pos >= input.len() { return None; }
                if !node_matches_byte(other, input[pos]) { return None; }
                match_rec(nodes, ni + 1, input, pos + 1, cap_start, cap_end, cap_start_pos, cap_end_pos)
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_dash_decodes() {
        assert_eq!(
            decode_cwd_name("D--Demo--agent-session-tui", "drive_dash", false),
            PathBuf::from("D:\\Demo\\agent-session-tui")
        );
    }

    #[test]
    fn drive_dash_backtrack_handles_hyphen_in_leaf() {
        // No disk → backtrack gives up and returns the fully-hyphenated leaf.
        // This confirms we at least do not crash and prefer the conservative
        // "keep the hyphen" interpretation when the path doesn't exist.
        let p = decode_cwd_name(
            "Z--DoesNotExist-subdir-with-hyphens",
            "drive_dash",
            true,
        );
        assert_eq!(p, PathBuf::from("Z:\\DoesNotExist-subdir-with-hyphens"));
    }

    #[test]
    fn tiny_regex_pid_extract() {
        let p = tiny_regex::capture1(r"^inuse\.([0-9]+)\.lock$", "inuse.1234.lock");
        assert_eq!(p.as_deref(), Some("1234"));
    }

    #[test]
    fn tiny_regex_no_match() {
        assert!(tiny_regex::capture1(r"^inuse\.([0-9]+)\.lock$", "random.txt").is_none());
    }

    #[test]
    fn apply_transforms_basic() {
        assert_eq!(
            apply_transforms("hello world", &["first_line".into(), "truncate:5".into()], None),
            "hello…"
        );
    }

    /// End-to-end smoke test: parse `providers/copilot.yaml` against a synthetic
    /// Copilot session on disk. Exercises schema loading, discovery,
    /// title/summary/timestamp extraction, and state signal detection.
    #[test]
    fn copilot_yaml_end_to_end() {
        use std::fs;

        // Locate providers/copilot.yaml relative to the crate root.
        let yaml = Path::new(env!("CARGO_MANIFEST_DIR")).join("providers").join("copilot.yaml");
        assert!(yaml.exists(), "providers/copilot.yaml missing");

        // Build a synthetic session tree.
        let tmp = tempfile::tempdir().unwrap();
        let sid = "11111111-2222-3333-4444-555555555555";
        let sess = tmp.path().join(sid);
        fs::create_dir_all(&sess).unwrap();
        fs::write(
            sess.join("workspace.yaml"),
            "cwd: D:\\Demo\\agent-session-tui\nsummary: Build the TUI\n",
        )
        .unwrap();
        fs::write(
            sess.join("events.jsonl"),
            r#"{"type":"user.message","timestamp":"2024-01-01T00:00:00Z","data":{"content":"hello world first line\nsecond"}}
{"type":"assistant.message","timestamp":"2024-01-01T00:00:01Z","data":{"toolRequests":[{"name":"report_intent","arguments":{"intent":"Exploring codebase"}}]}}
{"type":"assistant.turn_end","timestamp":"2024-01-01T00:00:02Z"}
"#,
        )
        .unwrap();

        // Load provider with the synthetic base_dir via AppProviderConfig.state_dir override.
        let app_cfg = AppProviderConfig {
            enabled: true,
            default: false,
            command: "copilot".into(),
            default_args: vec![],
            state_dir: Some(tmp.path().to_path_buf()),
            resume_flag: Some("--resume".into()),
            startup_dir: None,
            launch_method: "wt".into(),
            launch_cmd: None,
            launch_args: None,
            launch_fallback_cmd: None,
            launch_fallback_args: None,
            launch_fallback: None,
            wt_profile: None,
        };
        let prov = ConfigDrivenProvider::load_from_yaml(&yaml, &app_cfg)
            .expect("load providers/copilot.yaml");

        // Discovery.
        let sessions = prov.discover_sessions().expect("discover_sessions");
        assert_eq!(sessions.len(), 1, "expected exactly one session");
        let s = &sessions[0];
        assert_eq!(s.provider_session_id, sid);
        assert_eq!(s.provider_name, "copilot");
        assert_eq!(s.title, "hello world first line second");
        assert_eq!(s.summary, "Build the TUI");
        assert!(s.cwd.ends_with("agent-session-tui"));
        assert!(!s.updated_at.is_empty(), "updated_at missing");

        // Tab title from `report_intent` tool call.
        let tab = prov.tab_title(s);
        assert_eq!(tab.as_deref(), Some("Exploring codebase"));
    }

    /// End-to-end smoke test: parse `providers/claude.yaml` against a synthetic
    /// Claude project tree. Exercises file_per_session discovery, dirname_decode
    /// with backtracking, hide_paths_glob (memory/subagents), last-role state
    /// inference, and the literal tab title strategy.
    #[test]
    fn claude_yaml_end_to_end() {
        use std::fs;

        let yaml = Path::new(env!("CARGO_MANIFEST_DIR")).join("providers").join("claude.yaml");
        assert!(yaml.exists(), "providers/claude.yaml missing");

        // Synthetic `projects/` tree.
        let tmp = tempfile::tempdir().unwrap();
        let sid = "11111111-2222-3333-4444-555555555555";
        // Encoded CWD — `Z:` is almost never a real drive, so backtracking
        // harmlessly falls back to the literal hyphenated leaf.
        let proj_dir = tmp.path().join("Z--synth-proj");
        fs::create_dir_all(&proj_dir).unwrap();
        fs::write(
            proj_dir.join(format!("{}.jsonl", sid)),
            r#"{"type":"user","timestamp":"2024-02-01T00:00:00Z","message":{"content":"first user question\nsecond line"}}
{"type":"assistant","timestamp":"2024-02-01T00:00:01Z","message":{"content":"first assistant reply"}}
{"type":"user","timestamp":"2024-02-01T00:00:02Z","message":{"content":"follow up"}}
{"type":"assistant","timestamp":"2024-02-01T00:00:03Z","message":{"content":"final reply"}}
"#,
        )
        .unwrap();

        // Hidden: memory dir (Claude's own memory) — must be skipped.
        let mem = tmp.path().join("memory");
        fs::create_dir_all(&mem).unwrap();
        fs::write(
            mem.join("abc.jsonl"),
            r#"{"type":"user","message":{"content":"should be hidden"}}
"#,
        )
        .unwrap();

        // Hidden: subagents dir — must be skipped.
        let sub = tmp.path().join("Z--synth-proj").join("subagents");
        fs::create_dir_all(&sub).unwrap();
        fs::write(
            sub.join("sub.jsonl"),
            r#"{"type":"user","message":{"content":"also hidden"}}
"#,
        )
        .unwrap();

        let app_cfg = AppProviderConfig {
            enabled: true,
            default: false,
            command: "claude".into(),
            default_args: vec![],
            state_dir: Some(tmp.path().to_path_buf()),
            resume_flag: Some("--continue".into()),
            startup_dir: None,
            launch_method: "wt".into(),
            launch_cmd: None,
            launch_args: None,
            launch_fallback_cmd: None,
            launch_fallback_args: None,
            launch_fallback: None,
            wt_profile: None,
        };
        let prov = ConfigDrivenProvider::load_from_yaml(&yaml, &app_cfg)
            .expect("load providers/claude.yaml");

        let sessions = prov.discover_sessions().expect("discover_sessions");
        assert_eq!(sessions.len(), 1, "memory + subagents should be hidden");
        for s in &sessions {
            eprintln!("  -> id={} cwd={:?} title={:?}", s.provider_session_id, s.cwd, s.title);
        }
        assert_eq!(sessions.len(), 1, "memory + subagents should be hidden");
        let s = &sessions[0];
        assert_eq!(s.provider_session_id, sid);
        assert_eq!(s.provider_name, "claude");
        assert_eq!(s.title, "first user question");
        assert!(s.summary.starts_with("first user question"), "summary: {:?}", s.summary);
        // Backtracking with a non-existent drive falls back to literal leaf.
        assert!(s.cwd.ends_with("synth-proj"), "cwd: {:?}", s.cwd);
        assert!(!s.updated_at.is_empty(), "updated_at missing");

        // Literal tab title — any terminal tab containing "✳" matches.
        // `tab_title(&Session)` returns the sentinel value for matching.
        let tab = prov.tab_title(s);
        assert_eq!(tab.as_deref(), Some("✳"));
    }

    /// End-to-end smoke test: parse `providers/codex.yaml` against a synthetic
    /// Codex sessions tree. Exercises `date_partitioned` discovery, the
    /// `session_meta` → `payload.id` / `payload.cwd` extraction, the
    /// `event_predicates` state inference (task_started/task_complete,
    /// response_item role), the `payload.content.0.text` title path, and the
    /// `cwd_basename` tab title.
    #[test]
    fn codex_yaml_end_to_end() {
        use std::fs;

        let yaml = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("providers")
            .join("codex.yaml");
        assert!(yaml.exists(), "providers/codex.yaml missing");

        let tmp = tempfile::tempdir().unwrap();
        let sid = "019d6fa7-45f6-7951-aefa-efafb1f3b826";
        // Real layout: YYYY/MM/DD/rollout-<iso>-<uuid>.jsonl
        let day = tmp.path().join("2026").join("04").join("20");
        fs::create_dir_all(&day).unwrap();
        let file = day.join(format!("rollout-2026-04-20T00-00-00-{}.jsonl", sid));
        // cwd uses a Windows-style path in payload.cwd; the basename should be
        // "yaml-demo" which is what the tab title test asserts.
        fs::write(
            &file,
            r#"{"timestamp":"2026-04-20T00:00:00Z","type":"session_meta","payload":{"id":"019d6fa7-45f6-7951-aefa-efafb1f3b826","timestamp":"2026-04-20T00:00:00Z","cwd":"C:\\Users\\yeelam\\yaml-demo","cli_version":"0.118.0"}}
{"timestamp":"2026-04-20T00:00:01Z","type":"event_msg","payload":{"type":"task_started","turn_id":"t1"}}
{"timestamp":"2026-04-20T00:00:02Z","type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"<permissions instructions>ignored bootstrap</permissions instructions>"}]}}
{"timestamp":"2026-04-20T00:00:03Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"refactor the config loader\nfollow-up line"}]}}
{"timestamp":"2026-04-20T00:00:04Z","type":"event_msg","payload":{"type":"token_count","total":12345}}
{"timestamp":"2026-04-20T00:00:05Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"I will refactor the config loader."}]}}
{"timestamp":"2026-04-20T00:00:06Z","type":"event_msg","payload":{"type":"task_complete","turn_id":"t1"}}
"#,
        )
        .unwrap();

        let app_cfg = AppProviderConfig {
            enabled: true,
            default: false,
            command: "codex".into(),
            default_args: vec![],
            state_dir: Some(tmp.path().to_path_buf()),
            resume_flag: Some("resume".into()),
            startup_dir: None,
            launch_method: "wt".into(),
            launch_cmd: None,
            launch_args: None,
            launch_fallback_cmd: None,
            launch_fallback_args: None,
            launch_fallback: None,
            wt_profile: None,
        };
        let prov = ConfigDrivenProvider::load_from_yaml(&yaml, &app_cfg)
            .expect("load providers/codex.yaml");

        let sessions = prov.discover_sessions().expect("discover_sessions");
        assert_eq!(sessions.len(), 1, "expected exactly one Codex session");
        let s = &sessions[0];

        // session_id comes from payload.id in session_meta, NOT from filename.
        assert_eq!(s.provider_session_id, sid);
        assert_eq!(s.provider_name, "codex");

        // Title skips the `developer` bootstrap message and picks the first
        // real user response_item, first line, truncated.
        assert_eq!(s.title, "refactor the config loader");
        assert!(
            s.summary.starts_with("refactor the config loader"),
            "summary: {:?}",
            s.summary
        );

        // cwd from session_meta.payload.cwd — ends with the last folder.
        assert!(s.cwd.ends_with("yaml-demo"), "cwd: {:?}", s.cwd);
        assert!(!s.updated_at.is_empty(), "updated_at missing");

        // Tab title is the cwd basename.
        let tab = prov.tab_title(s);
        assert_eq!(tab.as_deref(), Some("yaml-demo"));

        // State signals: the most recent matching event is task_complete,
        // which should force interaction=waiting_input.
        let signals = build_state_signals(&prov, &[
            serde_json::from_str(r#"{"timestamp":"2026-04-20T00:00:01Z","type":"event_msg","payload":{"type":"task_started"}}"#).unwrap(),
            serde_json::from_str(r#"{"timestamp":"2026-04-20T00:00:06Z","type":"event_msg","payload":{"type":"task_complete"}}"#).unwrap(),
        ], s);
        assert_eq!(
            signals.forced_interaction.as_deref(),
            Some("waiting_input"),
            "event_predicates should resolve latest task_complete → waiting_input"
        );
        assert_eq!(signals.has_unfinished_turn, Some(false));
    }

    /// End-to-end smoke test: parse `providers/qwen.yaml` against a synthetic
    /// `<projects>/<encoded>/chats/<uuid>.jsonl` tree. Exercises the
    /// two-level `*/chats/*.jsonl` glob, `cwd` extraction from event_field
    /// on `type==user` lines, `system`-event filtering (ui_telemetry noise),
    /// `message.parts.0.text` array-indexed title path, last_event_map
    /// state inference after filtering, and `cwd_basename` tab title.
    #[test]
    fn qwen_yaml_end_to_end() {
        use std::fs;

        let yaml = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("providers")
            .join("qwen.yaml");
        assert!(yaml.exists(), "providers/qwen.yaml missing");

        let tmp = tempfile::tempdir().unwrap();
        let sid = "1761af36-3cff-4b40-bdc5-c5d054eef157";
        // <projects>/<encoded-cwd>/chats/<uuid>.jsonl
        let chats = tmp.path().join("d--demo-agent-session-tui").join("chats");
        fs::create_dir_all(&chats).unwrap();
        fs::write(
            chats.join(format!("{}.jsonl", sid)),
            r#"{"uuid":"u1","sessionId":"1761af36-3cff-4b40-bdc5-c5d054eef157","timestamp":"2026-04-20T01:52:17Z","type":"user","cwd":"D:\\Demo\\qwen-demo","version":"0.14.5","message":{"role":"user","parts":[{"text":"testing qwen yaml\nsecond line"}]}}
{"uuid":"u2","sessionId":"1761af36-3cff-4b40-bdc5-c5d054eef157","timestamp":"2026-04-20T01:52:18Z","type":"system","cwd":"D:\\Demo\\qwen-demo","subtype":"ui_telemetry","systemPayload":{"uiEvent":{"event.name":"qwen-code.api_response"}}}
{"uuid":"u3","sessionId":"1761af36-3cff-4b40-bdc5-c5d054eef157","timestamp":"2026-04-20T01:52:20Z","type":"assistant","cwd":"D:\\Demo\\qwen-demo","version":"0.14.5","model":"model-router","message":{"role":"model","parts":[{"text":"Hi! How can I help?"}]}}
"#,
        )
        .unwrap();

        let app_cfg = AppProviderConfig {
            enabled: true,
            default: false,
            command: "qwen".into(),
            default_args: vec![],
            state_dir: Some(tmp.path().to_path_buf()),
            resume_flag: Some("--resume".into()),
            startup_dir: None,
            launch_method: "wt".into(),
            launch_cmd: None,
            launch_args: None,
            launch_fallback_cmd: None,
            launch_fallback_args: None,
            launch_fallback: None,
            wt_profile: None,
        };
        let prov = ConfigDrivenProvider::load_from_yaml(&yaml, &app_cfg)
            .expect("load providers/qwen.yaml");

        let sessions = prov.discover_sessions().expect("discover_sessions");
        assert_eq!(sessions.len(), 1, "expected exactly one Qwen session");
        let s = &sessions[0];

        assert_eq!(s.provider_session_id, sid);
        assert_eq!(s.provider_name, "qwen");
        // Title: first_line of message.parts[0].text, truncated.
        assert_eq!(s.title, "testing qwen yaml");
        assert!(
            s.summary.starts_with("testing qwen yaml"),
            "summary: {:?}",
            s.summary
        );
        // cwd from event_field (user-type line).
        assert!(s.cwd.ends_with("qwen-demo"), "cwd: {:?}", s.cwd);
        assert!(!s.updated_at.is_empty(), "updated_at missing");

        // Tab title is the cwd basename (shared with Codex).
        let tab = prov.tab_title(s);
        assert_eq!(tab.as_deref(), Some("qwen-demo"));

        // State inference: the last non-filtered event is `assistant`, so
        // last_event_map should force interaction = waiting_input. The
        // intervening `system` (ui_telemetry) line must be filtered out —
        // if it leaked through, no last_event_map key would match.
        let events: Vec<serde_json::Value> = vec![
            serde_json::from_str(r#"{"timestamp":"2026-04-20T01:52:17Z","type":"user","cwd":"D:\\Demo\\qwen-demo","message":{"parts":[{"text":"testing qwen yaml"}]}}"#).unwrap(),
            serde_json::from_str(r#"{"timestamp":"2026-04-20T01:52:18Z","type":"system","subtype":"ui_telemetry"}"#).unwrap(),
            serde_json::from_str(r#"{"timestamp":"2026-04-20T01:52:20Z","type":"assistant","message":{"parts":[{"text":"Hi!"}]}}"#).unwrap(),
        ];
        let signals = build_state_signals(&prov, &events, s);
        assert_eq!(
            signals.forced_interaction.as_deref(),
            Some("waiting_input"),
            "last_event_map should resolve latest assistant → waiting_input"
        );
    }

    /// End-to-end smoke test: parse a Gemini-shaped YAML (mirroring the
    /// shipped `providers/gemini.yaml` but with a tempdir-scoped
    /// `lookup_file` for the config_reverse_lookup) against a synthetic
    /// `<tmp>/<project-name>/chats/session-*.jsonl` tree plus a
    /// `projects.json` map. Exercises the new `config_reverse_lookup`
    /// cwd strategy, the `"$set" != null` metadata filter, the
    /// `first_event_field sessionId` strategy on the first-line meta
    /// record, the `content.0.text` title path, and last_event_map
    /// state inference with user/gemini event types.
    #[test]
    fn gemini_yaml_end_to_end() {
        use std::fs;

        let tmp = tempfile::tempdir().unwrap();
        let sid = "992fb9b6-1a53-4a59-84fd-9cae1de984c2";
        let project_name = "agent-session-tui";
        let cwd_literal = "D:\\Demo\\agent-session-tui";

        // <tmp>/<project-name>/chats/session-<iso>-<short>.jsonl
        let chats = tmp.path().join(project_name).join("chats");
        fs::create_dir_all(&chats).unwrap();
        let session_file = chats.join("session-2026-04-20T07-54-992fb9b6.jsonl");
        fs::write(
            &session_file,
            r#"{"sessionId":"992fb9b6-1a53-4a59-84fd-9cae1de984c2","projectHash":"abc","startTime":"2026-04-20T07:54:30.298Z","lastUpdated":"2026-04-20T07:54:30.298Z","kind":"main"}
{"id":"c8d803a5","timestamp":"2026-04-20T07:56:15.010Z","type":"user","content":[{"text":"write a plugin\nsecond paragraph"}]}
{"$set":{"lastUpdated":"2026-04-20T07:56:15.011Z"}}
{"id":"fb166ac2","timestamp":"2026-04-20T07:56:20.351Z","type":"gemini","content":"Starting the plugin work.","thoughts":[{"subject":"plan","description":"assess"}]}
{"$set":{"lastUpdated":"2026-04-20T07:56:20.353Z"}}
{"id":"541effcd","timestamp":"2026-04-20T07:57:12.037Z","type":"info","content":"Request cancelled."}
{"$set":{"lastUpdated":"2026-04-20T07:57:12.039Z"}}
"#,
        )
        .unwrap();

        // Also lay down a sibling subagent continuation dir that the glob
        // MUST skip (`chats/<UUID>/*.jsonl`).
        let subagent_dir = chats.join("992fb9b6-1a53-4a59-84fd-9cae1de984c2");
        fs::create_dir_all(&subagent_dir).unwrap();
        fs::write(
            subagent_dir.join("ptun6d.jsonl"),
            r#"{"sessionId":"sub-1","kind":"subagent"}
"#,
        )
        .unwrap();

        // Fake `projects.json` — keys are cwds, values are project-names.
        let projects_json = tmp.path().join("projects.json");
        fs::write(
            &projects_json,
            r#"{"projects":{"c:\\users\\yeelam":"yeelam","d:\\demo\\agent-session-tui":"agent-session-tui"}}"#,
        )
        .unwrap();

        // Inline YAML mirroring providers/gemini.yaml with a tempdir
        // lookup_file. (The shipped YAML uses ${HOME}/.gemini/projects.json
        // which we can't safely redirect inside a parallel test run.)
        let yaml_src = format!(
            r#"name: gemini
display_name: Gemini CLI
capabilities:
  supports_resume: true
  supports_discovery: true
  supports_logs: true
  supports_wait_detection: true
  supports_kill: true
  supports_archive: true
  supports_summary_extraction: true
discovery:
  base_dir: {base}
  strategy: file_per_session
  glob: "*/chats/session-*.jsonl"
  tail_bytes: 524288
session_id:
  source: first_event_field
  field: "sessionId"
cwd:
  source: config_reverse_lookup
  lookup_file: {lookup}
  key_source: parent_parent_dir_name
  container_path: "projects"
events:
  format: jsonl
  filter_out:
    - '$set != null'
    - 'type == "info"'
fields:
  title:
    strategy: first_matching_event
    where: 'type == "user"'
    path: "content.0.text"
    transforms:
      - first_line
      - truncate:60
  summary:
    strategy: first_matching_event
    where: 'type == "user"'
    path: "content.0.text"
    transforms:
      - truncate:4000
  created_at:
    strategy: first_event_field
    path: "startTime"
    fallback:
      - strategy: file_mtime
  updated_at:
    strategy: file_mtime
state_signals:
  idle_threshold_seconds: 1800
  last_event_map:
    "user":   {{ interaction: busy }}
    "gemini": {{ interaction: waiting_input }}
process_match:
  strategy: cmdline
  executable: gemini
  script_contains: "gemini.js"
  id_match_kind: contains
  recently_active_secs: 15
tab_title:
  strategy: cwd_basename
"#,
            base = tmp.path().display().to_string().replace('\\', "\\\\"),
            lookup = projects_json.display().to_string().replace('\\', "\\\\"),
        );

        let cfg: ProviderConfigFile =
            serde_yaml::from_str(&yaml_src).expect("parse inline gemini yaml");

        let app_cfg = AppProviderConfig {
            enabled: true,
            default: false,
            command: "gemini".into(),
            default_args: vec![],
            state_dir: Some(tmp.path().to_path_buf()),
            resume_flag: None,
            startup_dir: None,
            launch_method: "wt".into(),
            launch_cmd: None,
            launch_args: None,
            launch_fallback_cmd: None,
            launch_fallback_args: None,
            launch_fallback: None,
            wt_profile: None,
        };
        let prov =
            ConfigDrivenProvider::from_config(cfg, &app_cfg).expect("construct gemini provider");

        let sessions = prov.discover_sessions().expect("discover_sessions");
        assert_eq!(
            sessions.len(),
            1,
            "glob must skip chats/<UUID>/*.jsonl subagent continuations"
        );
        let s = &sessions[0];

        // session_id comes from first-line meta sessionId (first_event_field),
        // not the filename — which only carries a short prefix.
        assert_eq!(s.provider_session_id, sid);
        assert_eq!(s.provider_name, "gemini");

        // Title: first `type == "user"` line, content.0.text, first_line only.
        assert_eq!(s.title, "write a plugin");
        assert!(
            s.summary.starts_with("write a plugin"),
            "summary: {:?}",
            s.summary
        );

        // cwd from ConfigReverseLookup on projects.json: the directory
        // `agent-session-tui` matches the VALUE, so we return the KEY
        // (`D:\Demo\agent-session-tui`).
        assert_eq!(
            s.cwd.to_string_lossy().to_lowercase(),
            cwd_literal.to_lowercase()
        );
        assert!(!s.updated_at.is_empty(), "updated_at missing");

        // Tab title uses cwd basename (shared strategy).
        let tab = prov.tab_title(s);
        assert_eq!(tab.as_deref(), Some("agent-session-tui"));

        // State inference: after filtering out `{"$set":...}` metadata and
        // `type == "info"` status, the tail event is `type == "gemini"`,
        // which last_event_map forces to waiting_input. Critically, the
        // `info` line ("Request cancelled.") MUST NOT leak through —
        // it would otherwise be the last event and produce no mapping.
        let events: Vec<serde_json::Value> = vec![
            serde_json::from_str(
                r#"{"timestamp":"2026-04-20T07:56:15.010Z","type":"user","content":[{"text":"write a plugin"}]}"#,
            )
            .unwrap(),
            serde_json::from_str(r#"{"$set":{"lastUpdated":"x"}}"#).unwrap(),
            serde_json::from_str(
                r#"{"timestamp":"2026-04-20T07:56:20.351Z","type":"gemini","content":"ok"}"#,
            )
            .unwrap(),
            serde_json::from_str(
                r#"{"timestamp":"2026-04-20T07:57:12.037Z","type":"info","content":"Request cancelled."}"#,
            )
            .unwrap(),
        ];
        // Filter events as parse_session would.
        let filters: Vec<Expr> = prov
            .cfg
            .events
            .filter_out
            .iter()
            .map(|s| prov.expr(s))
            .collect();
        let kept: Vec<serde_json::Value> = events
            .into_iter()
            .filter(|ev| !filters.iter().any(|f| f.eval_bool(ev)))
            .collect();
        assert_eq!(
            kept.len(),
            2,
            "$set and info lines must both be filtered out, leaving only user+gemini"
        );
        let signals = build_state_signals(&prov, &kept, s);
        assert_eq!(
            signals.forced_interaction.as_deref(),
            Some("waiting_input"),
            "last kept event is type=gemini → waiting_input"
        );
    }

    /// Sanity check: every shipped `providers/<name>.yaml` deserializes
    /// into a ProviderConfigFile without errors. These tests guard the
    /// atomic replacement — if the schema evolves and a YAML lags, CI
    /// fails here instead of at runtime when a user launches the TUI.
    fn load_shipped_yaml(name: &str) -> ProviderConfigFile {
        let yaml = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("providers")
            .join(format!("{name}.yaml"));
        assert!(yaml.exists(), "providers/{name}.yaml missing");
        let text = std::fs::read_to_string(&yaml).unwrap();
        serde_yaml::from_str(&text)
            .unwrap_or_else(|e| panic!("providers/{name}.yaml must parse: {e}"))
    }

    #[test]
    fn providers_copilot_yaml_parses() {
        let cfg = load_shipped_yaml("copilot");
        assert_eq!(cfg.name, "copilot");
    }

    #[test]
    fn providers_claude_yaml_parses() {
        let cfg = load_shipped_yaml("claude");
        assert_eq!(cfg.name, "claude");
    }

    #[test]
    fn providers_codex_yaml_parses() {
        let cfg = load_shipped_yaml("codex");
        assert_eq!(cfg.name, "codex");
    }

    #[test]
    fn providers_qwen_yaml_parses() {
        let cfg = load_shipped_yaml("qwen");
        assert_eq!(cfg.name, "qwen");
    }

    #[test]
    fn providers_gemini_yaml_parses() {
        let cfg = load_shipped_yaml("gemini");
        assert_eq!(cfg.name, "gemini");
        // config_reverse_lookup must round-trip.
        match &cfg.cwd {
            CwdConfig::ConfigReverseLookup { container_path, .. } => {
                assert_eq!(container_path, "projects");
            }
            other => panic!("expected ConfigReverseLookup, got {other:?}"),
        }
    }
}
