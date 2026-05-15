mod archive;
mod config;
mod focus;
mod groups;
mod acp;
mod log;
mod log_search;
mod mock;
mod models;
mod process_info;
mod provider;
mod search;
mod search_eval;
mod supervisor;
mod ui;
mod util;
#[cfg(target_os = "windows")]
mod wt_tabs;
#[cfg(not(target_os = "windows"))]
mod wt_tabs {
    pub fn list_tab_titles() -> anyhow::Result<Vec<String>> {
        Ok(Vec::new())
    }
}

use std::sync::{Arc, Mutex};

use anyhow::Result;
use tokio::sync::mpsc;

use archive::ArchiveStore;
use config::AppConfig;
use provider::config_driven::ConfigDrivenProvider;
use provider::ProviderRegistry;
use supervisor::Supervisor;
use ui::App;

/// Create a provider instance by loading `providers/<key>.yaml`.
///
/// All five providers (copilot, claude, codex, qwen, gemini) are defined
/// declaratively in YAML and driven by `ConfigDrivenProvider`. If the
/// YAML file for a given key is missing or fails to parse, the provider
/// is skipped with a log line — same behaviour as an unknown provider.
fn create_provider(
    key: &str,
    config: &config::ProviderConfig,
) -> Option<Box<dyn provider::Provider>> {
    // Candidate search paths for `providers/<key>.yaml`, tried in order.
    // Priority: installed layout (next to exe) > crate-root > cwd (last-resort,
    // since the cwd may contain a stale copy from a prior build).
    //   1. <exe-dir>/providers/<key>.yaml          (installed layout / target/release after sync)
    //   2. <exe-dir>/../providers/<key>.yaml       (cargo target/debug next to target/)
    //   3. <exe-dir>/../../providers/<key>.yaml    (cargo target/release — crate root)
    //   4. cwd/providers/<key>.yaml                (developer / cargo run — last-resort)
    let rel = std::path::PathBuf::from("providers").join(format!("{}.yaml", key));
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(p) = exe.parent() {
            candidates.push(p.join(&rel));
            if let Some(pp) = p.parent() {
                candidates.push(pp.join(&rel));
                if let Some(ppp) = pp.parent() {
                    candidates.push(ppp.join(&rel));
                }
            }
        }
    }
    candidates.push(rel.clone());
    for path in &candidates {
        if path.exists() {
            match ConfigDrivenProvider::load_from_yaml(path, config) {
                Ok(p) => {
                    log::info(&format!("Provider '{}' loaded from {:?}", key, path));
                    return Some(Box::new(p));
                }
                Err(e) => {
                    log::warn(&format!("YAML load failed for {:?}: {}", path, e));
                }
            }
        }
    }
    log::warn(&format!(
        "Provider '{}' skipped — providers/{}.yaml not found in any of {:?}",
        key, key, candidates
    ));
    None
}

