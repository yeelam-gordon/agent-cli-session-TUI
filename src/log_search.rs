//! Full-text log search over session transcript tails, powered by tantivy.
//!
//! Design:
//! - Index lives at `{data_dir}/log_index/` (tantivy's own on-disk format).
//! - Fingerprint sidecar at `{data_dir}/log_index_fingerprints.json` maps
//!   `session_id` → combined (mtime+size) hash, so we skip re-indexing
//!   sessions whose log files haven't changed.
//! - The first and last 256 KB of each log file are indexed (head+tail). For ~700
//!   sessions this keeps the index well under 200 MB.
//! - The UI thread only calls `search()`; a background thread owns
//!   `refresh()`. Tantivy readers are `Clone` and see committed docs
//!   automatically.
//! - Sessions that disappear from the active+hidden set get deleted from
//!   the index (so archived-and-purged sessions don't match phantom text).
//!
//! We lean on tantivy for tokenization, inverted-index storage, BM25
//! scoring, and incremental commits — no custom index code.
//!
//! Licensed: tantivy is MIT (same as this crate).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tantivy::{
    collector::TopDocs,
    directory::MmapDirectory,
    query::QueryParser,
    schema::{Field, Schema, Value, STORED, STRING, TEXT},
    Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term,
};

use crate::models::{ActivitySource, Session};
use crate::provider::ProviderRegistry;

/// Per-source byte budget when reading head from a large log file.
///
/// For most session activity, the most recent content is in the tail and
/// the topic/intent is set in the head. We index a generous head slice so
/// queries like "iteration review" or "townhall question" — whose terms
/// usually appear in the first user message and a few follow-ups — match
/// even when the conversation has grown large.
const HEAD_BYTES: u64 = 1_500_000;

/// Per-source byte budget when reading tail from a large log file.
const TAIL_BYTES: u64 = 500_000;

/// Per-session whole-file ceiling. Files at or below this size are indexed
/// in full — no head/tail split, no structured-extract layer. Files above
/// this size fall back to head + tail + structured extraction (Copilot).
///
/// 2 MB covers 93% of typical sessions on a heavy user's machine, so the
/// vast majority get full-content indexing without any chunking heuristics.
const WHOLE_FILE_THRESHOLD: u64 = 2 * 1024 * 1024;

/// Writer heap budget. tantivy requires >= 15 MB.
const WRITER_HEAP_BYTES: usize = 20 * 1024 * 1024;

/// Max hits returned from a single `search()` call.
/// Enough to cover any session list we'd realistically render.
const MAX_HITS: usize = 2000;

/// How many changed sessions to index between commits + yields.
/// Small enough that results appear incrementally; large enough that
/// tantivy's per-commit overhead stays amortized.
const INDEX_CHUNK: usize = 25;

/// Sleep between chunks so first-time indexing doesn't saturate the
/// machine. Refresh runs on a background thread — this just yields
/// the scheduler so foreground work (UI, other CLIs) stays responsive.
const CHUNK_SLEEP: Duration = Duration::from_millis(25);

/// Sidecar fingerprint file, kept next to the tantivy index dir.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct FingerprintFile {
    version: u32,
    fingerprints: HashMap<String, u64>,
}

impl FingerprintFile {
    fn load(path: &Path) -> Self {
        let Ok(bytes) = fs::read(path) else { return Self::default() };
        serde_json::from_slice(&bytes).unwrap_or_default()
    }

    fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec(self)?;
        fs::write(&tmp, &bytes)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }
}

pub struct LogSearcher {
    index: Index,
    reader: IndexReader,
    writer: Mutex<IndexWriter>,
    session_id_field: Field,
    content_field: Field,
    fingerprints: Mutex<HashMap<String, u64>>,
    fingerprint_path: PathBuf,
}

impl LogSearcher {
    /// Open an existing index or create a fresh one under `data_dir/log_index`.
    pub fn open_or_create(data_dir: &Path) -> Result<Self> {
        let index_dir = data_dir.join("log_index");
        fs::create_dir_all(&index_dir).context("creating log_index dir")?;
        let fingerprint_path = data_dir.join("log_index_fingerprints.json");

        let mut schema_builder = Schema::builder();
        let session_id_field = schema_builder.add_text_field("session_id", STRING | STORED);
        let content_field = schema_builder.add_text_field("content", TEXT);
        let schema = schema_builder.build();

        let mmap_dir = MmapDirectory::open(&index_dir).context("opening mmap dir")?;
        let index = Index::open_or_create(mmap_dir, schema).context("opening index")?;
        let writer = index
            .writer(WRITER_HEAP_BYTES)
            .context("creating index writer")?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .context("creating index reader")?;

        let fps = FingerprintFile::load(&fingerprint_path);

        Ok(Self {
            index,
            reader,
            writer: Mutex::new(writer),
            session_id_field,
            content_field,
            fingerprints: Mutex::new(fps.fingerprints),
            fingerprint_path,
        })
    }

