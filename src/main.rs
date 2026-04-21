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
use provider::claude::ClaudeProvider;
use provider::codex::CodexProvider;
use provider::copilot::CopilotProvider;
use provider::gemini::GeminiProvider;
use provider::qwen::QwenProvider;
use provider::ProviderRegistry;
use supervisor::Supervisor;
use ui::App;

/// Create a provider instance from a config key.
fn create_provider(
    key: &str,
    config: &config::ProviderConfig,
) -> Option<Box<dyn provider::Provider>> {
    match key {
        "copilot" => Some(Box::new(CopilotProvider::new(config))),
        "claude" => Some(Box::new(ClaudeProvider::new(config))),
        "codex" => Some(Box::new(CodexProvider::new(config))),
        "qwen" => Some(Box::new(QwenProvider::new(config))),
        "gemini" => Some(Box::new(GeminiProvider::new(config))),
        _ => None,
    }
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
