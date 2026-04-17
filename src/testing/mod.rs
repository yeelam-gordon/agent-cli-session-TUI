pub mod scenarios;

use std::time::Duration;

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
