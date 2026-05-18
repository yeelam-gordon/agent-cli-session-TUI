//! Tiered session search: exact → fuzzy → semantic (optional plugin).
//!
//! Ranking tiers:
//! 1. **Exact substring** match in title/summary/id → score 1000+
//! 2. **Fuzzy word** match (word-level containment) → score 500+
//! 3. **Semantic** similarity via optional DLL plugin → score 0-200 (boost only)
//!
//! The semantic tier is loaded at runtime from a shared library (`semantic_search.dll`
//! on Windows, `.so` on Linux, `.dylib` on macOS). If the library is not present,
//! search falls back gracefully to exact + fuzzy.

use crate::models::Session;

/// A scored search result — session index + relevance score.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub index: usize,
    pub score: u32,
    /// Whether this result got a semantic similarity boost.
    pub semantic_match: bool,
}

/// Rank sessions against a query. Returns indices sorted by relevance (highest first).
/// If a `SemanticPlugin` is provided and ready, semantic similarity boosts scores
/// using pre-computed cached embeddings (no embedding during search).
///
/// `log_matches` is an optional map of session.id → tantivy BM25 score from a
/// full-text search over each session's log tail. When provided, sessions that
/// matched the log index get a bonus score (below title/summary but above
/// provider name).
///
/// A recency multiplier is applied to every session's final score so that
/// "resume what I was working on last week" naturally outranks a year-old hit
/// of similar match quality.
pub fn ranked_search(
    sessions: &[Session],
    query: &str,
    semantic: Option<&SemanticPlugin>,
    log_matches: Option<&HashMap<String, f32>>,
) -> Vec<SearchResult> {
    if query.is_empty() {
        return (0..sessions.len())
            .map(|i| SearchResult { index: i, score: 0, semantic_match: false })
            .collect();
    }

    let query_lower = query.to_lowercase();
    let query_words: Vec<&str> = query_lower.split_whitespace().collect();

    // Tier 3: pre-compute semantic matches from cached embeddings.
    // Lowered threshold from 0.4 → 0.3: paraphrastic queries (user
    // remembers the MEETING context, not the literal words in the
    // session) often land in 0.3-0.4 similarity. The visible ✨ badge
    // is still gated above, so noisy low-similarity matches won't
    // pretend to be "smart" — they just contribute a small boost.
    let semantic_scores: HashMap<String, f32> = if query.len() >= 5 {
        semantic
            .filter(|s| s.is_ready())
            .map(|s| s.search_cached(query, 0.3).into_iter().collect())
            .unwrap_or_default()
    } else {
        HashMap::new()
    };

    let mut results: Vec<SearchResult> = sessions
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let log_score = log_matches.and_then(|m| m.get(&s.id)).copied();
            let mut score = score_session(s, &query_lower, &query_words, log_score);
            let mut semantic_match = false;

            // Tier 3: semantic boost from cached vectors (instant lookup).
            //
            // Boost formula deliberately generous: for long sessions where
            // the matching content is buried mid-transcript and never makes
            // it into the BM25 body index, semantic similarity is the
            // ONLY signal — fuzzy + semantic must carry recall on its own.
            // Floor 0.3, ceiling 800: sim 0.5 → +320, sim 0.6 → +480,
            // sim 0.7 → +640. Caps at 800 to keep it from drowning
            // legitimate title-exact matches (1000) but lets a strongly
            // semantic-relevant session compete with title/summary matches.
            if let Some(&sim) = semantic_scores.get(&s.id) {
                let boost = ((sim - 0.3) * 1600.0).clamp(0.0, 800.0) as u32;
                score = score.saturating_add(boost);
                semantic_match = true;
            }

            if score > 0 {
                // Recency bias — dampens older sessions so "what was I working
                // on lately" wins over a year-old hit of similar quality.
                let adjusted = (score as f32 * recency_multiplier(&s.updated_at)) as u32;
                Some(SearchResult { index: i, score: adjusted, semantic_match })
            } else {
                None
            }
        })
        .collect();

    // Sort by score descending (highest relevance first)
    results.sort_by_key(|r| std::cmp::Reverse(r.score));
    results
}

/// Detailed score breakdown for a single session — used by `--search-bench`
/// to explain WHY a session ranked where it did. Mirrors the logic in
/// `score_session` but records each tier's contribution rather than just
/// keeping the max.
///
/// Fields are the per-tier scores BEFORE the recency multiplier; `recency`
/// holds the multiplier so the caller can present `final = best * recency`.
#[derive(Debug, Clone, Default)]
pub struct ScoreBreakdown {
    /// Per-field tier scores. Field 0 = title, then session_id, summary,
    /// cwd, provider_name (matches the array in `score_session`).
    pub field_scores: [FieldTierScore; 5],
    /// BM25 bonus from tantivy log index (post-clamp).
    pub bm25_bonus: u32,
    /// Raw BM25 score from tantivy (pre-clamp / pre-multiplier).
    pub bm25_raw: f32,
    /// Bonus from matching session state label ("running", etc.).
    pub state_label_bonus: u32,
    /// Cosine similarity from semantic plugin (0.0 if no match).
    pub semantic_sim: f32,
    /// Computed semantic boost added to score.
    pub semantic_boost: u32,
    /// Total best score BEFORE recency multiplier.
    pub best_pre_recency: u32,
    /// Recency multiplier applied to the final score.
    pub recency: f32,
    /// Final score AFTER recency multiplier.
    pub final_score: u32,
}

#[derive(Debug, Clone, Default)]
pub struct FieldTierScore {
    pub label: &'static str,
    pub base: u32,
    /// Tier 1 exact-substring score (0 if not matched).
    pub exact: u32,
    /// Tier 2 all-words-in-field score (0 if not matched).
    pub all_words: u32,
    /// Tier 2b partial-words score (0 if not matched or gated out).
    pub partial: u32,
    /// Distinct query words that hit this field.
    pub word_hits: u32,
}

