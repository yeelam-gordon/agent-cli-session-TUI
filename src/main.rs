mod archive;
mod config;
mod focus;
mod log;
mod models;
mod process_info;
mod provider;
mod search;
mod supervisor;
mod ui;
mod util;

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
    tokio::spawn(async move {
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

    let app = App::new(enabled_keys, default_provider, config.log_max_lines);
    app.run(event_rx, cmd_tx).await?;

    Ok(())
}
