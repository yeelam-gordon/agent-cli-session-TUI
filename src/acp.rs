//! AI group suggestion engine (configured CLI in non-interactive `-p` mode).
//!
//! Spawns the user's configured CLI (default: `copilot`) as a one-shot
//! subprocess via `-p <prompt>` and parses stdout. Despite the file/config
//! name `acp` (kept stable for back-compat), this is **not** the Agent
//! Client Protocol — we don't speak JSON-RPC to a long-lived agent. A real
//! ACP migration could speed up chained auto-suggest batches by avoiding
//! per-call startup overhead, but it isn't currently used.

use std::path::PathBuf;

use serde::Deserialize;

use crate::config::AcpConfig;
use crate::groups::GroupManager;
use crate::models::Session;

/// A single AI suggestion: assign a session to a group.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct AiSuggestion {
    pub session: String,
    pub group: String,
    pub is_new: bool,
    pub score: f64,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize)]
struct AiResponse {
    suggestions: Vec<AiSuggestion>,
}

/// Resolve the prompt template path. Searches:
/// 1. `acp.prompt_template` from config (if set)
/// 2. `<exe-dir>/prompts/group-suggest.md`
/// 3. `<exe-dir>/../prompts/group-suggest.md` (dev layout)
pub fn resolve_template(cfg: &AcpConfig) -> Option<PathBuf> {
    if let Some(ref p) = cfg.prompt_template {
        if p.exists() {
            return Some(p.clone());
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            // <exe-dir>/prompts/ (installed layout)
            let candidate = exe_dir.join("prompts").join("group-suggest.md");
            if candidate.exists() {
                return Some(candidate);
            }
            // <exe-dir>/../prompts/ (one level up)
            if let Some(p1) = exe_dir.parent() {
                let candidate2 = p1.join("prompts").join("group-suggest.md");
                if candidate2.exists() {
                    return Some(candidate2);
                }
                // <exe-dir>/../../prompts/ (dev layout: target/release/../../prompts/)
                if let Some(p2) = p1.parent() {
                    let candidate3 = p2.join("prompts").join("group-suggest.md");
                    if candidate3.exists() {
                        return Some(candidate3);
                    }
                }
            }
        }
    }
    None
}

