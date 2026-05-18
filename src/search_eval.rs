//! Search evaluation harness.
//!
//! A small information-retrieval style benchmark for the search pipeline.
//! Given a TOML file of `(query, target session id, category)` triples,
//! runs the same `discover → log_search.refresh → ranked_search` pipeline
//! used by the live TUI against every session on disk, locates each
//! target's rank, and reports aggregate IR metrics:
//!
//! - **MRR**     — Mean Reciprocal Rank (primary "find the right one")
//! - **P@1**     — Precision at 1 (did we nail it first try?)
//! - **R@K**     — Recall at 5, 10, 20 (did the target appear at all?)
//! - **Failures** — queries whose target ranks below 20
//!
//! Metrics are also broken down per category so a change that helps
//! "exact-title" queries but hurts "partial-recall" ones is visible.
//!
//! ## Privacy
//!
//! Queries often contain real names or internal topic words. The default
//! lookup is `eval/search-queries.toml` — that file is gitignored. A
//! committed `eval/search-queries.example.toml` shows the schema with
//! synthetic content. CI / public repos see only the example.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
use crate::log_search;
use crate::models;
use crate::provider::ProviderRegistry;
use crate::search;

/// One labeled query in the eval set.
#[derive(Debug, Deserialize)]
struct QuerySpec {
    /// Free-form query text typed by the user.
    text: String,
    /// Expected target — provider-specific session id (e.g. Copilot UUID).
    target: String,
    /// Free-form category bucket — e.g. "exact-title", "partial-recall",
    /// "typo", "semantic-only". Aggregated in the report.
    #[serde(default)]
    category: String,
    /// Optional human note — not used in scoring.
    #[serde(default)]
    notes: String,
}

/// Top-level shape of `search-queries.toml`.
#[derive(Debug, Deserialize)]
struct QueryFile {
    #[serde(default, rename = "query")]
    queries: Vec<QuerySpec>,
}

/// Per-query result row used by the report.
#[derive(Debug, Serialize)]
struct QueryResult {
    text: String,
    target: String,
    category: String,
    notes: String,
    /// 1-based rank of the target session, or `None` if not present.
    rank: Option<usize>,
    /// Reciprocal rank: 1/rank, or 0 if not found.
    reciprocal_rank: f32,
    /// Title of the top result (sanity check that we're indexing what we think).
    top1_title: String,
}

/// Aggregated metrics for a slice of queries (overall or per-category).
#[derive(Debug, Serialize, Default)]
struct Aggregate {
    n: usize,
    mrr: f32,
    p_at_1: f32,
    recall_at_5: f32,
    recall_at_10: f32,
    recall_at_20: f32,
}

impl Aggregate {
    fn compute(results: &[&QueryResult]) -> Self {
        let n = results.len();
        if n == 0 {
            return Self::default();
        }
        let denom = n as f32;
        let mrr = results.iter().map(|r| r.reciprocal_rank).sum::<f32>() / denom;
        let p1 = results.iter().filter(|r| r.rank == Some(1)).count() as f32 / denom;
        let r5 = results.iter().filter(|r| matches!(r.rank, Some(k) if k <= 5)).count() as f32
            / denom;
        let r10 = results.iter().filter(|r| matches!(r.rank, Some(k) if k <= 10)).count() as f32
            / denom;
        let r20 = results.iter().filter(|r| matches!(r.rank, Some(k) if k <= 20)).count() as f32
            / denom;
        Self {
            n,
            mrr,
            p_at_1: p1,
            recall_at_5: r5,
            recall_at_10: r10,
            recall_at_20: r20,
        }
    }
}

/// Full report — what we serialize to JSON for diffing across runs.
#[derive(Debug, Serialize)]
struct Report {
    /// ISO-8601 timestamp when the eval ran.
    timestamp: String,
    /// Number of sessions discovered across all providers.
    sessions_total: usize,
    /// Aggregate metrics over all queries.
    overall: Aggregate,
    /// Aggregate metrics per category.
    by_category: HashMap<String, Aggregate>,
    /// Per-query rows in the original order.
    queries: Vec<QueryResult>,
}

/// Resolve the queries file path, with fallback to the example.
/// Tries `--queries <path>` first, then `eval/search-queries.toml`, then
/// `eval/search-queries.example.toml`. Returns an error if none exist.
fn resolve_queries_path(explicit: Option<&str>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        let path = PathBuf::from(p);
        if !path.exists() {
            anyhow::bail!("queries file not found: {}", path.display());
        }
        return Ok(path);
    }
    let candidates = [
        PathBuf::from("eval").join("search-queries.toml"),
        PathBuf::from("eval").join("search-queries.example.toml"),
    ];
    for c in &candidates {
        if c.exists() {
            return Ok(c.clone());
        }
    }
    anyhow::bail!(
        "no queries file found — looked in {} and {}",
        candidates[0].display(),
        candidates[1].display()
    );
}

