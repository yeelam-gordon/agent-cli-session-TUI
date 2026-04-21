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
}

/// Rank sessions against a query. Returns indices sorted by relevance (highest first).
pub fn ranked_search(sessions: &[Session], query: &str) -> Vec<SearchResult> {
    if query.is_empty() {
        return (0..sessions.len())
            .map(|i| SearchResult { index: i, score: 0 })
            .collect();
    }

    let query_lower = query.to_lowercase();
    let query_words: Vec<&str> = query_lower.split_whitespace().collect();

    let mut results: Vec<SearchResult> = sessions
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let score = score_session(s, &query_lower, &query_words);
            if score > 0 { Some(SearchResult { index: i, score }) } else { None }
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
// Semantic search plugin (optional DLL) — future integration point
// ---------------------------------------------------------------------------

/// Status of the semantic search plugin.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum SemanticStatus {
    /// Not available (DLL not found).
    Unavailable,
    /// Loading model (first use).
    Loading,
    /// Ready for queries.
    Ready,
    /// Failed to load.
    Failed(String),
}

/// Semantic search plugin — loads a shared library at runtime.
///
/// The DLL must export these C functions:
/// ```c
/// int32_t semantic_init(const char* model_dir);
/// int32_t semantic_embed(const char* text, float* out_vec, int32_t max_dim);
/// int32_t semantic_embed_dim();
/// ```
///
/// If the DLL is not present, all operations return gracefully.
#[allow(dead_code)]
pub struct SemanticPlugin {
    status: SemanticStatus,
}

impl Default for SemanticPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
impl SemanticPlugin {
    pub fn new() -> Self {
        Self {
            status: SemanticStatus::Unavailable,
        }
    }

    pub fn status(&self) -> &SemanticStatus {
        &self.status
    }

    /// Try to load the semantic search DLL from next to the executable.
    pub fn try_load(&mut self) {
        let dll_name = if cfg!(windows) {
            "semantic_search.dll"
        } else if cfg!(target_os = "macos") {
            "libsemantic_search.dylib"
        } else {
            "libsemantic_search.so"
        };

        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()));

        if let Some(dir) = exe_dir {
            let dll_path = dir.join(dll_name);
            if dll_path.exists() {
                crate::log::info(&format!("Semantic plugin found: {:?}", dll_path));
                // TODO: load via libloading, call semantic_init
                self.status = SemanticStatus::Unavailable; // placeholder
            } else {
                self.status = SemanticStatus::Unavailable;
            }
        }
    }
}

#[allow(dead_code)]
impl SearchResult {
    pub fn new(index: usize, score: u32) -> Self {
        Self { index, score }
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

    #[test]
    fn exact_title_match_ranks_highest() {
        let sessions = vec![
            make_session("fix auth bug", "some work", "copilot"),
            make_session("deploy server", "auth related fix", "copilot"),
        ];
        let results = ranked_search(&sessions, "fix auth");
        assert!(!results.is_empty());
        assert_eq!(results[0].index, 0); // exact title match first
    }

    #[test]
    fn word_match_finds_partial() {
        let sessions = vec![
            make_session("unrelated work", "nothing here", "copilot"),
            make_session("deploy server", "fixed the auth bug yesterday", "copilot"),
        ];
        let results = ranked_search(&sessions, "auth bug");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].index, 1); // found via summary word match
    }

    #[test]
    fn empty_query_returns_all() {
        let sessions = vec![
            make_session("a", "x", "copilot"),
            make_session("b", "y", "claude"),
        ];
        let results = ranked_search(&sessions, "");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn state_label_searchable() {
        let mut s = make_session("my session", "stuff", "copilot");
        s.state.process = ProcessState::Running;
        s.state.interaction = InteractionState::WaitingInput;
        let sessions = vec![s];
        let results = ranked_search(&sessions, "waiting");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn no_match_returns_empty() {
        let sessions = vec![
            make_session("deploy server", "production release", "copilot"),
        ];
        let results = ranked_search(&sessions, "xyznonexistent");
        assert!(results.is_empty());
    }
}