    /// Query the index and return `session_id` → BM25 score.
    /// Returns empty map on empty/invalid query — never panics.
    ///
    /// Search strategy: try strict AND first (high precision). If that
    /// returns nothing, retry with OR semantics (recall) so a single
    /// unknown term doesn't nuke the entire query. Without this, a query
    /// like "Iteration Review March Seana" (where "Seana" appears nowhere)
    /// returns zero log hits — even though three of the four words match
    /// strongly in a real session.
    pub fn search(&self, query_str: &str) -> HashMap<String, f32> {
        let trimmed = query_str.trim();
        if trimmed.is_empty() {
            return HashMap::new();
        }
        let searcher = self.reader.searcher();

        // Default to OR semantics: BM25 naturally ranks docs matching more
        // query terms higher (so docs with all N terms outrank docs with
        // N-1 terms by a wide margin), but unlike strict AND we don't
        // *exclude* otherwise-strong matches just because one rare term
        // (e.g. a misspelled name) doesn't appear anywhere in the corpus.
        let or_parser = QueryParser::for_index(&self.index, vec![self.content_field]);
        self.run_parsed_query(&searcher, &or_parser, trimmed)
    }

    /// Parse + execute a query with the given parser. Returns
    /// session_id → score, or empty map on parse/search error.
    fn run_parsed_query(
        &self,
        searcher: &tantivy::Searcher,
        query_parser: &QueryParser,
        trimmed: &str,
    ) -> HashMap<String, f32> {
        let query = match query_parser.parse_query(trimmed) {
            Ok(q) => q,
            Err(_) => {
                // Tantivy rejects certain punctuation ("foo:bar", stray
                // quotes, etc.) — escape and retry.
                let escaped = escape_query(trimmed);
                match query_parser.parse_query(&escaped) {
                    Ok(q) => q,
                    Err(_) => return HashMap::new(),
                }
            }
        };
        let top = match searcher.search(&query, &TopDocs::with_limit(MAX_HITS)) {
            Ok(t) => t,
            Err(_) => return HashMap::new(),
        };
        let mut out = HashMap::with_capacity(top.len());
        for (score, addr) in top {
            if let Ok(doc) = searcher.doc::<TantivyDocument>(addr) {
                if let Some(sid) = doc
                    .get_first(self.session_id_field)
                    .and_then(|v| v.as_str())
                {
                    out.insert(sid.to_string(), score);
                }
            }
        }
        out
    }

