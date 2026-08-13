use agent_session_tui::config::AppConfig;
use agent_session_tui::testing::TestRunner;
use agent_session_tui::testing::{load_provider, scenarios};

#[test]
fn kimi_lifecycle() {
    let config = AppConfig::load().expect("config");
    let pc = config.providers.get("kimi").expect("'kimi' not in config");
    let provider = load_provider("kimi", pc);
    let mut runner = TestRunner::new("Kimi");

    scenarios::discover(&mut runner, &provider);
    scenarios::graceful(&mut runner, &provider);

    assert!(runner.summary(), "Tests failed");
}