/// Compute the same score as `score_session` but return a full per-tier
/// breakdown for debugging / benchmarking.
pub fn score_breakdown(
    session: &Session,
    query: &str,
    query_words: &[&str],
    log_score: Option<f32>,
    semantic_sim: f32,
) -> ScoreBreakdown {
    let cwd_string = session.cwd.to_string_lossy().to_string();
    let fields: [(&str, &str, u32); 5] = [
        ("title", session.title.as_str(), 1000),
        ("session_id", session.provider_session_id.as_str(), 800),
        ("summary", session.summary.as_str(), 600),
        ("cwd", cwd_string.as_str(), 400),
        ("provider", session.provider_name.as_str(), 300),
    ];

    let mut concat_lower = String::new();
    for (_, f, _) in &fields {
        concat_lower.push(' ');
        concat_lower.push_str(&f.to_lowercase());
    }
    let total_distinct_hits = query_words
        .iter()
        .filter(|w| w.len() >= 3 && concat_lower.contains(*w))
        .count() as u32;
    let tier2b_eligible = if query_words.len() == 1 {
        true
    } else {
        total_distinct_hits >= 2
    };

    let mut bd = ScoreBreakdown::default();
    let mut best_score = 0u32;

    for (i, (label, field, base)) in fields.iter().enumerate() {
        let field_lower = field.to_lowercase();
        let mut fs = FieldTierScore {
            label,
            base: *base,
            exact: 0,
            all_words: 0,
            partial: 0,
            word_hits: 0,
        };

        if field_lower.contains(query) {
            fs.exact = *base;
            best_score = best_score.max(*base);
        } else if query_words.len() > 1
            && query_words.iter().all(|w| field_lower.contains(w))
        {
            fs.all_words = base / 2;
            best_score = best_score.max(base / 2);
        } else if tier2b_eligible {
            let word_hits: u32 = query_words
                .iter()
                .filter(|w| w.len() >= 3 && field_lower.contains(*w))
                .count() as u32;
            fs.word_hits = word_hits;
            if word_hits > 0 {
                let partial_score = base / 4 + word_hits * 50;
                fs.partial = partial_score;
                best_score = best_score.max(partial_score);
            }
        }
        bd.field_scores[i] = fs;
    }

    if let Some(bm25) = log_score {
        bd.bm25_raw = bm25;
        let bonus = (bm25 * 80.0).clamp(50.0, 1200.0) as u32;
        bd.bm25_bonus = bonus;
        best_score = best_score.max(bonus);
    }

    let label_lower = session.state.label().to_lowercase();
    if label_lower.contains(query) || query_words.iter().any(|w| label_lower.contains(w)) {
        bd.state_label_bonus = 200;
        best_score = best_score.max(200);
    }

    bd.semantic_sim = semantic_sim;
    if semantic_sim >= 0.3 {
        let boost = ((semantic_sim - 0.3) * 1600.0).clamp(0.0, 800.0) as u32;
        bd.semantic_boost = boost;
        best_score = best_score.saturating_add(boost);
    }

    bd.best_pre_recency = best_score;
    bd.recency = recency_multiplier(&session.updated_at);
    bd.final_score = (best_score as f32 * bd.recency) as u32;
    bd
}

/// Multiplier applied to the final score based on how old the session is.
/// Uses an exponential decay with a 90-day half-life, floored at 0.2 so
/// ancient sessions can still surface when they're a uniquely strong match.
///
/// Examples (days_old → multiplier):
///   0   → 1.00 (today)
///   7   → 0.85 (last week)
///   30  → 0.67 (last month)
///   90  → 0.60 (halflife point, ~0.60 due to floor)
///   180 → 0.40
///   365 → 0.23 (year old, near floor)
fn recency_multiplier(updated_at: &str) -> f32 {
    let parsed = chrono::DateTime::parse_from_rfc3339(updated_at)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc));
    let Some(updated) = parsed else { return 1.0 };
    let now = chrono::Utc::now();
    let delta = now.signed_duration_since(updated);
    let days = (delta.num_seconds() as f32 / 86_400.0).max(0.0);
    let decay = 0.5_f32.powf(days / 90.0);
    (0.2 + 0.8 * decay).clamp(0.2, 1.0)
}

/// Score a single session against a query. `log_score` is the tantivy BM25
/// score from the full-text log index, if the session matched.
fn score_session(
    session: &Session,
    query: &str,
    query_words: &[&str],
    log_score: Option<f32>,
) -> u32 {
    let fields = [
        (&session.title, 1000u32),       // title exact match = highest
        (&session.provider_session_id, 800),
        (&session.summary, 600),
        (&session.cwd.to_string_lossy().to_string(), 400),
        (&session.provider_name, 300),
    ];

    // Pre-compute distinct query-word hits ACROSS all fields combined.
    // Used by tier 2b to gate single-common-word noise: a multi-word
    // query whose only matching word is one common term (e.g., "march")
    // shouldn't surface unrelated sessions. We require ≥2 distinct hits
    // ANYWHERE in the session before partial-match scores fire.
    let mut total_distinct_hits: u32 = 0;
    if query_words.len() > 1 {
        let mut concat_lower = String::new();
        for (f, _) in &fields {
            concat_lower.push(' ');
            concat_lower.push_str(&f.to_lowercase());
        }
        total_distinct_hits = query_words
            .iter()
            .filter(|w| w.len() >= 3 && concat_lower.contains(*w))
            .count() as u32;
    }
    let tier2b_eligible = if query_words.len() == 1 {
        true
    } else {
        total_distinct_hits >= 2
    };

    let mut best_score = 0u32;

    for (field, base_score) in &fields {
        let field_lower = field.to_lowercase();

        // Tier 1: exact substring match
        if field_lower.contains(query) {
            best_score = best_score.max(*base_score);
            continue;
        }

        // Tier 2: all query words appear in the field (word-level fuzzy)
        if query_words.len() > 1 {
            let all_words_match = query_words.iter().all(|w| field_lower.contains(w));
            if all_words_match {
                best_score = best_score.max(base_score / 2);
                continue;
            }
        }

        // Tier 2b: partial word match — gated by tier2b_eligible above so
        // multi-word queries with only ONE distinct word matching anywhere
        // in the session no longer score (kills "search 'iteration review
        // march seana' matches every session containing 'march' in title").
        if !tier2b_eligible {
            continue;
        }
        let word_hits: u32 = query_words
            .iter()
            .filter(|w| w.len() >= 3 && field_lower.contains(*w))
            .count() as u32;
        if word_hits > 0 {
            let partial_score = base_score / 4 + word_hits * 50;
            best_score = best_score.max(partial_score);
        }
    }

    // Tier 1c: log/transcript content — BM25 score from tantivy. Typical
    // BM25 values are 0-10+ for single hits; multi-word OR queries can
    // produce 20-40+. Ceiling raised to 1200 (above title-exact 1000) so
    // that strong multi-term BODY matches can dominate when the title
    // doesn't mention the search terms — this is the recall case where
    // a user remembers what the session was ABOUT, not what it was NAMED.
    if let Some(bm25) = log_score {
        let bonus = (bm25 * 80.0).clamp(50.0, 1200.0) as u32;
        best_score = best_score.max(bonus);
    }

    // Tier 2c: check state label as a search term (e.g., "running", "waiting")
    let label = session.state.label().to_lowercase();
    if label.contains(query) || query_words.iter().any(|w| label.contains(w)) {
        best_score = best_score.max(200);
    }

    best_score
}

// ---------------------------------------------------------------------------
// Semantic search plugin with embedding cache
// ---------------------------------------------------------------------------

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Status of the semantic search plugin.
#[derive(Debug, Clone, PartialEq)]
pub enum SemanticStatus {
    /// Not available (DLL not found).
    Unavailable,
    /// DLL loaded, indexing session embeddings.
    Indexing { done: usize, total: usize },
    /// Embeddings computed and searchable.
    Ready { count: usize },
    /// Failed to load.
    Failed(String),
}

