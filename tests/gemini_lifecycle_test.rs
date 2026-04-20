use agent_session_tui::config::AppConfig;
use agent_session_tui::provider::gemini::GeminiProvider;
use agent_session_tui::testing::TestRunner;
use agent_session_tui::testing::scenarios;

#[test]
fn gemini_lifecycle() {
    let config = AppConfig::load().expect("config");
    let pc = config.providers.get("gemini").expect("'gemini' not in config");
    
    let provider = GeminiProvider::new(pc);
    let mut runner = TestRunner::new("Gemini");

    // Common scenarios work with any Provider
    scenarios::discover(&mut runner, &provider);
    scenarios::graceful(&mut runner, &provider);

    assert!(runner.summary(), "Tests failed");
}