#[tokio::main]
async fn main() -> Result<()> {
    // Init log file in temp directory
    let log_path = std::env::temp_dir().join("agent-session-tui.log");
    log::init(log_path.clone());
    log::info("=== agent-session-tui starting ===");

    // Force backtraces ON for the rest of this process so panic_hook below
    // captures useful frames. Cheap; no effect when no panic occurs.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        std::env::set_var("RUST_BACKTRACE", "1");
    }

    // Panic hook — runs BEFORE the process aborts (release profile uses
    // panic = "abort", which still invokes set_hook). Without this, the
    // process exits via Windows fail-fast (0xc0000409) with NO trace of
    // what panicked, since the log file's last buffered line is whatever
    // was being written milliseconds before. We log the panic message,
    // location, and backtrace synchronously, then let the abort proceed.
    //
    // Also writes a one-line marker that's easy to grep for: "PANIC".
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".to_string());
        let message = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic payload>".to_string()
        };
        let bt = std::backtrace::Backtrace::force_capture();
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("<unnamed>");
        // Two log writes: a one-line marker for fast grep, then a multi-line
        // block with location + backtrace for forensics.
        crate::log::error(&format!(
            "PANIC: thread='{}' at {} :: {}",
            thread_name, location, message
        ));
        crate::log::error(&format!("PANIC backtrace:\n{}", bt));
        // Best-effort flush to stderr too in case logging is broken.
        eprintln!(
            "\n--- PANIC ---\nthread='{}' at {}\nmessage: {}\n{}\n",
            thread_name, location, message, bt
        );
    }));
    log::info("Panic hook installed (logs to %TEMP%\\agent-session-tui.log)");

    let config = AppConfig::load()?;
    config.write_default_if_missing()?;
    log::info(&format!(
        "Config loaded from {:?}",
        AppConfig::config_path()
    ));

    // --mock-data: launch with a curated synthetic session list for demo GIFs
    // and screenshots. The data lives in the gitignored `mock-plugin/` workspace
    // member; without that DLL dropped next to the exe, this flag is a silent
    // no-op so end-users of distributed builds never see mock behaviour.
    let args: Vec<String> = std::env::args().collect();
    let mock_flag = args.iter().any(|a| a == "--mock-data");
    let mock_plugin = if mock_flag { mock::MockPlugin::try_load() } else { None };
    let mock_data = mock_plugin.is_some();
    if mock_flag && !mock_data {
        log::warn(
            "--mock-data: mock_data plugin DLL not found next to executable; flag ignored",
        );
    }
    if mock_data {
        log::info("MOCK MODE: --mock-data flag detected and plugin loaded; skipping provider registry + semantic preload (supervisor stays alive for resume actions)");
    }

    // Simple JSON archive
    let archive_path = config.data_dir.join("archived.json");
    std::fs::create_dir_all(&config.data_dir)?;
    log::info(&format!("Archive path: {:?}", archive_path));
    let archive = ArchiveStore::open(&archive_path)?;
    let archive = Arc::new(Mutex::new(archive));
    // Spawn the persist worker so archive/unarchive mutations become
    // write-back buffered (coalesces bursts of 'a' presses into one
    // atomic disk write). The supervisor's Shutdown handler is
    // responsible for calling `flush_blocking()` so no buffered state
    // is lost on quit.
    ArchiveStore::spawn_persist_worker(&archive);

    // Session groups (separate from archives). In mock mode, point at a
    // temp file so demo group assignments never write to the real groups.json.
    let groups_path = if mock_data {
        std::env::temp_dir().join("agent-session-tui-mock-groups.json")
    } else {
        config.data_dir.join("groups.json")
    };
    log::info(&format!("Groups path: {:?}", groups_path));
    let group_mgr = groups::GroupManager::open(&groups_path);

    // Build provider registry — skipped entirely in mock mode.
    let mut registry = ProviderRegistry::new();
    let mut enabled_keys = Vec::new();

    if !mock_data {
        for (key, provider_config) in &config.providers {
            if !provider_config.enabled {
                continue;
            }
            match create_provider(key, provider_config) {
                Some(provider) => {
                    log::info(&format!("Provider '{}' registered", key));
                    registry.register(provider);
                    enabled_keys.push(key.clone());
                }
                None => {
                    log::warn(&format!("Unknown provider '{}' in config — skipping", key));
                }
            }
        }
    } else {
        // Mock mode: synthesize the enabled-keys list from what providers the
        // mock dataset uses, so the title-bar provider count matches reality.
        enabled_keys = vec![
            "copilot".into(), "claude".into(), "codex".into(),
            "qwen".into(), "gemini".into(), "kimi".into(),
        ];
    }

    // --- dump-json comparison hook -----------------------------------------
    // `--dump-json [N]` runs discovery on every registered provider, merges
    // all Session objects, sorts by updated_at desc, and prints the top N
    // (default 20) as JSON. Skips the TUI entirely — used for side-by-side
    // golden comparison vs the legacy branch.
    if let Some(pos) = args.iter().position(|a| a == "--dump-json") {
        let n: usize = args
            .get(pos + 1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(20);
        let mut all: Vec<serde_json::Value> = Vec::new();
        for prov in registry.providers() {
            match prov.discover_sessions() {
                Ok(sessions) => {
                    for s in sessions {
                        all.push(serde_json::to_value(&s).unwrap_or(serde_json::Value::Null));
                    }
                }
                Err(e) => {
                    eprintln!("discover failed for {}: {}", prov.name(), e);
                }
            }
        }
        // Sort newest first by updated_at string (ISO-8601 sorts lexically).
        all.sort_by(|a, b| {
            let ka = a.get("updated_at").and_then(|v| v.as_str()).unwrap_or("");
            let kb = b.get("updated_at").and_then(|v| v.as_str()).unwrap_or("");
            kb.cmp(ka)
        });
        all.truncate(n);
        println!("{}", serde_json::to_string_pretty(&all)?);
        return Ok(());
    }
    // -----------------------------------------------------------------------

    // --- search-bench debug hook --------------------------------------------
    // `--search-bench "<query>" [--expect <session_id>] [--top N]` runs the
    // full search pipeline (discovery + log-index refresh + ranked_search)
    // against real disk data and prints the top-N ranked sessions with a
    // tier-by-tier score breakdown. If --expect is given, also prints that
    // session's rank + breakdown even when it falls outside the top-N.
    //
    // This is a diagnostic tool — never invoked by the TUI proper. Used to
    // answer "why did this session NOT surface for that query?".
    if let Some(pos) = args.iter().position(|a| a == "--search-bench") {
        let query = match args.get(pos + 1) {
            Some(q) => q.clone(),
            None => {
                eprintln!("--search-bench requires a query argument");
                return Ok(());
            }
        };
        let expect_id: Option<String> = args
            .iter()
            .position(|a| a == "--expect")
            .and_then(|i| args.get(i + 1).cloned());
        let top_n: usize = args
            .iter()
            .position(|a| a == "--top")
            .and_then(|i| args.get(i + 1).and_then(|s| s.parse().ok()))
            .unwrap_or(20);
        run_search_bench(&registry, &config, &query, expect_id.as_deref(), top_n)?;
        return Ok(());
    }
    // -----------------------------------------------------------------------

    // --- search-eval IR benchmark -------------------------------------------
    // `--search-eval [--queries path/to/queries.toml] [--report out.json]`
    // runs a labeled query set through the same pipeline and reports
    // MRR / P@1 / Recall@K aggregated overall and per category. Default
    // queries file is `eval/search-queries.toml` (gitignored) with fallback
    // to `eval/search-queries.example.toml`.
    if args.iter().any(|a| a == "--search-eval") {
        let queries: Option<String> = args
            .iter()
            .position(|a| a == "--queries")
            .and_then(|i| args.get(i + 1).cloned());
        let report: Option<String> = args
            .iter()
            .position(|a| a == "--report")
            .and_then(|i| args.get(i + 1).cloned());
        search_eval::run_search_eval(&registry, &config, queries.as_deref(), report.as_deref())?;
        return Ok(());
    }
    // -----------------------------------------------------------------------

    let registry = Arc::new(registry);

    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();

    // Spawn the supervisor in BOTH normal and mock mode. In mock mode the
    // ProviderRegistry is empty (built above with `if !mock_data`) so the
    // periodic scan finds zero providers, sends no SessionsUpdated events,
    // and never clobbers the preloaded mock data. We DO pass real
    // `config.providers` (the HashMap with command/resume_flag for each
    // provider) so `handle_resume` can build `copilot --resume <id>` and
    // launch a real terminal when the user presses Enter/r on a mock row
    // that was wired to a real session id by `mock::mock_sessions`.
    let supervisor = Supervisor::new(
        Arc::clone(&registry),
        Arc::clone(&archive),
        config.poll_interval_ms,
        config.providers.clone(),
    );
    let supervisor_handle = Some(tokio::spawn(async move {
        supervisor.run(event_tx, cmd_rx).await;
    }));

    // Resolve default provider: find the one with default=true, else first enabled
    let default_provider = config
        .providers
        .iter()
        .find(|(k, v)| v.enabled && v.default && enabled_keys.contains(k))
        .map(|(k, _)| k.clone())
        .or_else(|| enabled_keys.first().cloned())
        .unwrap_or_default();

    // Preload the semantic plugin BEFORE entering the TUI so fastembed's first-run
    // model download progress bar renders on the normal shell (not corrupting the
    // TUI's alternate screen). On subsequent runs the model is cached and this
    // returns in milliseconds.
    //
    // Skipped entirely in mock mode — semantic search has no embeddings to
    // compare against synthetic data, and we don't want to trigger a 550 MB
    // model download when somebody is just trying to record a GIF.
    let semantic = std::sync::Arc::new(std::sync::Mutex::new(
        search::SemanticPlugin::new(),
    ));
    if !mock_data {
        let cache_dir = dirs::data_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("agent-session-tui")
            .join("models");
        std::fs::create_dir_all(&cache_dir).ok();
        let needs_download = !cache_dir
            .join("models--nomic-ai--nomic-embed-text-v1.5")
            .exists();
        if needs_download {
            eprintln!("Preparing semantic search model (first-run download, ~550 MB)...");
        }
        if let Ok(mut plugin) = semantic.lock() {
            plugin.try_load(&cache_dir.to_string_lossy());
        }
        if needs_download {
            eprintln!("Semantic model ready. Starting TUI...");
        }
    }

    let mut app = App::new(
        enabled_keys,
        default_provider,
        config.log_max_lines,
        Arc::clone(&registry),
        config.data_dir.clone(),
        semantic,
        config.tick_rate_ms,
        config.semantic_index_min_interval_ms,
        group_mgr,
        config.acp.clone(),
    );

    // In mock mode, populate the TUI with the curated demo dataset.
    if mock_data {
        if let Some(p) = mock_plugin.as_ref() {
            app.preload_demo_data(p.sessions());
            app.preload_demo_groups(p.group_assignments());
            app.preload_demo_suggestions(p.auto_suggestions());
        }
    }

    app.run(event_rx, cmd_tx).await?;

    // The UI loop just sent `SupervisorCommand::Shutdown` before returning.
    // That command sits in the mpsc channel BEHIND any still-unprocessed
    // `ArchiveSession` / `UnarchiveSession` commands that the user queued
    // via rapid 'a' presses before quitting. If we exited the process
    // right here, those pending archive writes would be lost — archives
    // persisted on disk only once `handle_archive` (synchronous
    // `fs::write`) runs, and that only happens when the supervisor task
    // dequeues the command.
    //
    // Awaiting `supervisor_handle` drains the channel in FIFO order,
    // persists every queued archive, and finally exits on `Shutdown`.
    // A 5-second cap guards against a pathologically stuck supervisor
    // (e.g. a provider taking forever) so the user never loses more than
    // a moment after pressing 'q'.
    //
    // In mock mode there is no supervisor — skip the wait entirely.
    if let Some(handle) = supervisor_handle {
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            handle,
        )
        .await;
    }

    // After the supervisor has drained, the only things still keeping
    // the process alive are detached std::thread workers (scan threads
    // and the semantic indexer holding the 550MB embedding model).
    // They hold no unflushed state — the embed cache re-warms next
    // launch, scans are read-only — so force-exit instead of waiting
    // the multiple seconds they might take to unwind naturally.
    std::process::exit(0);
}