/// Cached embedding entry: text hash + vector.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CachedEmbedding {
    text_hash: u64,
    vector: Vec<f32>,
}

/// Persistent embedding cache — JSON file on disk.
/// Each session can have multiple embedding chunks (title, compaction summaries,
/// task completions, user messages) so semantic search matches against any aspect.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct EmbeddingCache {
    /// session_id → list of embedding chunks
    entries: HashMap<String, Vec<CachedEmbedding>>,
}

impl EmbeddingCache {
    fn load(path: &std::path::Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save(&self, path: &std::path::Path) {
        let json = match serde_json::to_string(self) {
            Ok(j) => j,
            Err(e) => {
                crate::log::warn(&format!("EmbeddingCache::save serialize failed: {e}"));
                return;
            }
        };
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                crate::log::warn(&format!(
                    "EmbeddingCache::save mkdir {parent:?} failed: {e}"
                ));
                return;
            }
        }
        // Atomic write: tmp + rename. Without this, a crash or
        // AV/OneDrive interference mid-write leaves a truncated JSON
        // file, and next startup silently discards every cached
        // embedding and recomputes them all.
        let tmp = path.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp, json) {
            crate::log::warn(&format!(
                "EmbeddingCache::save write {tmp:?} failed: {e}"
            ));
            return;
        }
        if let Err(e) = std::fs::rename(&tmp, path) {
            crate::log::warn(&format!(
                "EmbeddingCache::save rename {tmp:?} -> {path:?} failed: {e}"
            ));
        }
    }
}

