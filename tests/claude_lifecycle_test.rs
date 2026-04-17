//! Claude Code Provider — Lifecycle Detection Tests
//!
//! Uses the common test framework from `src/testing/`.
//! Re-run after any change to `src/provider/claude/`.
//!
//! Run:  cargo test --test claude_lifecycle_test -- --nocapture
//! Args: --scenario discover|launch|kill|graceful|all (default: all)

use agent_session_tui::config::AppConfig;
use agent_session_tui::provider::claude::ClaudeProvider;
use agent_session_tui::testing::TestRunner;
use agent_session_tui::testing::scenarios;

#[test]
fn claude_lifecycle() {
    let args: Vec<String> = std::env::args().collect();
    let scenario = args.iter()
        .position(|a| a == "--scenario")
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
        .unwrap_or("all");

    println!("\n╔══════════════════════════════════════════════════════════╗");
    println!("║  Claude Code Provider — Lifecycle Detection Tests       ║");
    println!("╚══════════════════════════════════════════════════════════╝");
    println!("Scenario: {scenario}\n");

    let config = AppConfig::load().expect("Failed to load config");
    let pc = config.providers.get("claude").expect("'claude' not in config");
    let provider = ClaudeProvider::new(pc);
    let mut runner = TestRunner::new("Claude");

    match scenario {
        "discover" => scenarios::discover(&mut runner, &provider),
        "launch"   => scenarios::launch(&mut runner, &provider, pc),
        "kill"     => scenarios::kill(&mut runner, &provider),
        "graceful" => scenarios::graceful(&mut runner, &provider),
        "all" => {
            scenarios::discover(&mut runner, &provider);
            scenarios::graceful(&mut runner, &provider);
            println!("\n  ℹ Interactive: --scenario launch | --scenario kill");
        }
        other => panic!("Unknown scenario: {other}"),
    }

    assert!(runner.summary(), "Some tests failed");
}