/// Run a single search query against real on-disk sessions and print a
/// tier-by-tier score breakdown for the top-N results plus (optionally)
/// a target session-id so we can see WHY it ranked where it did.
///
/// Invoked via `agent-session-tui --search-bench "<query>"`.
/// Optional flags: `--expect <session_id>`, `--top N`.
fn run_search_bench(
    registry: &ProviderRegistry,
    config: &AppConfig,
    query: &str,
    expect_id: Option<&str>,
    top_n: usize,
) -> Result<()> {
    use search::{score_breakdown, SemanticPlugin};

    println!("Query: {:?}", query);
    println!("Expect: {}", expect_id.unwrap_or("(none)"));
    println!();

    // Discover sessions across all providers — same path as the live TUI.
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

    // Refresh log index against real data so BM25 reflects reality.
    println!("Refreshing log index (tantivy)...");
    let start = std::time::Instant::now();
    let searcher = log_search::LogSearcher::open_or_create(&config.data_dir)?;
    if let Err(e) = searcher.refresh(&all_sessions, registry) {
        eprintln!("  refresh error (continuing with whatever's indexed): {:#}", e);
    }
    println!("  refresh: {:?}", start.elapsed());

    // Run the log query — note whether AND-first hit or OR-fallback kicked in.
    println!();
    println!("Running log_searcher.search()...");
    let log_matches = searcher.search(query);
    println!("  BM25 hits: {}", log_matches.len());
    println!();

    // Load semantic plugin if available, for visibility into cosine sims.
    let cache_dir = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("agent-session-tui")
        .join("models");
    let mut sem = SemanticPlugin::new();
    sem.try_load(&cache_dir.to_string_lossy());
    let sem_ready = sem.is_ready();
    println!("Semantic plugin ready: {}", sem_ready);
    let sem_scores: std::collections::HashMap<String, f32> = if sem_ready && query.len() >= 5 {
        sem.search_cached(query, 0.0).into_iter().collect()
    } else {
        std::collections::HashMap::new()
    };
    println!("  semantic hits (any sim): {}", sem_scores.len());
    println!();

    // Compute breakdowns for every session — slower but clearer.
    let query_lower = query.to_lowercase();
    let query_words: Vec<&str> = query_lower.split_whitespace().collect();
    let mut all_results: Vec<(usize, search::ScoreBreakdown)> = all_sessions
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let log_score = log_matches.get(&s.id).copied();
            let sem = sem_scores.get(&s.id).copied().unwrap_or(0.0);
            let bd = score_breakdown(s, &query_lower, &query_words, log_score, sem);
            (i, bd)
        })
        .collect();
    all_results.sort_by_key(|(_, bd)| std::cmp::Reverse(bd.final_score));

    // Top-N display.
    println!("─── Top {} results ───", top_n.min(all_results.len()));
    println!(
        "{:>4}  {:>6}  {:>4}  {:<10}  {:<46}  why",
        "rank", "score", "rec%", "state", "title"
    );
    for (rank, (idx, bd)) in all_results.iter().take(top_n).enumerate() {
        let s = &all_sessions[*idx];
        let title = util::truncate_str_safe(&s.title, 44);
        let state_label = s.state.label();
        let why = score_why(bd);
        let recency_pct = (bd.recency * 100.0) as u32;
        println!(
            "{:>4}  {:>6}  {:>3}%  {:<10}  {:<46}  {}",
            rank + 1,
            bd.final_score,
            recency_pct,
            state_label,
            title,
            why
        );
    }
    println!();

    // Targeted breakdown for an --expect session ID.
    if let Some(target) = expect_id {
        println!("─── Expected target: {} ───", target);
        let pos = all_results
            .iter()
            .position(|(i, _)| all_sessions[*i].provider_session_id == target);
        match pos {
            Some(p) => {
                let (idx, bd) = &all_results[p];
                let s = &all_sessions[*idx];
                println!("  Found at rank: {}", p + 1);
                println!("  Title: {:?}", s.title);
                println!("  Summary (first 120): {:?}", util::truncate_str_safe(&s.summary, 120));
                println!("  Updated: {}", s.updated_at);
                println!("  Final score: {} (recency={:.2})", bd.final_score, bd.recency);
                println!("  Best pre-recency: {}", bd.best_pre_recency);
                println!();
                println!("  Field tier scores:");
                for fs in &bd.field_scores {
                    println!(
                        "    {:<10} base={:<4} exact={:<4} all_words={:<4} partial={:<4} word_hits={}",
                        fs.label, fs.base, fs.exact, fs.all_words, fs.partial, fs.word_hits
                    );
                }
                println!();
                println!("  BM25 raw: {:.3}  →  bonus={}", bd.bm25_raw, bd.bm25_bonus);
                println!(
                    "  Semantic sim: {:.3}  →  boost={}",
                    bd.semantic_sim, bd.semantic_boost
                );
                println!("  State label bonus: {}", bd.state_label_bonus);
            }
            None => {
                println!("  *** NOT FOUND in {} discovered sessions ***", all_sessions.len());
                println!("  (Check: is this session in an archived state? is the provider enabled?)");
            }
        }
    }
    Ok(())
}

/// One-line explanation of which tiers contributed to a score, for the
/// top-N display. Looks like: "title:partial bm25=8.4(672) sem=0.45(+16) rec×0.90"
fn score_why(bd: &search::ScoreBreakdown) -> String {
    let mut parts: Vec<String> = Vec::new();
    for fs in &bd.field_scores {
        if fs.exact > 0 {
            parts.push(format!("{}:exact({})", fs.label, fs.exact));
        } else if fs.all_words > 0 {
            parts.push(format!("{}:all_words({})", fs.label, fs.all_words));
        } else if fs.partial > 0 {
            parts.push(format!("{}:partial({})", fs.label, fs.partial));
        }
    }
    if bd.bm25_bonus > 0 {
        parts.push(format!("bm25={:.2}({})", bd.bm25_raw, bd.bm25_bonus));
    }
    if bd.semantic_boost > 0 || bd.semantic_sim >= 0.3 {
        parts.push(format!("sem={:.2}(+{})", bd.semantic_sim, bd.semantic_boost));
    }
    if bd.state_label_bonus > 0 {
        parts.push(format!("state(+{})", bd.state_label_bonus));
    }
    if parts.is_empty() {
        return String::from("—");
    }
    parts.join(" ")
}