pub fn hash_text(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Semantic search plugin — loads a shared library at runtime.
pub struct SemanticPlugin {
    status: SemanticStatus,
    /// A separately-lockable copy of status so the UI can poll progress
    /// without contending on the main plugin mutex (which the indexer
    /// holds for seconds at a time during each embed call).
    shared_status: Arc<Mutex<SemanticStatus>>,
    pub(crate) lib: Option<libloading::Library>,
    dim: i32,
    cache: EmbeddingCache,
    cache_path: Option<std::path::PathBuf>,
    /// True once `semantic_init` has populated the in-DLL model.
    /// Set false by `unload()`, set true again by `ensure_loaded()`.
    model_loaded: bool,
    /// Cache directory we initialized with — needed by ensure_loaded to
    /// re-init the FFI model after an unload.
    cache_dir: Option<String>,
}

impl Default for SemanticPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl SemanticPlugin {
    pub fn new() -> Self {
        Self {
            status: SemanticStatus::Unavailable,
            shared_status: Arc::new(Mutex::new(SemanticStatus::Unavailable)),
            lib: None,
            dim: 0,
            cache: EmbeddingCache::default(),
            cache_path: None,
            model_loaded: false,
            cache_dir: None,
        }
    }

    pub fn status(&self) -> &SemanticStatus {
        &self.status
    }

    /// Return a cloneable handle to the shared-status mutex. Callers (the
    /// UI renderer) should hold an `Arc<Mutex<SemanticStatus>>` and poll
    /// it with `try_lock` — this lock is only held for nanoseconds during
    /// writes, so the UI always sees fresh progress even while the
    /// indexer thread is mid-embed holding the plugin mutex.
    pub fn shared_status(&self) -> Arc<Mutex<SemanticStatus>> {
        self.shared_status.clone()
    }

    fn set_status(&mut self, s: SemanticStatus) {
        self.status = s.clone();
        if let Ok(mut g) = self.shared_status.lock() {
            *g = s;
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(self.status, SemanticStatus::Ready { .. })
    }

    // --- Incremental indexing helpers (used by the external indexer loop
    // so it can release the plugin mutex between sessions).

    /// Does this session still need an embedding for the given text hash?
    pub fn needs_embedding(&self, session_id: &str, text_hash: u64) -> bool {
        self.lib.is_some()
            && self.dim > 0
            && self
                .cache
                .entries
                .get(session_id)
                .map(|chunks| {
                    // Check if the first chunk's hash matches (base text identity)
                    chunks.first().map(|c| c.text_hash != text_hash).unwrap_or(true)
                })
                .unwrap_or(true)
    }

    /// Embed one text and cache the result under `session_id`. Returns
    /// true if a new embedding was produced and stored. This replaces all
    /// existing chunks for the session. Kept as library-public surface
    /// alongside `embed_and_cache_multi`; the multi variant is the only
    /// in-tree caller today.
    #[allow(dead_code)]
    pub fn embed_and_cache(&mut self, session_id: &str, text: &str, text_hash: u64) -> bool {
        if let Some(vec) = self.embed(text) {
            self.cache.entries.insert(
                session_id.to_string(),
                vec![CachedEmbedding {
                    text_hash,
                    vector: vec,
                }],
            );
            true
        } else {
            false
        }
    }

    /// Embed multiple text chunks for a session. Each chunk gets its own vector.
    /// The first chunk's text_hash is used as the identity hash for change detection.
    /// Returns the number of chunks successfully embedded.
    pub fn embed_and_cache_multi(&mut self, session_id: &str, chunks: &[(String, u64)]) -> usize {
        let mut cached = Vec::new();
        for (text, hash) in chunks {
            if let Some(vec) = self.embed(text) {
                cached.push(CachedEmbedding {
                    text_hash: *hash,
                    vector: vec,
                });
            }
        }
        let count = cached.len();
        if !cached.is_empty() {
            self.cache.entries.insert(session_id.to_string(), cached);
        }
        count
    }

    /// Update indexing progress (visible to the UI via `shared_status`).
    pub fn update_progress(&mut self, done: usize, total: usize) {
        self.set_status(SemanticStatus::Indexing { done, total });
    }

    /// Mark the plugin Ready with the current cached embedding count.
    pub fn mark_ready(&mut self) {
        let count = self.cache.entries.values().map(|v| v.len()).sum::<usize>();
        self.set_status(SemanticStatus::Ready { count });
    }

    /// Flush the embedding cache to disk (if a cache path is configured).
    pub fn save_cache(&self) {
        if let Some(ref path) = self.cache_path {
            self.cache.save(path);
        }
    }

    /// True if the embedding model is currently loaded in memory.
    /// `lib` stays loaded across unload/reload cycles; only the model
    /// weights + ONNX runtime arenas are freed.
    pub fn is_loaded(&self) -> bool {
        self.model_loaded && self.lib.is_some()
    }

    /// Count how many sessions would actually need a new embedding.
    /// Used by the UI to skip spawning the indexer thread entirely when
    /// everything is already up-to-date.
    pub fn count_needing_embedding<F>(&self, sessions: &[Session], text_fn: F) -> usize
    where
        F: Fn(&Session) -> String,
    {
        if self.lib.is_none() || self.dim <= 0 {
            return 0;
        }
        sessions
            .iter()
            .filter(|s| {
                let t = text_fn(s);
                let h = hash_text(&t);
                self.needs_embedding(&s.id, h)
            })
            .count()
    }

    /// Unload the embedding model to free ~550MB of weights + ONNX state.
    /// `lib` stays loaded so we can call `semantic_init` again cheaply.
    /// Called after indexing completes to keep idle memory low.
    pub fn unload(&mut self) {
        if let Some(ref lib) = self.lib {
            unsafe {
                if let Ok(unload_fn) = lib
                    .get::<libloading::Symbol<unsafe extern "C" fn() -> i32>>(b"semantic_unload")
                {
                    let _ = unload_fn();
                }
            }
        }
        self.model_loaded = false;
        crate::log::info("Semantic: model unloaded (idle memory freed)");
    }

    /// Ensure the model is loaded before calling `embed`. Blocks for 1–2s
    /// if a reload is needed. No-op if already loaded.
    pub fn ensure_loaded(&mut self, cache_dir: &str) -> bool {
        if self.is_loaded() {
            return true;
        }
        let lib = match self.lib.as_ref() {
            Some(l) => l,
            None => return false,
        };
        let result: i32 = unsafe {
            let init: libloading::Symbol<
                unsafe extern "C" fn(*const std::ffi::c_char) -> i32,
            > = match lib.get(b"semantic_init") {
                Ok(f) => f,
                Err(_) => return false,
            };
            let c_dir = std::ffi::CString::new(cache_dir).unwrap_or_default();
            init(c_dir.as_ptr())
        };
        if result == 0 {
            self.model_loaded = true;
            crate::log::info("Semantic: model reloaded for query");
            true
        } else {
            false
        }
    }

    /// Path of the model cache dir (needed by `ensure_loaded`).
    pub fn cache_dir(&self) -> Option<&str> {
        self.cache_dir.as_deref()
    }

    /// Try to load the semantic search DLL from next to the executable.
    /// `cache_dir` is where the model files will be downloaded/cached.
    pub fn try_load(&mut self, cache_dir: &str) {
        let dll_name = if cfg!(windows) {
            "semantic_search_plugin.dll"
        } else if cfg!(target_os = "macos") {
            "libsemantic_search_plugin.dylib"
        } else {
            "libsemantic_search_plugin.so"
        };

        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()));

        let dll_path = match exe_dir {
            Some(dir) => dir.join(dll_name),
            None => return,
        };

        if !dll_path.exists() {
            crate::log::info(&format!("Semantic plugin not found at {:?}", dll_path));
            return;
        }

        crate::log::info(&format!("Loading semantic plugin: {:?}", dll_path));
        self.set_status(SemanticStatus::Indexing { done: 0, total: 0 });

        let lib = match unsafe { libloading::Library::new(&dll_path) } {
            Ok(l) => l,
            Err(e) => {
                let msg = format!("Failed to load DLL: {}", e);
                crate::log::error(&msg);
                self.set_status(SemanticStatus::Failed(msg));
                return;
            }
        };

        // Call semantic_init with cache directory
        let init_result: i32 = unsafe {
            let init: libloading::Symbol<unsafe extern "C" fn(*const std::ffi::c_char) -> i32> =
                match lib.get(b"semantic_init") {
                    Ok(f) => f,
                    Err(e) => {
                        let msg = format!("Missing semantic_init: {}", e);
                        crate::log::error(&msg);
                        self.set_status(SemanticStatus::Failed(msg));
                        return;
                    }
                };
            let c_dir = std::ffi::CString::new(cache_dir).unwrap_or_default();
            init(c_dir.as_ptr())
        };

        if init_result != 0 {
            let msg = "semantic_init returned error".to_string();
            crate::log::error(&msg);
            self.set_status(SemanticStatus::Failed(msg));
            return;
        }

        // Get embedding dimension
        let dim: i32 = unsafe {
            let dim_fn: libloading::Symbol<unsafe extern "C" fn() -> i32> =
                match lib.get(b"semantic_dim") {
                    Ok(f) => f,
                    Err(_) => {
                        self.set_status(SemanticStatus::Failed("Missing semantic_dim".into()));
                        return;
                    }
                };
            dim_fn()
        };

        if dim <= 0 {
            self.set_status(SemanticStatus::Failed("Invalid embedding dimension".into()));
            return;
        }

        self.dim = dim;
        self.lib = Some(lib);
        self.model_loaded = true;
        self.cache_dir = Some(cache_dir.to_string());

        // Load embedding cache from disk.
        // `_v3` suffix: v2 was 384-dim MiniLM with 1KB text; v3 is 768-dim Nomic
        // Embed v1.5 with 8K context and `search_document:` / `search_query:`
        // instruction prefixes. Vectors are incompatible — old cache discarded.
        let cache_file = std::path::PathBuf::from(cache_dir).join("embeddings_cache_v3.json");
        self.cache = EmbeddingCache::load(&cache_file);
        self.cache_path = Some(cache_file);

        let cached_count = self.cache.entries.len();
        let initial = if cached_count > 0 {
            SemanticStatus::Ready { count: cached_count }
        } else {
            SemanticStatus::Indexing { done: 0, total: 0 }
        };
        self.set_status(initial);
        crate::log::info(&format!(
            "Semantic plugin loaded (dim={}, cached={})",
            dim, cached_count
        ));
    }

    /// Index sessions: compute embeddings for new/changed sessions.
    ///
    /// `text_fn` returns the text to embed for each session. The caller is
    /// responsible for shaping this text (title, summary, log head/tail, cwd,
    /// etc.) within the embedding model's token window (~256 tokens / ~1 KB
    /// for all-MiniLM-L6-v2).
    ///
    /// Only embeds sessions whose text hash changed. Saves cache to disk.
    /// Returns (newly_embedded, total_cached).
    #[allow(dead_code)]
    pub fn index_sessions<F>(&mut self, sessions: &[Session], text_fn: F) -> (usize, usize)
    where
        F: Fn(&Session) -> String,
    {
        if self.lib.is_none() || self.dim <= 0 {
            return (0, 0);
        }

        let total = sessions.len();
        let mut newly_embedded = 0usize;

        for (i, session) in sessions.iter().enumerate() {
            let text = text_fn(session);
            let text_hash = hash_text(&text);

            // Skip if already cached with same hash
            if let Some(cached) = self.cache.entries.get(&session.id) {
                if cached.first().map(|c| c.text_hash == text_hash).unwrap_or(false) {
                    continue;
                }
            }

            // Embed this session
            if let Some(vec) = self.embed(&text) {
                self.cache.entries.insert(
                    session.id.clone(),
                    vec![CachedEmbedding {
                        text_hash,
                        vector: vec,
                    }],
                );
                newly_embedded += 1;

                // Update status periodically
                if newly_embedded.is_multiple_of(10) {
                    self.status = SemanticStatus::Indexing {
                        done: i + 1,
                        total,
                    };
                }
            }
        }

        let count = self.cache.entries.values().map(|v| v.len()).sum::<usize>();
        self.status = SemanticStatus::Ready { count };

        // Flush cache to disk (async-safe: write then rename would be better, but this works)
        if newly_embedded > 0 {
            if let Some(ref path) = self.cache_path {
                self.cache.save(path);
                crate::log::info(&format!(
                    "Semantic index: {} new embeddings, {} total cached",
                    newly_embedded, count
                ));
            }
        }

        (newly_embedded, count)
    }

    /// Search cached embeddings for sessions similar to the query.
    /// Returns (session_id, cosine_similarity) pairs above the threshold.
    ///
    /// Prefixes the query with `search_query:` as required by Nomic Embed v1.5
    /// for asymmetric retrieval (documents were embedded with `search_document:`).
    pub fn search_cached(&self, query: &str, threshold: f32) -> Vec<(String, f32)> {
        let prefixed = format!("search_query: {}", query);
        let query_vec = match self.embed(&prefixed) {
            Some(v) => v,
            None => return vec![],
        };

        let mut results: Vec<(String, f32)> = self
            .cache
            .entries
            .iter()
            .filter_map(|(id, chunks)| {
                // Max similarity across all chunks for this session
                let max_sim = chunks.iter()
                    .map(|c| cosine_similarity(&query_vec, &c.vector))
                    .fold(0.0f32, f32::max);
                if max_sim > threshold {
                    Some((id.clone(), max_sim))
                } else {
                    None
                }
            })
            .collect();

        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results
    }

    /// Embed a single text string. Returns None if plugin not ready.
    pub fn embed(&self, text: &str) -> Option<Vec<f32>> {
        let lib = self.lib.as_ref()?;
        if self.dim <= 0 {
            return None;
        }

        let mut out = vec![0.0f32; self.dim as usize];
        let c_text = std::ffi::CString::new(text).ok()?;

        let result: i32 = unsafe {
            let embed_fn: libloading::Symbol<
                unsafe extern "C" fn(*const std::ffi::c_char, *mut f32, i32) -> i32,
            > = lib.get(b"semantic_embed").ok()?;
            embed_fn(c_text.as_ptr(), out.as_mut_ptr(), self.dim)
        };

        if result > 0 {
            out.truncate(result as usize);
            Some(out)
        } else {
            None
        }
    }

    /// Compute cosine similarity between two embedding vectors via DLL.
    #[allow(dead_code)]
    pub fn cosine(&self, a: &[f32], b: &[f32]) -> Option<f32> {
        let lib = self.lib.as_ref()?;
        if a.len() != b.len() || a.is_empty() {
            return None;
        }

        let sim: f32 = unsafe {
            let cosine_fn: libloading::Symbol<
                unsafe extern "C" fn(*const f32, *const f32, i32) -> f32,
            > = lib.get(b"semantic_cosine").ok()?;
            cosine_fn(a.as_ptr(), b.as_ptr(), a.len() as i32)
        };

        if sim <= -2.0 { None } else { Some(sim) }
    }

    /// Compute pairwise cosine similarities between cached session embeddings.
    /// Returns pairs `(session_a, session_b, similarity)` where similarity > threshold.
    /// Uses only cached vectors — does not load the model.
    pub fn pairwise_similarities(
        &self,
        session_keys: &[String],
        threshold: f32,
    ) -> Vec<(String, String, f32)> {
        let vecs: Vec<(&String, &Vec<f32>)> = session_keys
            .iter()
            .filter_map(|k| {
                self.cache.entries.get(k)
                    .and_then(|chunks| chunks.first())
                    .map(|c| (k, &c.vector))
            })
            .collect();

        let mut result = Vec::new();
        for i in 0..vecs.len() {
            for j in (i + 1)..vecs.len() {
                let sim = cosine_similarity(vecs[i].1, vecs[j].1);
                if sim > threshold {
                    result.push((vecs[i].0.clone(), vecs[j].0.clone(), sim));
                }
            }
        }
        result.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        result
    }
}

