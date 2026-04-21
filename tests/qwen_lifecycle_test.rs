use agent_session_tui::config::ProviderConfig;
use agent_session_tui::provider::qwen::QwenProvider;
use agent_session_tui::testing::TestRunner;
use agent_session_tui::testing::scenarios;

#[test]
fn qwen_lifecycle() {
    let pc = ProviderConfig {
        enabled: true,
        default: false,
        command: "qwen".into(),
        default_args: vec![],
        state_dir: dirs::home_dir().map(|h| h.join(".qwen").join("projects")),
        resume_flag: Some("--resume".into()),
        startup_dir: None,
        launch_method: "wt".into(),
        launch_cmd: None,
        launch_args: None,
        launch_fallback_cmd: None,
        launch_fallback_args: None,
        launch_fallback: None,
        wt_profile: None,
    };
    let provider = QwenProvider::new(&pc);

    let mut runner = TestRunner::new("Qwen CLI");

    scenarios::discover(&mut runner, &provider);
    scenarios::graceful(&mut runner, &provider);

    assert!(runner.summary(), "Tests failed");
}
