pub mod scenarios;

use std::time::Duration;

use crate::config::ProviderConfig;
use crate::provider::config_driven::ConfigDrivenProvider;

// ───────────────────────────────────────────────────────────────────────────
// Provider construction for integration tests
// ───────────────────────────────────────────────────────────────────────────

/// Build the provider for `key` from the crate's shipped `providers/<key>.yaml`.
///
/// The hand-written per-provider modules (`provider::copilot::CopilotProvider`
/// and friends) were replaced by the YAML-driven engine, which left every
/// `tests/*_lifecycle_test.rs` importing types that no longer exist. CI only
/// runs `cargo test --lib`, so that breakage went unnoticed — plain
/// `cargo test` failed to compile. Tests now go through this helper so a
/// future provider-loading change breaks in exactly one place.
///
/// Resolves against `CARGO_MANIFEST_DIR` so it works regardless of the
/// test binary's location or the current working directory.
pub fn load_provider(key: &str, cfg: &ProviderConfig) -> ConfigDrivenProvider {
    let yaml = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("providers")
        .join(format!("{key}.yaml"));
    assert!(
        yaml.exists(),
        "providers/{key}.yaml not found at {yaml:?} — provider definitions ship \
         with the crate and are required by the lifecycle tests"
    );
    ConfigDrivenProvider::load_from_yaml(&yaml, cfg)
        .unwrap_or_else(|e| panic!("failed to load providers/{key}.yaml: {e}"))
}

// ───────────────────────────────────────────────────────────────────────────
// Common test runner — shared by all provider lifecycle tests
// ───────────────────────────────────────────────────────────────────────────

pub struct TestRunner {
    pub provider_name: String,
    results: Vec<TestResult>,
}

pub struct TestResult {
    pub name: String,
    pub passed: bool,
    pub message: String,
    pub duration: Duration,
}

impl TestRunner {
    pub fn new(provider_name: &str) -> Self {
        Self {
            provider_name: provider_name.to_string(),
            results: Vec::new(),
        }
    }

    pub fn record(&mut self, name: &str, pass: bool, msg: &str, dur: Duration) {
        let tag = if pass { "PASS" } else { "FAIL" };
        println!("  [{tag}] {name} ({:.1}s) — {msg}", dur.as_secs_f64());
        self.results.push(TestResult {
            name: name.to_string(),
            passed: pass,
            message: msg.to_string(),
            duration: dur,
        });
    }

    /// Print summary and return true if all passed.
    pub fn summary(&self) -> bool {
        println!("\n============================================================");
        let total = self.results.len();
        let passed = self.results.iter().filter(|r| r.passed).count();
        let failed = total - passed;
        println!(
            "{} provider: {passed}/{total} passed, {failed} failed\n",
            self.provider_name
        );
        for r in &self.results {
            let icon = if r.passed { "✓" } else { "✗" };
            println!("  {icon} {} — {}", r.name, r.message);
        }
        failed == 0
    }
}

/// Truncate a string at a char boundary for display.
pub fn trunc(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        let mut e = n;
        while e > 0 && !s.is_char_boundary(e) {
            e -= 1;
        }
        format!("{}…", &s[..e])
    }
}