/// Entry point invoked from `main.rs` when `--search-eval` is passed.
pub fn run_search_eval(
    registry: &ProviderRegistry,
    config: &AppConfig,
    queries_path: Option<&str>,
    report_path: Option<&str>,
) -> Result<()> {
    let qpath = resolve_queries_path(queries_path)?;
    println!("Eval queries: {}", qpath.display());
    println!("Scorer: RRF (default)");

    let toml_text = std::fs::read_to_string(&qpath)
        .with_context(|| format!("reading {}", qpath.display()))?;
    let qfile: QueryFile = toml::from_str(&toml_text)
        .with_context(|| format!("parsing TOML in {}", qpath.display()))?;
    println!("  {} queries loaded", qfile.queries.len());
    if qfile.queries.is_empty() {
        println!("  (nothing to evaluate — add [[query]] entries to the file)");
        return Ok(());
    }
    println!();

    // Discover once across all providers — shared by every query.
    println!("Discovering sessions...");
    let start = std::time::Instant::now();
    let mut all_sessions: Vec<models::Session> = Vec::new();
    for prov in registry.providers() {
        match prov.discover_sessions() {
            Ok(sessions) => {
                println!("  {}: {} sessions", prov.name(), sessions.len());
                all_sessions.extend(sessions);
            }
            Err(e) => eprintln!("  {}: discover failed: {}", prov.name(), e),
        }
    }
    println!(
        "  Total: {} sessions in {:?}",
        all_sessions.len(),
        start.elapsed()
    );
    println!();

    // Refresh the index once. Real-world ranking is what we want to measure.
    println!("Refreshing log index (tantivy)...");
    let start = std::time::Instant::now();
    let searcher = log_search::LogSearcher::open_or_create(&config.data_dir)?;
    if let Err(e) = searcher.refresh(&all_sessions, registry) {
        eprintln!("  refresh error (continuing): {:#}", e);
    }
    println!("  refresh: {:?}", start.elapsed());
    println!();

    // Load semantic plugin once.
    let cache_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("agent-session-tui")
        .join("models");
    let mut sem = search::SemanticPlugin::new();
    sem.try_load(&cache_dir.to_string_lossy());
    let sem_ready = sem.is_ready();
    println!("Semantic plugin ready: {}", sem_ready);
    println!();

    // Run each query through the same ranking pipeline as the live TUI.
    let mut results: Vec<QueryResult> = Vec::with_capacity(qfile.queries.len());
    for q in &qfile.queries {
        let result = score_one_query(&all_sessions, &searcher, &mut sem, sem_ready, q);
        results.push(result);
    }

    // Compose the report.
    let timestamp = chrono::Utc::now().to_rfc3339();
    let overall = Aggregate::compute(&results.iter().collect::<Vec<_>>());
    let mut by_category: HashMap<String, Vec<&QueryResult>> = HashMap::new();
    for r in &results {
        by_category
            .entry(if r.category.is_empty() {
                "(uncategorized)".to_string()
            } else {
                r.category.clone()
            })
            .or_default()
            .push(r);
    }
    let by_category: HashMap<String, Aggregate> = by_category
        .into_iter()
        .map(|(k, v)| (k, Aggregate::compute(&v)))
        .collect();

    let report = Report {
        timestamp,
        sessions_total: all_sessions.len(),
        overall,
        by_category,
        queries: results,
    };

    print_report(&report);

    if let Some(path) = report_path {
        let json = serde_json::to_string_pretty(&report)?;
        if let Some(parent) = Path::new(path).parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(path, json).with_context(|| format!("writing {}", path))?;
        println!();
        println!("JSON report written to: {}", path);
    }

    Ok(())
}

/// Run a single query end-to-end and locate the target's rank.
/// Uses the RRF pipeline — same as the live TUI's `ranked_search_default`.
fn score_one_query(
    all_sessions: &[models::Session],
    searcher: &log_search::LogSearcher,
    sem: &mut search::SemanticPlugin,
    sem_ready: bool,
    q: &QuerySpec,
) -> QueryResult {
    let log_matches = searcher.search(&q.text);
    let sem_scores: HashMap<String, f32> = if sem_ready && q.text.len() >= 5 {
        sem.search_cached(&q.text, 0.0).into_iter().collect()
    } else {
        HashMap::new()
    };

    let scored = search::ranked_search_rrf(all_sessions, &q.text, &log_matches, &sem_scores);
    let rank = scored
        .iter()
        .position(|(i, _)| all_sessions[*i].provider_session_id == q.target)
        .map(|p| p + 1);
    let top1_title = scored
        .first()
        .map(|(i, _)| crate::util::truncate_str_safe(&all_sessions[*i].title, 60))
        .unwrap_or_default();

    let reciprocal_rank = rank.map(|r| 1.0 / r as f32).unwrap_or(0.0);

    QueryResult {
        text: q.text.clone(),
        target: q.target.clone(),
        category: q.category.clone(),
        notes: q.notes.clone(),
        rank,
        reciprocal_rank,
        top1_title,
    }
}