/// Pure-Rust cosine similarity (no DLL needed — for cached vector search).
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a < 1e-10 || norm_b < 1e-10 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

#[allow(dead_code)]
impl SearchResult {
    pub fn new(index: usize, score: u32) -> Self {
        Self { index, score, semantic_match: false }
    }
}

// ──────────────────────────────────────────────────────────────────────────
//  Reciprocal Rank Fusion (RRF) — experimental hybrid scoring on this branch
// ──────────────────────────────────────────────────────────────────────────
//
// Rationale: the additive `score_breakdown` above suffers from score-scale
// incompatibility — title-tier returns 0–1000, BM25 0–25 (clamped 50–1200),
// semantic 0–1 (scaled to 0–800). The downstream max/saturating_add ops
// produce a single number, but the magnitudes never balance cleanly: a tiny
// fuzzy or semantic signal is either crushed to the BM25 floor or drowned
// by a title-tier mismatch.
//
// RRF sidesteps this: each layer (lexical, BM25, semantic) ranks its own
// candidates by raw score, and final ranking comes from the WEIGHTED SUM
// of reciprocal ranks across layers. Score magnitudes never enter the
// fusion. A doc that ranks #1 in any layer is guaranteed to be a strong
// candidate; a doc that ranks high in multiple layers wins decisively.
//
// Following critique pre-implementation:
//   - Only docs with positive signal enter a layer (no zero-score ranks).
//   - Title layer is WEIGHTED 2× (the strongest single-layer winner here).
//   - Recency softened: `rrf * (0.7 + 0.3 * recency)` — not pure multiplication.
//   - State-label match is a small additive tie-breaker, not a 4th layer.
//   - Fuzzy/typo layer is deliberately NOT included in this first pass:
//     measure the pure-RRF result first, add fuzzy as L4 only if needed.

/// Per-layer rank + score for a single session under RRF scoring. Used by
/// the `--search-bench` / `--search-eval` diagnostic paths to explain why
/// a session ranked where it did.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)] // Fields read by future --search-bench --rrf diagnostic output.
pub struct RrfScoreBreakdown {
    /// Discretized final score (RRF × softened recency × 1e6, plus tie
    /// breakers). Sort by this descending to get the ranking.
    pub final_score: u32,
    /// Raw RRF score (sum of weighted reciprocal ranks).
    pub rrf_raw: f32,
    /// Standard recency multiplier (same function as additive scoring).
    pub recency: f32,
    /// Rank in the lexical layer (None if no positive lexical match).
    pub title_rank: Option<usize>,
    /// Rank in the BM25 log-index layer (None if no BM25 hit).
    pub bm25_rank: Option<usize>,
    /// Rank in the semantic layer (None if sim < 0.3).
    pub semantic_rank: Option<usize>,
    /// Best per-field tier score (the existing 0–1000 magnitude) for diag.
    pub title_score: u32,
    /// Raw BM25 score from tantivy.
    pub bm25_raw: f32,
    /// Raw cosine similarity from the semantic plugin.
    pub semantic_sim: f32,
}

