//! Codex CLI Provider - Lifecycle Detection Tests
//!
//! Uses the common test framework from `src/testing/`.
//! Re-run after any change to `src/provider/codex/`.
//!
//! Run:  cargo test --test codex_lifecycle_test -- --nocapture
//! Args: --scenario discover|launch|kill|graceful|all (default: all)

use agent_session_tui::config::AppConfig;
use agent_session_tui::provider::codex::CodexProvider;
use agent_session_tui::testing::scenarios;
use agent_session_tui::testing::TestRunner;

#[test]
fn codex_lifecycle() {
    let args: Vec<String> = std::env::args().collect();
    let scenario = args
        .iter()
        .position(|a| a == "--scenario")
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
        .unwrap_or("all");

    println!("\n╔══════════════════════════════════════════════════════════╗");
    println!("║  Codex CLI Provider - Lifecycle Detection Tests         ║");
    println!("╚══════════════════════════════════════════════════════════╝");
    println!("Scenario: {scenario}\n");

    let config = AppConfig::load().unwrap_or_default();
    let pc = config
        .providers
        .get("codex")
        .cloned()
        .or_else(|| AppConfig::default().providers.get("codex").cloned())
        .expect("'codex' not in config or defaults");
    let provider = CodexProvider::new(&pc);
    let mut runner = TestRunner::new("Codex");

    match scenario {
        "discover" => scenarios::discover(&mut runner, &provider),
        "launch" => scenarios::launch(&mut runner, &provider, &pc),
        "kill" => scenarios::kill(&mut runner, &provider),
        "graceful" => scenarios::graceful(&mut runner, &provider),
        "all" => {
            scenarios::discover(&mut runner, &provider);
            scenarios::graceful(&mut runner, &provider);
            println!("\n  Interactive: --scenario launch | --scenario kill");
        }
        other => panic!("Unknown scenario: {other}"),
    }

    assert!(runner.summary(), "Some tests failed");
}