    /// Re-index new/changed sessions and evict sessions no longer present.
    /// Pass BOTH active + hidden sessions so archived sessions stay searchable
    /// in the Hidden view.
    pub fn refresh(&self, sessions: &[Session], registry: &ProviderRegistry) -> Result<()> {
        let mut writer = self.writer.lock().map_err(|_| anyhow::anyhow!("writer poisoned"))?;
        let mut fps = self
            .fingerprints
            .lock()
            .map_err(|_| anyhow::anyhow!("fingerprints poisoned"))?;

        let current_ids: HashSet<&str> = sessions.iter().map(|s| s.id.as_str()).collect();

        // Evict sessions that no longer exist (archived+purged, deleted from agent CLI, etc.)
        let stale_ids: Vec<String> = fps
            .keys()
            .filter(|id| !current_ids.contains(id.as_str()))
            .cloned()
            .collect();
        let had_stale = !stale_ids.is_empty();
        for id in &stale_ids {
            writer.delete_term(Term::from_field_text(self.session_id_field, id));
            fps.remove(id);
        }
        // Commit evictions immediately — they're cheap and keep phantom matches
        // from lingering while the (slower) newest-first reindex runs below.
        if had_stale {
            writer.commit().context("tantivy commit (evictions)")?;
        }

        // Index newest-first: a session touched last week is more likely to be
        // resumed than one from a year ago, so it should become searchable
        // sooner. We parse `updated_at` as RFC3339 and sort DESC; unparseable
        // rows sink to the end.
        let mut ordered: Vec<&Session> = sessions.iter().collect();
        ordered.sort_by(|a, b| {
            let ta = chrono::DateTime::parse_from_rfc3339(&a.updated_at).ok();
            let tb = chrono::DateTime::parse_from_rfc3339(&b.updated_at).ok();
            match (ta, tb) {
                (Some(a), Some(b)) => b.cmp(&a),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
        });

        // Index new / changed sessions, in chunks, yielding between chunks.
        let mut pending_in_chunk: usize = 0;
        for s in ordered {
            let Some(provider) = registry
                .providers()
                .iter()
                .find(|p| p.key() == s.provider_name)
            else {
                continue;
            };
            let sources = provider.activity_sources(s).unwrap_or_default();
            if sources.is_empty() {
                continue;
            }

            // Compute a cheap (mtime + size) fingerprint across all sources
            let mut fp: u64 = 0;
            let mut any_present = false;
            for src in &sources {
                let path = source_path(src);
                let Ok(meta) = fs::metadata(path) else { continue };
                any_present = true;
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                fp = fp.wrapping_add(mtime).wrapping_mul(1000003);
                fp = fp.wrapping_add(meta.len());
            }
            if !any_present {
                // All source files vanished — treat like deletion
                if fps.remove(&s.id).is_some() {
                    writer.delete_term(Term::from_field_text(self.session_id_field, &s.id));
                    pending_in_chunk += 1;
                }
                continue;
            }

            if fps.get(&s.id).copied() == Some(fp) {
                continue; // Unchanged
            }

            // Read tails and concatenate
            let mut combined = String::new();
            for src in &sources {
                if let Some(tail) = read_tail(source_path(src)) {
                    combined.push_str(&tail);
                    combined.push('\n');
                }
            }
            if combined.is_empty() {
                continue;
            }

            // Replace existing doc, if any
            writer.delete_term(Term::from_field_text(self.session_id_field, &s.id));
            let mut doc = TantivyDocument::default();
            doc.add_text(self.session_id_field, &s.id);
            doc.add_text(self.content_field, &combined);
            writer
                .add_document(doc)
                .context("adding doc to tantivy")?;
            fps.insert(s.id.clone(), fp);
            pending_in_chunk += 1;

            if pending_in_chunk >= INDEX_CHUNK {
                writer.commit().context("tantivy commit (chunk)")?;
                let snapshot = FingerprintFile {
                    version: 1,
                    fingerprints: fps.clone(),
                };
                if let Err(e) = snapshot.save(&self.fingerprint_path) {
                    crate::log::warn(&format!(
                        "log_search: fingerprint chunk save failed: {e}"
                    ));
                }
                pending_in_chunk = 0;
                // Yield so a cold-start reindex doesn't saturate the machine.
                std::thread::sleep(CHUNK_SLEEP);
            }
        }

        if pending_in_chunk > 0 {
            writer.commit().context("tantivy commit (final)")?;
            let snapshot = FingerprintFile {
                version: 1,
                fingerprints: fps.clone(),
            };
            drop(fps);
            if let Err(e) = snapshot.save(&self.fingerprint_path) {
                crate::log::warn(&format!(
                    "log_search: fingerprint final save failed: {e}"
                ));
            }
        }
        Ok(())
    }
}

fn source_path(src: &ActivitySource) -> &Path {
    match src {
        ActivitySource::EventStream(p)
        | ActivitySource::ProcessLog(p)
        | ActivitySource::LogFile(p) => p,
    }
}

/// Read the head (first 1.5 MB), tail (last 500 KB), AND all compaction
/// summaries from an events file. For files ≤ 2 MB the whole file is
/// read in a single pass — no head/tail split needed.
///
/// Compaction summaries (`session.compaction_complete` → `data.summaryContent`)
/// are the densest source of searchable context in long Copilot sessions —
/// ~10 KB each, containing structured overviews of everything discussed
/// before the compaction point.
fn read_tail(path: &Path) -> Option<String> {
    let mut f = fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();

    if len <= WHOLE_FILE_THRESHOLD {
        // Small/medium file — read the whole thing. This is the universal
        // win for sessions across all providers: anything ≤ 2 MB gets its
        // full content indexed, mid-conversation messages included.
        let mut buf = String::with_capacity(len as usize);
        f.read_to_string(&mut buf).ok()?;
        return Some(buf);
    }

    let mut buf = String::with_capacity((HEAD_BYTES + TAIL_BYTES + 1024) as usize);

    // Read head (first 1.5 MB — captures topic-setting first messages and
    // early conversation context, where queries by topic usually match).
    // Use a `take()` adapter + `read_to_end` to guarantee we get up to
    // HEAD_BYTES bytes — a single `read()` may return short on some
    // platforms even when more data is available.
    let mut head_bytes = Vec::with_capacity(HEAD_BYTES as usize);
    (&mut f).take(HEAD_BYTES).read_to_end(&mut head_bytes).ok()?;
    buf.push_str(&String::from_utf8_lossy(&head_bytes));
    buf.push_str("\n...\n");

    // Scan entire file for high-value structured events (Copilot-format only;
    // other providers' extractors fall through cleanly with no work done).
    drop(f);
    if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
        extract_structured_summaries(path, &mut buf);
    }

