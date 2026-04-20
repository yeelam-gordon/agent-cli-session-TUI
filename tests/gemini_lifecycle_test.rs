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

    // 1. Static Discovery (Offline)
    scenarios::discover(&mut runner, &provider);
    scenarios::graceful(&mut runner, &provider);

    // 2. Live Lifecycle (Requires a real gemini-cli installation)
    // This will test: Launch -> Running (🟢) -> Waiting (🟡) -> Kill -> Resumable (💤)
    // We check for "launch" anywhere in the args because libtest puts its own args first
    if std::env::args().any(|arg| arg == "launch") {
        scenarios::launch(&mut runner, &provider, pc);
    }

    assert!(runner.summary(), "Tests failed");
}
