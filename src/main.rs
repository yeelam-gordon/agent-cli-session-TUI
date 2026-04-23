mod archive;
mod config;
mod focus;
mod log;
mod log_search;
mod models;
mod process_info;
mod provider;
mod search;
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
    log::init(log_path);
    log::info("=== agent-session-tui starting ===");

    let config = AppConfig::load()?;
    config.write_default_if_missing()?;
    log::info(&format!(
        "Config loaded from {:?}",
        AppConfig::config_path()
    ));

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

    // Build provider registry
    let mut registry = ProviderRegistry::new();
    let mut enabled_keys = Vec::new();

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

    // --- dump-json comparison hook -----------------------------------------
    // `--dump-json [N]` runs discovery on every registered provider, merges
    // all Session objects, sorts by updated_at desc, and prints the top N
    // (default 20) as JSON. Skips the TUI entirely — used for side-by-side
    // golden comparison vs the legacy branch.
    let args: Vec<String> = std::env::args().collect();
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

    let registry = Arc::new(registry);

    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();

    let supervisor = Supervisor::new(
        Arc::clone(&registry),
        Arc::clone(&archive),
        config.poll_interval_ms,
        config.providers.clone(),
    );
    let supervisor_handle = tokio::spawn(async move {
        supervisor.run(event_tx, cmd_rx).await;
    });

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
    let semantic = std::sync::Arc::new(std::sync::Mutex::new(
        search::SemanticPlugin::new(),
    ));
    {
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

    let app = App::new(
        enabled_keys,
        default_provider,
        config.log_max_lines,
        Arc::clone(&registry),
        config.data_dir.clone(),
        semantic,
        config.tick_rate_ms,
        config.semantic_index_min_interval_ms,
    );
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
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        supervisor_handle,
    )
    .await;

    // After the supervisor has drained, the only things still keeping
    // the process alive are detached std::thread workers (scan threads
    // and the semantic indexer holding the 550MB embedding model).
    // They hold no unflushed state — the embed cache re-warms next
    // launch, scans are read-only — so force-exit instead of waiting
    // the multiple seconds they might take to unwind naturally.
    std::process::exit(0);
}