/// RRF tuning constants. Documented as constants (not magic numbers in code)
/// so each adjustment shows up in diffs and is bench-comparable.
// Tuning constants — values established by `--search-eval` against the
// real query set. Trials documented in eval/runs/rrf-v{1..4}.json:
//   v1 (k=60, title=2.0): MRR 0.775, P@1 66.7%   ← CURRENT / BEST
//   v2 (k=60, title=3.0): MRR 0.741, P@1 60.0%   ← over-weights title
//   v3 (k=10, title=2.0): MRR 0.750, P@1 63.3%   ← top-rank too dominant
//   v4 (k=60, title=1.5): MRR 0.697, P@1 53.3%   ← under-weights title
const RRF_K: f32 = 60.0;
const RRF_W_TITLE: f32 = 2.0;
const RRF_W_BM25: f32 = 1.0;
const RRF_W_SEM: f32 = 1.0;
const RRF_SEM_THRESHOLD: f32 = 0.3;
const RRF_STATE_TIE: u32 = 100;

/// Compute only the best lexical tier score for a session (same formula as
/// the field-tier portion of `score_breakdown`, ignoring BM25 / semantic /
/// state / recency). Used to rank sessions in the RRF lexical layer.
fn best_lexical_tier_score(session: &Session, query: &str, query_words: &[&str]) -> u32 {
    let cwd_string = session.cwd.to_string_lossy().to_string();
    let fields: [(&str, u32); 5] = [
        (session.title.as_str(), 1000),
        (session.provider_session_id.as_str(), 800),
        (session.summary.as_str(), 600),
        (cwd_string.as_str(), 400),
        (session.provider_name.as_str(), 300),
    ];

    let mut concat_lower = String::new();
    for (f, _) in &fields {
        concat_lower.push(' ');
        concat_lower.push_str(&f.to_lowercase());
    }
    let total_distinct_hits = query_words
        .iter()
        .filter(|w| w.len() >= 3 && concat_lower.contains(*w))
        .count() as u32;
    let tier2b_eligible = if query_words.len() == 1 {
        true
    } else {
        total_distinct_hits >= 2
    };

    let mut best: u32 = 0;
    for (field, base) in fields.iter() {
        let field_lower = field.to_lowercase();
        if field_lower.contains(query) {
            best = best.max(*base);
        } else if query_words.len() > 1 && query_words.iter().all(|w| field_lower.contains(w)) {
            best = best.max(base / 2);
        } else if tier2b_eligible {
            let word_hits: u32 = query_words
                .iter()
                .filter(|w| w.len() >= 3 && field_lower.contains(*w))
                .count() as u32;
            if word_hits > 0 {
                best = best.max(base / 4 + word_hits * 50);
            }
        }
    }
    best
}