    // Read tail (last 500 KB — captures recent activity and most recent
    // task/turn state, for "what was I just working on" queries).
    let mut f = fs::File::open(path).ok()?;
    f.seek(SeekFrom::Start(len - TAIL_BYTES)).ok()?;
    buf.push_str("\n...\n");
    let mut tail = Vec::with_capacity(TAIL_BYTES as usize);
    f.read_to_end(&mut tail).ok()?;
    buf.push_str(&String::from_utf8_lossy(&tail));

    Some(buf)
}

/// Extract high-value structured text from JSONL events:
/// - `session.compaction_complete` → `data.summaryContent` (HIGHEST: ~10KB each, LLM-written overviews)
/// - `session.task_complete` → `data.summary` (HIGH: ~0.5KB each, concise task descriptions)
/// - `user.message` → `data.content` (HIGH: first 150 chars each — captures what the user asked about)
///
/// These are the densest sources of searchable context. Names, decisions, topics,
/// and technical details that would otherwise be lost in the middle of a large
/// events file are captured here.
fn extract_structured_summaries(path: &Path, buf: &mut String) {
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return,
    };
    use std::io::BufRead;
    let reader = std::io::BufReader::new(file);
    let mut total_added = 0usize;
    const MAX_EXTRACT_BYTES: usize = 2 * 1024 * 1024; // cap at 2MB total
    const USER_MSG_CAP: usize = 150; // first N chars of each user message

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        if total_added >= MAX_EXTRACT_BYTES {
            break;
        }

        // Quick pre-filter before parsing JSON
        if line.contains("session.compaction_complete") {
            if let Ok(obj) = serde_json::from_str::<serde_json::Value>(&line) {
                if obj.get("type").and_then(|v| v.as_str()) == Some("session.compaction_complete") {
                    if let Some(summary) = obj
                        .get("data")
                        .and_then(|d| d.get("summaryContent"))
                        .and_then(|v| v.as_str())
                    {
                        if !summary.is_empty() {
                            buf.push_str("\n[compaction]\n");
                            buf.push_str(summary);
                            buf.push('\n');
                            total_added += summary.len();
                        }
                    }
                }
            }
        } else if line.contains("session.task_complete") {
            if let Ok(obj) = serde_json::from_str::<serde_json::Value>(&line) {
                if obj.get("type").and_then(|v| v.as_str()) == Some("session.task_complete") {
                    if let Some(summary) = obj
                        .get("data")
                        .and_then(|d| d.get("summary"))
                        .and_then(|v| v.as_str())
                    {
                        if !summary.is_empty() {
                            buf.push_str("\n[task]\n");
                            buf.push_str(summary);
                            buf.push('\n');
                            total_added += summary.len();
                        }
                    }
                }
            }
        } else if line.contains("\"user.message\"") {
            if let Ok(obj) = serde_json::from_str::<serde_json::Value>(&line) {
                if obj.get("type").and_then(|v| v.as_str()) == Some("user.message") {
                    if let Some(content) = obj
                        .get("data")
                        .and_then(|d| d.get("content"))
                        .and_then(|v| v.as_str())
                    {
                        // Take first N chars — captures the user's question/topic
                        let trimmed = content.trim();
                        if !trimmed.is_empty() {
                            let cap = trimmed.char_indices()
                                .nth(USER_MSG_CAP)
                                .map(|(i, _)| i)
                                .unwrap_or(trimmed.len());
                            buf.push_str("\n[user]\n");
                            buf.push_str(&trimmed[..cap]);
                            buf.push('\n');
                            total_added += cap;
                        }
                    }
                }
            }
        }
    }
}

/// Escape tantivy query-parser-reserved characters so user input is always
/// treated as literal tokens. Called only as a fallback when the first parse
/// attempt fails (most queries pass through fine).
fn escape_query(q: &str) -> String {
    let mut out = String::with_capacity(q.len() * 2);
    for ch in q.chars() {
        if matches!(ch, '+' | '-' | '&' | '|' | '!' | '(' | ')' | '{' | '}' | '[' | ']' | '^' | '"' | '~' | '*' | '?' | ':' | '\\' | '/') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}
