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
pub fn ranked_search(
    sessions: &[Session],
    query: &str,
    semantic: Option<&SemanticPlugin>,
) -> Vec<SearchResult> {
    if query.is_empty() {
        return (0..sessions.len())
            .map(|i| SearchResult { index: i, score: 0, semantic_match: false })
            .collect();
    }

    let query_lower = query.to_lowercase();
    let query_words: Vec<&str> = query_lower.split_whitespace().collect();

    // Tier 3: pre-compute semantic matches from cached embeddings
    let semantic_scores: HashMap<String, f32> = if query.len() >= 5 {
        semantic
            .filter(|s| s.is_ready())
            .map(|s| s.search_cached(query, 0.4).into_iter().collect())
            .unwrap_or_default()
    } else {
        HashMap::new()
    };

    let mut results: Vec<SearchResult> = sessions
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let mut score = score_session(s, &query_lower, &query_words);
            let mut semantic_match = false;

            // Tier 3: semantic boost from cached vectors (instant lookup)
            if let Some(&sim) = semantic_scores.get(&s.id) {
                let boost = ((sim - 0.4) * 333.0).min(200.0) as u32;
                score = score.saturating_add(boost);
                semantic_match = true;
            }

            if score > 0 { Some(SearchResult { index: i, score, semantic_match }) } else { None }
        })
        .collect();

    // Sort by score descending (highest relevance first)
    results.sort_by(|a, b| b.score.cmp(&a.score));
    results
}