/// Pretty-print the report to stdout as a table.
fn print_report(report: &Report) {
    println!("─── Overall ({} queries, {} sessions) ───", report.overall.n, report.sessions_total);
    println!("  MRR           = {:.3}", report.overall.mrr);
    println!("  P@1           = {:.1}%", report.overall.p_at_1 * 100.0);
    println!("  Recall@5      = {:.1}%", report.overall.recall_at_5 * 100.0);
    println!("  Recall@10     = {:.1}%", report.overall.recall_at_10 * 100.0);
    println!("  Recall@20     = {:.1}%", report.overall.recall_at_20 * 100.0);
    println!();

    if !report.by_category.is_empty() {
        println!("─── By category ───");
        let mut keys: Vec<&String> = report.by_category.keys().collect();
        keys.sort();
        println!(
            "  {:<24} {:>4}  {:>5}  {:>5}  {:>5}  {:>5}  {:>5}",
            "category", "n", "MRR", "P@1", "R@5", "R@10", "R@20"
        );
        for k in keys {
            let a = &report.by_category[k];
            println!(
                "  {:<24} {:>4}  {:>5.2}  {:>4.0}%  {:>4.0}%  {:>4.0}%  {:>4.0}%",
                k,
                a.n,
                a.mrr,
                a.p_at_1 * 100.0,
                a.recall_at_5 * 100.0,
                a.recall_at_10 * 100.0,
                a.recall_at_20 * 100.0,
            );
        }
        println!();
    }

    println!("─── Per query ───");
    println!("  {:>4}  {:<22}  {:<32}  top1", "rank", "category", "query");
    for r in &report.queries {
        let rank_disp = match r.rank {
            Some(k) => k.to_string(),
            None => "—".to_string(),
        };
        let query_short = crate::util::truncate_str_safe(&r.text, 30);
        let cat_short = crate::util::truncate_str_safe(&r.category, 20);
        let title_short = crate::util::truncate_str_safe(&r.top1_title, 40);
        println!(
            "  {:>4}  {:<22}  {:<32}  {}",
            rank_disp, cat_short, query_short, title_short
        );
    }
    println!();

    // Failure callout — the most important thing to see.
    let failures: Vec<&QueryResult> = report
        .queries
        .iter()
        .filter(|r| r.rank.is_none_or(|k| k > 20))
        .collect();
    if !failures.is_empty() {
        println!("─── Failures (target below rank 20 or missing) ───");
        for f in failures {
            println!(
                "  query={:?}  target={}  rank={}  category={}",
                f.text,
                f.target,
                f.rank
                    .map(|k| k.to_string())
                    .unwrap_or_else(|| "MISSING".to_string()),
                f.category
            );
        }
        println!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_handles_empty_input() {
        let agg = Aggregate::compute(&[]);
        assert_eq!(agg.n, 0);
        assert_eq!(agg.mrr, 0.0);
        assert_eq!(agg.p_at_1, 0.0);
    }

    #[test]
    fn aggregate_computes_mrr_and_recall() {
        let r1 = QueryResult {
            text: "a".into(),
            target: "x".into(),
            category: String::new(),
            notes: String::new(),
            rank: Some(1),
            reciprocal_rank: 1.0,
            top1_title: String::new(),
        };
        let r2 = QueryResult {
            text: "b".into(),
            target: "y".into(),
            category: String::new(),
            notes: String::new(),
            rank: Some(4),
            reciprocal_rank: 0.25,
            top1_title: String::new(),
        };
        let r3 = QueryResult {
            text: "c".into(),
            target: "z".into(),
            category: String::new(),
            notes: String::new(),
            rank: None,
            reciprocal_rank: 0.0,
            top1_title: String::new(),
        };
        let agg = Aggregate::compute(&[&r1, &r2, &r3]);
        assert_eq!(agg.n, 3);
        // (1 + 0.25 + 0) / 3
        assert!((agg.mrr - 0.4167).abs() < 0.001);
        // 1 of 3 at rank 1
        assert!((agg.p_at_1 - 0.3333).abs() < 0.001);
        // r1 + r2 both <= 5; r3 missing → 2/3
        assert!((agg.recall_at_5 - 0.6667).abs() < 0.001);
        assert!((agg.recall_at_10 - 0.6667).abs() < 0.001);
        assert!((agg.recall_at_20 - 0.6667).abs() < 0.001);
    }

    #[test]
    fn resolve_queries_returns_error_for_missing_explicit_path() {
        let err = resolve_queries_path(Some("definitely/does/not/exist/queries.toml")).unwrap_err();
        assert!(err.to_string().contains("queries file not found"));
    }
}