/// Build the rendered prompt by filling {{groups}} and {{sessions}} placeholders.
fn render_prompt(
    template: &str,
    ungrouped: &[(&Session, String)],
    group_mgr: &GroupManager,
) -> String {
    let all_groups = group_mgr.all_groups_with_descriptions();
    let groups_json = if all_groups.is_empty() {
        "[]".to_string()
    } else {
        let items: Vec<String> = all_groups
            .iter()
            .map(|(name, count, desc)| {
                if let Some(d) = desc {
                    let d_escaped = d.replace('"', r#"\""#);
                    format!(r#"{{"name":"{}","count":{},"description":"{}"}}"#, name, count, d_escaped)
                } else {
                    format!(r#"{{"name":"{}","count":{}}}"#, name, count)
                }
            })
            .collect();
        format!("[{}]", items.join(","))
    };

    let sessions_json = {
        let items: Vec<String> = ungrouped
            .iter()
            .take(30)
            .map(|(s, key)| {
                let title = s.title.replace('"', r#"\""#);
                // Truncate summary to 100 chars to keep prompt compact
                let summary_raw = s.summary.replace('"', r#"\""#);
                let summary: String = summary_raw.chars().take(100).collect();
                let cwd_full = s.cwd.to_string_lossy();
                // Use only last path component for cwd
                let cwd = std::path::Path::new(cwd_full.as_ref())
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| cwd_full.to_string())
                    .replace('"', r#"\""#);
                format!(
                    r#"{{"id":"{}","title":"{}","summary":"{}","cwd":"{}"}}"#,
                    key, title, summary, cwd
                )
            })
            .collect();
        format!("[{}]", items.join(","))
    };

    template
        .replace("{{groups}}", &groups_json)
        .replace("{{sessions}}", &sessions_json)
}

/// Collect ungrouped sessions and build the prompt string.
/// Includes semantic similarity hints if embeddings are available.
/// Returns (prompt_text, session_count) or an error.
/// Collect ungrouped sessions and build the prompt string.
/// Includes semantic similarity hints if embeddings are available.
/// Returns `(prompt_text, asked_keys)` on success.
///
/// `skip_keys` is a set of session keys (`provider:session_id`) that have
/// already been asked in a previous batch — they are excluded so successive
/// auto-suggest runs cover different sessions instead of re-asking the same
/// top 30 every time.
pub fn prepare_prompt(
    cfg: &AcpConfig,
    sessions: &[Session],
    group_mgr: &GroupManager,
    semantic: Option<&crate::search::SemanticPlugin>,
    skip_keys: &std::collections::HashSet<String>,
) -> Result<(String, Vec<String>), String> {
    let template_path = resolve_template(cfg).ok_or_else(|| {
        "AI grouping not set up — see README → AI Auto-Suggest. Missing prompts/group-suggest.md.".to_string()
    })?;
    let template = std::fs::read_to_string(&template_path)
        .map_err(|e| format!("Failed to read template: {}", e))?;

    const BATCH_SIZE: usize = 30;

    let ungrouped: Vec<(&Session, String)> = sessions
        .iter()
        .filter(|s| {
            let key = format!("{}:{}", s.provider_name, s.provider_session_id);
            // Skip already grouped sessions
            if !group_mgr.groups_for(&key).is_empty() {
                return false;
            }
            // Skip sessions the user previously dismissed from ALL groups
            // (dismissed means "don't suggest for now")
            if group_mgr.has_any_dismissal(&key) {
                return false;
            }
            // Skip sessions already asked in a previous batch this run.
            if skip_keys.contains(&key) {
                return false;
            }
            true
        })
        .take(BATCH_SIZE)
        .map(|s| {
            let key = format!("{}:{}", s.provider_name, s.provider_session_id);
            (s, key)
        })
        .collect();

    if ungrouped.is_empty() {
        return Err("No ungrouped sessions to analyze".to_string());
    }

    let asked_keys: Vec<String> = ungrouped.iter().map(|(_, k)| k.clone()).collect();
    let mut prompt = render_prompt(&template, &ungrouped, group_mgr);

    // Add semantic similarity hints if available
    if let Some(sem) = semantic {
        let pairs = sem.pairwise_similarities(&asked_keys, 0.5);
        if !pairs.is_empty() {
            let sim_items: Vec<String> = pairs
                .iter()
                .take(30) // cap to avoid prompt bloat
                .map(|(a, b, sim)| format!(r#"["{}", "{}", {:.2}]"#, a, b, sim))
                .collect();
            prompt.push_str("\n\n## Semantic Similarity (embedding cosine > 0.5)\n\n");
            prompt.push_str(&format!("[{}]", sim_items.join(",")));
            prompt.push_str("\n\nUse these as hints — sessions with high similarity likely belong in the same group.\n");
        }
    }

    Ok((prompt, asked_keys))
}

/// Run the ACP agent subprocess: initialize → session/new → session/prompt.
/// Uses raw JSON-RPC over stdio (newline-delimited) — no SDK, proven to work
/// with `copilot --acp --stdio` via direct pipe testing.
///
/// `session_id` is the UUID copilot will use for its new session (passed via
/// `--resume=<UUID>` which copilot interprets as "start new with this UUID"
/// when no session with that ID exists). Caller is responsible for archiving
/// `copilot:<session_id>` BEFORE invoking this so the spawned session never
/// surfaces in the user's active list.
pub async fn run_acp_suggest(
    cfg: AcpConfig,
    prompt: String,
    session_id: String,
) -> Result<Vec<AiSuggestion>, String> {
    // Use std::process (synchronous) inside spawn_blocking because
    // tokio::process async pipe reads hang on Windows for ACP stdio.
    let result = tokio::task::spawn_blocking(move || {
        run_acp_sync(cfg, prompt, session_id)
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?;
    result
}

/// Synchronous AI suggestion flow — runs on a blocking thread.
/// Uses `copilot -p "<prompt>" -s --resume=<uuid>` so the spawned session
/// has a known-in-advance UUID; the caller pre-archives that UUID so the
/// session never pollutes the active list. The ACP stdio protocol has
/// buffering issues on Windows that make interactive session/prompt unreliable.
fn run_acp_sync(
    cfg: AcpConfig,
    prompt: String,
    session_id: String,
) -> Result<Vec<AiSuggestion>, String> {
    use std::process::{Command, Stdio};

    crate::log::info(&format!("ACP: spawning '{}' with -p mode", cfg.command));
    crate::log::info(&format!("ACP: prompt length = {} chars", prompt.len()));

    // Build a clean env without COPILOT_* vars (avoid recursion when asTUI
    // itself is running inside a Copilot CLI session).
    let clean_env: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| !k.starts_with("COPILOT_"))
        .collect();

    // Build args. The grouping task is pure text-in / JSON-out; the model
    // doesn't need any tools or MCP servers. We pass:
    //   --available-tools=        empty allowlist → model has no tools
    //                             → no permission prompts → safe for -p mode
    //                             → no need for --allow-all-tools
    //   --disable-builtin-mcps    skip github-mcp-server startup
    //
    // We deliberately do NOT pass `--config-dir` or override `COPILOT_HOME`.
    // Whatever auth setup the user has configured for `copilot` is theirs to
    // manage; we don't second-guess it.
    let mut args: Vec<String> = vec![
        "-p".to_string(),
        prompt,
        "-s".to_string(),
        "--available-tools=".to_string(),
        "--disable-builtin-mcps".to_string(),
        // Pre-assign the session UUID so the caller can archive
        // `copilot:<session_id>` BEFORE we spawn — keeps these
        // grouping-helper sessions out of the user's active list.
        // `--resume=<UUID>` against a non-existent UUID means
        // "start a NEW session with this UUID" per copilot --help.
        format!("--resume={}", session_id),
    ];
    for arg in &cfg.extra_args {
        // Filter out ACP-protocol flags that don't apply to -p mode.
        if arg != "--acp" && arg != "--stdio" {
            args.push(arg.clone());
        }
    }

    crate::log::info(&format!(
        "ACP: running {} -p ... -s --available-tools= --disable-builtin-mcps (extra_args={:?})",
        cfg.command, cfg.extra_args
    ));

    let start = std::time::Instant::now();
    let output = Command::new(&cfg.command)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .envs(clean_env)
        .output()
        .map_err(|e| format!("Failed to run '{}': {}", cfg.command, e))?;
    let elapsed = start.elapsed();

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    crate::log::info(&format!(
        "ACP: copilot exited (status={}, elapsed={:.1}s, stdout={}b, stderr={}b)",
        output.status,
        elapsed.as_secs_f64(),
        stdout.len(),
        stderr.len()
    ));

    if !stderr.is_empty() {
        let stderr_preview: String = stderr.chars().take(800).collect();
        crate::log::info(&format!("ACP: stderr preview: {}", stderr_preview));
    }

    if !output.status.success() {
        return Err(format!(
            "Command failed ({}, {:.1}s): stderr={}",
            output.status,
            elapsed.as_secs_f64(),
            stderr.chars().take(400).collect::<String>()
        ));
    }

    let stdout_preview: String = stdout.chars().take(800).collect();
    crate::log::info(&format!("ACP: stdout preview ({} chars total): {}", stdout.len(), stdout_preview));

    match parse_suggestions(&stdout) {
        Ok(s) => {
            crate::log::info(&format!("ACP: parsed {} suggestions", s.len()));
            Ok(s)
        }
        Err(e) => {
            crate::log::warn(&format!(
                "ACP: parse FAILED — full stdout follows ({} chars): {}",
                stdout.len(),
                stdout
            ));
            Err(e)
        }
    }
}

/// Extract suggestions from ACP response text. Tolerant of markdown wrapping.
fn parse_suggestions(output: &str) -> Result<Vec<AiSuggestion>, String> {
    let trimmed = output.trim();

    if let Ok(resp) = serde_json::from_str::<AiResponse>(trimmed) {
        return Ok(resp.suggestions);
    }

    // Look for JSON object in case AI wrapped it in markdown
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            let json_slice = &trimmed[start..=end];
            if let Ok(resp) = serde_json::from_str::<AiResponse>(json_slice) {
                return Ok(resp.suggestions);
            }
        }
    }

    Err(format!(
        "Failed to parse AI response: {}",
        trimmed.chars().take(200).collect::<String>()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_compact_json() {
        let input = r#"{"suggestions":[{"session":"copilot:abc","group":"perf","is_new":false,"score":0.87,"reason":"Benchmark"}]}"#;
        let result = parse_suggestions(input).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].session, "copilot:abc");
        assert_eq!(result[0].group, "perf");
        assert!(!result[0].is_new);
        assert!((result[0].score - 0.87).abs() < 0.01);
    }

    #[test]
    fn parse_json_wrapped_in_markdown() {
        let input = "Here is the result:\n```json\n{\"suggestions\":[{\"session\":\"claude:def\",\"group\":\"new-thing\",\"is_new\":true,\"score\":0.74,\"reason\":\"New project\"}]}\n```\n";
        let result = parse_suggestions(input).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].is_new);
    }

    #[test]
    fn parse_empty_suggestions() {
        let input = r#"{"suggestions":[]}"#;
        let result = parse_suggestions(input).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_garbage_returns_error() {
        let input = "I don't understand your question.";
        assert!(parse_suggestions(input).is_err());
    }
}
