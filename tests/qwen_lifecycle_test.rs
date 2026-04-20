use agent_session_tui::config::{AppConfig, ProviderConfig};
use agent_session_tui::provider::qwen::QwenProvider;
use agent_session_tui::testing::TestRunner;
use agent_session_tui::testing::scenarios;

#[test]
fn qwen_lifecycle() {
    // Build a minimal config for qwen (doesn't need to be in the user's config.toml)
    let pc = ProviderConfig {
        enabled: true,
        default: false,
        command: "qwen".into(),
        default_args: vec![],
        state_dir: dirs::home_dir().map(|h| h.join(".qwen").join("session-state")),
        resume_flag: Some("--resume".into()),
        startup_dir: None,
        launch_method: "wt".into(),
        wt_profile: None,
    };
    let provider = QwenProvider::new(&pc);

    let mut runner = TestRunner::new("QwenCLI");

    scenarios::discover(&mut runner, &provider);
    scenarios::graceful(&mut runner, &provider);

    assert!(runner.summary(), "Tests failed");
}
