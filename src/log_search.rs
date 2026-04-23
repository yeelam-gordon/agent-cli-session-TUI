//! Full-text log search over session transcript tails, powered by tantivy.
//!
//! Design:
//! - Index lives at `{data_dir}/log_index/` (tantivy's own on-disk format).
//! - Fingerprint sidecar at `{data_dir}/log_index_fingerprints.json` maps
//!   `session_id` → combined (mtime+size) hash, so we skip re-indexing
//!   sessions whose log files haven't changed.
//! - Only the last 256 KB of each log file is indexed (tails). For ~700
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

/// Bytes of each log file to index (from the tail).
const TAIL_BYTES: u64 = 256 * 1024;

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
    pub fn search(&self, query_str: &str) -> HashMap<String, f32> {
        let trimmed = query_str.trim();
        if trimmed.is_empty() {
            return HashMap::new();
        }
        let searcher = self.reader.searcher();
        let mut query_parser = QueryParser::for_index(&self.index, vec![self.content_field]);
        query_parser.set_conjunction_by_default();
        let query = match query_parser.parse_query(trimmed) {
            Ok(q) => q,
            Err(_) => {
                // Fall back to an escaped verbatim search — tantivy's query parser
                // rejects certain punctuation ("foo:bar", stray quotes, etc.).
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

fn read_tail(path: &Path) -> Option<String> {
    let mut f = fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    if len > TAIL_BYTES {
        f.seek(SeekFrom::Start(len - TAIL_BYTES)).ok()?;
    }
    let mut buf = Vec::with_capacity(TAIL_BYTES as usize);
    f.take(TAIL_BYTES).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
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