/// Score a single session against a query.
fn score_session(session: &Session, query: &str, query_words: &[&str]) -> u32 {
    let fields = [
        (&session.title, 1000u32),       // title exact match = highest
        (&session.provider_session_id, 800),
        (&session.summary, 600),
        (&session.cwd.to_string_lossy().to_string(), 400),
        (&session.provider_name, 300),
    ];

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

        // Tier 2b: any query word appears in the field (partial word match)
        let word_hits: u32 = query_words
            .iter()
            .filter(|w| w.len() >= 3 && field_lower.contains(*w))
            .count() as u32;
        if word_hits > 0 {
            let partial_score = base_score / 4 + word_hits * 50;
            best_score = best_score.max(partial_score);
        }
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
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct EmbeddingCache {
    entries: HashMap<String, CachedEmbedding>,
}

impl EmbeddingCache {
    fn load(path: &std::path::Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save(&self, path: &std::path::Path) {
        if let Ok(json) = serde_json::to_string(self) {
            let _ = std::fs::write(path, json);
        }
    }
}

fn hash_text(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Semantic search plugin — loads a shared library at runtime.
pub struct SemanticPlugin {
    status: SemanticStatus,
    pub(crate) lib: Option<libloading::Library>,
    dim: i32,
    cache: EmbeddingCache,
    cache_path: Option<std::path::PathBuf>,
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
            lib: None,
            dim: 0,
            cache: EmbeddingCache::default(),
            cache_path: None,
        }
    }

    pub fn status(&self) -> &SemanticStatus {
        &self.status
    }

    pub fn is_ready(&self) -> bool {
        matches!(self.status, SemanticStatus::Ready { .. })
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
        self.status = SemanticStatus::Indexing { done: 0, total: 0 };

        let lib = match unsafe { libloading::Library::new(&dll_path) } {
            Ok(l) => l,
            Err(e) => {
                let msg = format!("Failed to load DLL: {}", e);
                crate::log::error(&msg);
                self.status = SemanticStatus::Failed(msg);
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
                        self.status = SemanticStatus::Failed(msg);
                        return;
                    }
                };
            let c_dir = std::ffi::CString::new(cache_dir).unwrap_or_default();
            init(c_dir.as_ptr())
        };

        if init_result != 0 {
            let msg = "semantic_init returned error".to_string();
            crate::log::error(&msg);
            self.status = SemanticStatus::Failed(msg);
            return;
        }

        // Get embedding dimension
        let dim: i32 = unsafe {
            let dim_fn: libloading::Symbol<unsafe extern "C" fn() -> i32> =
                match lib.get(b"semantic_dim") {
                    Ok(f) => f,
                    Err(_) => {
                        self.status = SemanticStatus::Failed("Missing semantic_dim".into());
                        return;
                    }
                };
            dim_fn()
        };

        if dim <= 0 {
            self.status = SemanticStatus::Failed("Invalid embedding dimension".into());
            return;
        }

        self.dim = dim;
        self.lib = Some(lib);

        // Load embedding cache from disk
        let cache_file = std::path::PathBuf::from(cache_dir).join("embeddings_cache.json");
        self.cache = EmbeddingCache::load(&cache_file);
        self.cache_path = Some(cache_file);

        let cached_count = self.cache.entries.len();
        self.status = if cached_count > 0 {
            SemanticStatus::Ready { count: cached_count }
        } else {
            SemanticStatus::Indexing { done: 0, total: 0 }
        };
        crate::log::info(&format!(
            "Semantic plugin loaded (dim={}, cached={})",
            dim, cached_count
        ));
    }

    /// Index sessions: compute embeddings for new/changed sessions.
    /// Only embeds sessions whose title+summary hash changed. Saves cache to disk.
    /// Returns (newly_embedded, total_cached).
    pub fn index_sessions(&mut self, sessions: &[Session]) -> (usize, usize) {
        if self.lib.is_none() || self.dim <= 0 {
            return (0, 0);
        }

        let total = sessions.len();
        let mut newly_embedded = 0usize;

        for (i, session) in sessions.iter().enumerate() {
            let text = format!("{} {}", session.title, session.summary);
            let text_hash = hash_text(&text);

            // Skip if already cached with same hash
            if let Some(cached) = self.cache.entries.get(&session.id) {
                if cached.text_hash == text_hash {
                    continue;
                }
            }

            // Embed this session
            if let Some(vec) = self.embed(&text) {
                self.cache.entries.insert(
                    session.id.clone(),
                    CachedEmbedding {
                        text_hash,
                        vector: vec,
                    },
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

        let count = self.cache.entries.len();
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
    pub fn search_cached(&self, query: &str, threshold: f32) -> Vec<(String, f32)> {
        let query_vec = match self.embed(query) {
            Some(v) => v,
            None => return vec![],
        };

        let mut results: Vec<(String, f32)> = self
            .cache
            .entries
            .iter()
            .filter_map(|(id, cached)| {
                let sim = cosine_similarity(&query_vec, &cached.vector);
                if sim > threshold {
                    Some((id.clone(), sim))
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
        let results = ranked_search(&sessions, "", None);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn empty_sessions_returns_empty() {
        let results = ranked_search(&[], "something", None);
        assert!(results.is_empty());
    }

    #[test]
    fn no_match_returns_empty() {
        let sessions = vec![
            make_session("deploy server", "production release", "copilot"),
        ];
        let results = ranked_search(&sessions, "xyznonexistent", None);
        assert!(results.is_empty());
    }

    // ── Tier 1: exact substring match ────────────────────────────────

    #[test]
    fn exact_title_match_ranks_highest() {
        let sessions = vec![
            make_session("fix auth bug", "some work", "copilot"),
            make_session("deploy server", "auth related fix", "copilot"),
        ];
        let results = ranked_search(&sessions, "fix auth", None);
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
        let results = ranked_search(&sessions, "fix the authentication", None);
        assert_eq!(results[0].index, 1); // title match scores higher
        assert!(results[0].score > results[1].score);
    }

    #[test]
    fn exact_session_id_match() {
        let sessions = vec![
            make_session_full("some title", "summary", "copilot", "703611e6-890c-4df2", "D:\\Demo"),
        ];
        let results = ranked_search(&sessions, "703611e6", None);
        assert_eq!(results.len(), 1);
        assert!(results[0].score >= 800);
    }

    #[test]
    fn exact_cwd_match() {
        let sessions = vec![
            make_session_full("title", "summary", "copilot", "abc", "D:\\Demo\\myproject"),
        ];
        let results = ranked_search(&sessions, "myproject", None);
        assert_eq!(results.len(), 1);
        assert!(results[0].score >= 400);
    }

    #[test]
    fn exact_provider_name_match() {
        let sessions = vec![
            make_session("title a", "summary a", "copilot"),
            make_session("title b", "summary b", "claude"),
        ];
        let results = ranked_search(&sessions, "claude", None);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].index, 1);
    }

    #[test]
    fn case_insensitive_matching() {
        let sessions = vec![
            make_session("Fix Authentication Bug", "IMPORTANT work", "copilot"),
        ];
        let results = ranked_search(&sessions, "fix authentication", None);
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
        let results = ranked_search(&sessions, "auth bug", None);
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
        let results = ranked_search(&sessions, "auth bug", None);
        assert_eq!(results[0].index, 1); // exact match ranks first
        assert!(results[0].score > results[1].score);
    }

    #[test]
    fn partial_word_match_single_word() {
        let sessions = vec![
            make_session("authentication module", "handles login", "copilot"),
        ];
        // "auth" is a substring of "authentication" — should match
        let results = ranked_search(&sessions, "auth", None);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn short_words_under_3_chars_ignored_for_partial() {
        let sessions = vec![
            make_session("fix it now", "some summary", "copilot"),
        ];
        // "it" is < 3 chars, shouldn't trigger partial word match on its own
        // but "fix it" as full query IS an exact substring match in title
        let results = ranked_search(&sessions, "it", None);
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
        let results = ranked_search(&sessions, "running", None);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn search_waiting_state() {
        let mut s = make_session("my session", "stuff", "copilot");
        s.state.process = ProcessState::Running;
        s.state.interaction = InteractionState::WaitingInput;
        let sessions = vec![s];
        let results = ranked_search(&sessions, "waiting", None);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn search_resumable_state() {
        let mut s = make_session("my session", "stuff", "copilot");
        s.state.persistence = PersistenceState::Resumable;
        let sessions = vec![s];
        let results = ranked_search(&sessions, "resumable", None);
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
        let results = ranked_search(&sessions, "running", None);
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
        let results = ranked_search(&sessions, "auth", None);
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
        let results = ranked_search(&sessions, "auth", None);
        assert_eq!(results.len(), 2); // only 2 match, not the deploy one
    }

    // ── Multi-word queries ───────────────────────────────────────────

    #[test]
    fn multi_word_exact_phrase_in_title() {
        let sessions = vec![
            make_session("fix the authentication bug", "work", "copilot"),
            make_session("authentication fix", "bug report", "copilot"),
        ];
        let results = ranked_search(&sessions, "fix the authentication bug", None);
        assert_eq!(results[0].index, 0); // exact phrase match
    }

    #[test]
    fn multi_word_scattered_across_field() {
        let sessions = vec![
            make_session("the server has a bug in authentication", "details", "copilot"),
        ];
        // "authentication bug" — both words present but not as exact phrase
        let results = ranked_search(&sessions, "authentication bug", None);
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
}