/// Rank sessions via RRF over (lexical, BM25, semantic) layers.
///
/// Returns sessions in descending order of RRF-final-score, paired with a
/// breakdown for diagnostic output. Sessions with zero signal in every
/// layer are excluded entirely (they cannot rank — they have no anchor).
pub fn ranked_search_rrf(
    sessions: &[Session],
    query: &str,
    log_matches: &std::collections::HashMap<String, f32>,
    sem_scores: &std::collections::HashMap<String, f32>,
) -> Vec<(usize, RrfScoreBreakdown)> {
    if query.is_empty() || sessions.is_empty() {
        return Vec::new();
    }
    let query_lower = query.to_lowercase();
    let query_words: Vec<&str> = query_lower.split_whitespace().collect();

    // Build per-layer scored lists. Each entry: (session_index, raw_score).
    // Only docs with a real positive signal enter a layer — that prevents
    // RRF from credit-assigning "rank 700 of 700" to docs that don't even
    // have a hit, which would otherwise let recency / tie-breakers
    // dominate irrelevant results.

    let mut lex: Vec<(usize, u32)> = sessions
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let score = best_lexical_tier_score(s, &query_lower, &query_words);
            if score > 0 { Some((i, score)) } else { None }
        })
        .collect();
    lex.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let mut bm25: Vec<(usize, f32)> = sessions
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            log_matches
                .get(&s.id)
                .copied()
                .filter(|v| *v > 0.0)
                .map(|v| (i, v))
        })
        .collect();
    bm25.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut sem: Vec<(usize, f32)> = sessions
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            sem_scores
                .get(&s.id)
                .copied()
                .filter(|v| *v >= RRF_SEM_THRESHOLD)
                .map(|v| (i, v))
        })
        .collect();
    sem.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Build session_idx → rank maps (rank is 1-based to match the RRF paper).
    let lex_ranks: std::collections::HashMap<usize, usize> = lex
        .iter()
        .enumerate()
        .map(|(rank, (idx, _))| (*idx, rank + 1))
        .collect();
    let bm25_ranks: std::collections::HashMap<usize, usize> = bm25
        .iter()
        .enumerate()
        .map(|(rank, (idx, _))| (*idx, rank + 1))
        .collect();
    let sem_ranks: std::collections::HashMap<usize, usize> = sem
        .iter()
        .enumerate()
        .map(|(rank, (idx, _))| (*idx, rank + 1))
        .collect();

    // Union of candidates — any session that appears in any layer.
    let mut all: std::collections::HashSet<usize> = std::collections::HashSet::new();
    all.extend(lex_ranks.keys());
    all.extend(bm25_ranks.keys());
    all.extend(sem_ranks.keys());

    let mut scored: Vec<(usize, RrfScoreBreakdown)> = all
        .into_iter()
        .map(|idx| {
            let s = &sessions[idx];
            let title_rank = lex_ranks.get(&idx).copied();
            let bm25_rank = bm25_ranks.get(&idx).copied();
            let semantic_rank = sem_ranks.get(&idx).copied();

            let mut rrf: f32 = 0.0;
            if let Some(r) = title_rank {
                rrf += RRF_W_TITLE / (RRF_K + r as f32);
            }
            if let Some(r) = bm25_rank {
                rrf += RRF_W_BM25 / (RRF_K + r as f32);
            }
            if let Some(r) = semantic_rank {
                rrf += RRF_W_SEM / (RRF_K + r as f32);
            }

            // Recency softened: pure multiplication crushes RRF (values are
            // already in the 0.01–0.06 range) when a session is old. We
            // keep a floor of 0.7 so recency only nudges, never dominates.
            let recency = recency_multiplier(&s.updated_at);
            let softened = 0.7 + 0.3 * recency;
            let base_final = (rrf * softened * 1_000_000.0) as u32;

            // State-label match is a small additive tie-breaker — not a
            // full RRF layer (a state-word query is rare and a state match
            // is too coarse to deserve equal weight with content layers).
            let label_lower = s.state.label().to_lowercase();
            let state_match = label_lower.contains(&query_lower)
                || query_words.iter().any(|w| label_lower.contains(w));
            let final_score = if state_match {
                base_final.saturating_add(RRF_STATE_TIE)
            } else {
                base_final
            };

            // Diagnostic snapshot of raw per-layer signals.
            let title_score = lex
                .iter()
                .find(|(i, _)| *i == idx)
                .map(|(_, score)| *score)
                .unwrap_or(0);
            let bm25_raw = log_matches.get(&s.id).copied().unwrap_or(0.0);
            let semantic_sim = sem_scores.get(&s.id).copied().unwrap_or(0.0);

            (
                idx,
                RrfScoreBreakdown {
                    final_score,
                    rrf_raw: rrf,
                    recency,
                    title_rank,
                    bm25_rank,
                    semantic_rank,
                    title_score,
                    bm25_raw,
                    semantic_sim,
                },
            )
        })
        .collect();

    scored.sort_by(|a, b| {
        b.1.final_score
            .cmp(&a.1.final_score)
            // Deterministic tie-break: lexical score first, then session id.
            .then_with(|| b.1.title_score.cmp(&a.1.title_score))
            .then_with(|| sessions[a.0].id.cmp(&sessions[b.0].id))
    });
    scored
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;
    use std::path::PathBuf;

    fn make_session(title: &str, summary: &str, provider: &str) -> Session {
        Session {
            id: format!("{}_{}", provider, title),
            provider_session_id: "abc-123".into(),
            provider_name: provider.into(),
            cwd: PathBuf::from("D:\\Demo\\myproject"),
            title: title.into(),
            tab_title: None,
            summary: summary.into(),
            state: SessionState::default(),
            pid: None,
            created_at: String::new(),
            updated_at: String::new(),
            state_dir: None,
        }
    }

    fn make_session_full(
        title: &str,
        summary: &str,
        provider: &str,
        session_id: &str,
        cwd: &str,
    ) -> Session {
        Session {
            id: format!("{}_{}", provider, session_id),
            provider_session_id: session_id.into(),
            provider_name: provider.into(),
            cwd: PathBuf::from(cwd),
            title: title.into(),
            tab_title: None,
            summary: summary.into(),
            state: SessionState::default(),
            pid: None,
            created_at: String::new(),
            updated_at: String::new(),
            state_dir: None,
        }
    }

    // ── Empty / trivial queries ──────────────────────────────────────

    #[test]
    fn empty_query_returns_all() {
        let sessions = vec![
            make_session("a", "x", "copilot"),
            make_session("b", "y", "claude"),
        ];
        let results = ranked_search(&sessions, "", None, None);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn empty_sessions_returns_empty() {
        let results = ranked_search(&[], "something", None, None);
        assert!(results.is_empty());
    }

    #[test]
    fn no_match_returns_empty() {
        let sessions = vec![
            make_session("deploy server", "production release", "copilot"),
        ];
        let results = ranked_search(&sessions, "xyznonexistent", None, None);
        assert!(results.is_empty());
    }

    // ── Tier 1: exact substring match ────────────────────────────────

    #[test]
    fn exact_title_match_ranks_highest() {
        let sessions = vec![
            make_session("fix auth bug", "some work", "copilot"),
            make_session("deploy server", "auth related fix", "copilot"),
        ];
        let results = ranked_search(&sessions, "fix auth", None, None);
        assert!(!results.is_empty());
        assert_eq!(results[0].index, 0); // exact title match first
        assert!(results[0].score >= 1000);
    }

    #[test]
    fn exact_title_beats_exact_summary() {
        let sessions = vec![
            make_session("unrelated title", "fix the authentication flow", "copilot"),
            make_session("fix the authentication flow", "unrelated summary", "copilot"),
        ];
        let results = ranked_search(&sessions, "fix the authentication", None, None);
        assert_eq!(results[0].index, 1); // title match scores higher
        assert!(results[0].score > results[1].score);
    }

    #[test]
    fn exact_session_id_match() {
        let sessions = vec![
            make_session_full("some title", "summary", "copilot", "703611e6-890c-4df2", "D:\\Demo"),
        ];
        let results = ranked_search(&sessions, "703611e6", None, None);
        assert_eq!(results.len(), 1);
        assert!(results[0].score >= 800);
    }

    #[test]
    fn exact_cwd_match() {
        let sessions = vec![
            make_session_full("title", "summary", "copilot", "abc", "D:\\Demo\\myproject"),
        ];
        let results = ranked_search(&sessions, "myproject", None, None);
        assert_eq!(results.len(), 1);
        assert!(results[0].score >= 400);
    }

    #[test]
    fn exact_provider_name_match() {
        let sessions = vec![
            make_session("title a", "summary a", "copilot"),
            make_session("title b", "summary b", "claude"),
        ];
        let results = ranked_search(&sessions, "claude", None, None);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].index, 1);
    }

    #[test]
    fn case_insensitive_matching() {
        let sessions = vec![
            make_session("Fix Authentication Bug", "IMPORTANT work", "copilot"),
        ];
        let results = ranked_search(&sessions, "fix authentication", None, None);
        assert_eq!(results.len(), 1);
        assert!(results[0].score >= 1000);
    }

    // ── Tier 2: word-level matching ──────────────────────────────────

    #[test]
    fn all_words_match_in_summary() {
        let sessions = vec![
            make_session("unrelated work", "nothing here", "copilot"),
            make_session("deploy server", "fixed the auth bug yesterday", "copilot"),
        ];
        let results = ranked_search(&sessions, "auth bug", None, None);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].index, 1);
    }

    #[test]
    fn all_words_match_scores_lower_than_exact() {
        let sessions = vec![
            // Title has "auth" and "bug" but not as "auth bug" substring
            make_session("auth system", "found the bug in handler", "copilot"),
            // Title has exact substring "auth bug"
            make_session("fix auth bug now", "todo", "copilot"),
        ];
        let results = ranked_search(&sessions, "auth bug", None, None);
        assert_eq!(results[0].index, 1); // exact match ranks first
        assert!(results[0].score > results[1].score);
    }

    #[test]
    fn partial_word_match_single_word() {
        let sessions = vec![
            make_session("authentication module", "handles login", "copilot"),
        ];
        // "auth" is a substring of "authentication" — should match
        let results = ranked_search(&sessions, "auth", None, None);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn short_words_under_3_chars_ignored_for_partial() {
        let sessions = vec![
            make_session("fix it now", "some summary", "copilot"),
        ];
        // "it" is < 3 chars, shouldn't trigger partial word match on its own
        // but "fix it" as full query IS an exact substring match in title
        let results = ranked_search(&sessions, "it", None, None);
        // "it" appears in title as exact substring match
        assert_eq!(results.len(), 1);
    }

    // ── Tier 2c: state label matching ────────────────────────────────

    #[test]
    fn search_running_state() {
        let mut s = make_session("my session", "stuff", "copilot");
        s.state.process = ProcessState::Running;
        s.state.interaction = InteractionState::Busy;
        let sessions = vec![s];
        let results = ranked_search(&sessions, "running", None, None);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn search_waiting_state() {
        let mut s = make_session("my session", "stuff", "copilot");
        s.state.process = ProcessState::Running;
        s.state.interaction = InteractionState::WaitingInput;
        let sessions = vec![s];
        let results = ranked_search(&sessions, "waiting", None, None);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn search_resumable_state() {
        let mut s = make_session("my session", "stuff", "copilot");
        s.state.persistence = PersistenceState::Resumable;
        let sessions = vec![s];
        let results = ranked_search(&sessions, "resumable", None, None);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn state_label_ranks_lower_than_title() {
        let s1 = make_session("running tests", "unit tests", "copilot");
        // s1 matches "running" in title (score 1000)
        let mut s2 = make_session("deploy app", "production", "copilot");
        s2.state.process = ProcessState::Running;
        s2.state.interaction = InteractionState::Busy;
        // s2 matches "running" in state label (score 200)
        let sessions = vec![s1, s2];
        let results = ranked_search(&sessions, "running", None, None);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].index, 0); // title match ranks higher
        assert!(results[0].score > results[1].score);
    }

    // ── Ranking order / multi-field ──────────────────────────────────

    #[test]
    fn ranking_preserves_order_by_score() {
        let sessions = vec![
            make_session("unrelated", "deploy the auth system", "copilot"),  // summary match (600)
            make_session("auth system deploy", "nothing", "copilot"),         // title match (1000)
            make_session("other work", "stuff", "copilot"),                   // no match
        ];
        let results = ranked_search(&sessions, "auth", None, None);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].index, 1); // title match first (1000)
        assert_eq!(results[1].index, 0); // summary match second (600)
    }

    #[test]
    fn multiple_matches_all_returned() {
        let sessions = vec![
            make_session("auth login", "handles auth", "copilot"),
            make_session("auth signup", "new user auth", "claude"),
            make_session("deploy server", "no match here", "copilot"),
        ];
        let results = ranked_search(&sessions, "auth", None, None);
        assert_eq!(results.len(), 2); // only 2 match, not the deploy one
    }

    // ── Multi-word queries ───────────────────────────────────────────

    #[test]
    fn multi_word_exact_phrase_in_title() {
        let sessions = vec![
            make_session("fix the authentication bug", "work", "copilot"),
            make_session("authentication fix", "bug report", "copilot"),
        ];
        let results = ranked_search(&sessions, "fix the authentication bug", None, None);
        assert_eq!(results[0].index, 0); // exact phrase match
    }

    #[test]
    fn multi_word_scattered_across_field() {
        let sessions = vec![
            make_session("the server has a bug in authentication", "details", "copilot"),
        ];
        // "authentication bug" — both words present but not as exact phrase
        let results = ranked_search(&sessions, "authentication bug", None, None);
        assert_eq!(results.len(), 1);
        // Should match via word-level matching (tier 2)
        assert!(results[0].score > 0);
        assert!(results[0].score < 1000); // not exact match score
    }

    // ── Semantic plugin status ───────────────────────────────────────

    #[test]
    fn semantic_plugin_defaults_unavailable() {
        let plugin = SemanticPlugin::new();
        assert_eq!(*plugin.status(), SemanticStatus::Unavailable);
    }

    // ── Regression tests (weak-search investigation 2026-05-13) ──────

    /// Regression: 4-word query where only ONE word ("march") appears in
    /// an unrelated session's title used to surface that session above
    /// the real target. Tier 2b now requires ≥2 word hits when the query
    /// has multiple words, eliminating "single-common-word" noise.
    ///
    /// User-visible symptom: searching "Iteration Review March Seana"
    /// surfaced unrelated sessions whose only commonality was a single
    /// word in their title.
    #[test]
    fn multi_word_query_single_hit_no_longer_scores() {
        let sessions = vec![
            // Noise: only "march" matches; should NOT score under tier 2b.
            make_session("March release notes", "release planning", "copilot"),
            // Real target: 3 of 4 words match in summary.
            make_session(
                "Build slides",
                "Iteration review notes for March meeting",
                "copilot",
            ),
        ];
        let results = ranked_search(&sessions, "Iteration Review March Seana", None, None);
        // The "March release notes" row must NOT surface — it only matches
        // one word out of four (below the new min_hits=2 threshold).
        let surfaced_titles: Vec<&str> = results
            .iter()
            .map(|r| sessions[r.index].title.as_str())
            .collect();
        assert!(
            !surfaced_titles.contains(&"March release notes"),
            "single-word-hit noise leaked: {:?}",
            surfaced_titles
        );
        // The real target with 3 matching words MUST surface.
        assert!(
            surfaced_titles.contains(&"Build slides"),
            "3-of-4-word match was filtered out: {:?}",
            surfaced_titles
        );
    }

    /// Regression: single-word queries must still score on a single hit.
    /// The tightened tier 2b threshold is multi-word-only — a one-word
    /// search like "/auth" must still return everything containing "auth".
    #[test]
    fn single_word_query_still_scores_on_one_hit() {
        let sessions = vec![
            make_session("Fix auth bug", "JWT refresh", "copilot"),
        ];
        let results = ranked_search(&sessions, "auth", None, None);
        assert_eq!(results.len(), 1, "single-word query lost recall");
        assert!(results[0].score > 0);
    }

    /// Regression: when a session's BODY (events.jsonl, indexed via
    /// tantivy) matches many query terms but its title/summary don't,
    /// the BM25 boost must dominate over title-tier scores. Previously
    /// the BM25 bonus was capped at 350, below title-exact 1000, so
    /// "session about X" queries never beat "session named X".
    #[test]
    fn strong_body_match_can_outrank_weak_title_match() {
        let sessions = vec![
            // Title contains "iteration" → tier 2b ≈ base/4 + 50.
            make_session("iteration something", "", "copilot"),
            // Title is unrelated; body has very strong BM25 (simulated 12.0).
            make_session("Build slides", "", "copilot"),
        ];
        // Inject a strong body BM25 for the "Build slides" session.
        let mut log_matches = std::collections::HashMap::new();
        let body_match_id = sessions[1].id.clone();
        log_matches.insert(body_match_id.clone(), 12.0_f32);
        let results = ranked_search(&sessions, "iteration review", None, Some(&log_matches));
        // The body-match session must rank #1.
        assert_eq!(
            sessions[results[0].index].id, body_match_id,
            "strong body match did not outrank weak title match: {:?}",
            results
                .iter()
                .map(|r| (sessions[r.index].title.clone(), r.score))
                .collect::<Vec<_>>()
        );
    }
}
