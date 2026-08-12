use agent_session_tui::config::AppConfig;
use agent_session_tui::provider::config_driven::ConfigDrivenProvider;
use agent_session_tui::testing::scenarios;
use agent_session_tui::testing::TestRunner;
use std::path::Path;

#[test]
fn kimi_lifecycle() {
    let config = AppConfig::load().expect("config");
    let pc = config.providers.get("kimi").expect("'kimi' not in config");
    let yaml = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("providers")
        .join("kimi.yaml");
    let provider =
        ConfigDrivenProvider::load_from_yaml(&yaml, pc).expect("load kimi provider yaml");
    let mut runner = TestRunner::new("Kimi");

    scenarios::discover(&mut runner, &provider);
    scenarios::graceful(&mut runner, &provider);

    assert!(runner.summary(), "Tests failed");
}
