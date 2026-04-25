//! ACP-based AI group suggestion engine.
//!
//! Uses the Agent Client Protocol (agentclientprotocol.com) to communicate
//! with coding agents over JSON-RPC/stdio. The TUI acts as an ACP Client:
//! spawns an agent subprocess, sends initialize → session/new → session/prompt,
//! reads session/update notifications for the response, then parses the
//! compact JSON for group suggestions.

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
fn resolve_template(cfg: &AcpConfig) -> Option<PathBuf> {
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
    let all_groups = group_mgr.all_groups();
    let groups_json = if all_groups.is_empty() {
        "[]".to_string()
    } else {
        let items: Vec<String> = all_groups
            .iter()
            .map(|(name, count)| format!(r#"{{"name":"{}","count":{}}}"#, name, count))
            .collect();
        format!("[{}]", items.join(","))
    };

    let sessions_json = {
        let items: Vec<String> = ungrouped
            .iter()
            .take(20)
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
pub fn prepare_prompt(
    cfg: &AcpConfig,
    sessions: &[Session],
    group_mgr: &GroupManager,
    semantic: Option<&crate::search::SemanticPlugin>,
) -> Result<(String, usize), String> {
    let template_path = resolve_template(cfg)
        .ok_or_else(|| "Prompt template not found (prompts/group-suggest.md)".to_string())?;
    let template = std::fs::read_to_string(&template_path)
        .map_err(|e| format!("Failed to read template: {}", e))?;

    let ungrouped: Vec<(&Session, String)> = sessions
        .iter()
        .filter(|s| {
            let key = format!("{}:{}", s.provider_name, s.provider_session_id);
            group_mgr.groups_for(&key).is_empty()
        })
        .take(20)
        .map(|s| {
            let key = format!("{}:{}", s.provider_name, s.provider_session_id);
            (s, key)
        })
        .collect();

    if ungrouped.is_empty() {
        return Err("No ungrouped sessions to analyze".to_string());
    }

    let count = ungrouped.len();
    let mut prompt = render_prompt(&template, &ungrouped, group_mgr);

    // Add semantic similarity hints if available
    if let Some(sem) = semantic {
        let keys: Vec<String> = ungrouped.iter().map(|(_, k)| k.clone()).collect();
        let pairs = sem.pairwise_similarities(&keys, 0.5);
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

    Ok((prompt, count))
}

/// Run the ACP agent subprocess: initialize → session/new → session/prompt.
/// Uses raw JSON-RPC over stdio (newline-delimited) — no SDK, proven to work
/// with `copilot --acp --stdio` via direct pipe testing.
pub async fn run_acp_suggest(
    cfg: AcpConfig,
    prompt: String,
) -> Result<Vec<AiSuggestion>, String> {
    // Use std::process (synchronous) inside spawn_blocking because
    // tokio::process async pipe reads hang on Windows for ACP stdio.
    let result = tokio::task::spawn_blocking(move || {
        run_acp_sync(cfg, prompt)
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?;
    result
}

/// Synchronous AI suggestion flow — runs on a blocking thread.
/// Uses `copilot -p "<prompt>" -s --allow-all-tools` for reliable one-shot
/// prompt→response. The ACP stdio protocol has buffering issues on Windows
/// that make interactive session/prompt unreliable.
fn run_acp_sync(
    cfg: AcpConfig,
    prompt: String,
) -> Result<Vec<AiSuggestion>, String> {
    use std::process::{Command, Stdio};

    crate::log::info(&format!("ACP: spawning '{}' with -p mode", cfg.command));
    crate::log::info(&format!("ACP: prompt length = {} chars", prompt.len()));

    // Build a clean env without COPILOT_* vars (avoid nesting detection)
    let clean_env: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| !k.starts_with("COPILOT_"))
        .collect();

    // Create a temp config dir with empty MCP config to skip MCP server
    // startup (~300s overhead). The AI grouping task doesn't need any MCP tools.
    let tmp_cfg = std::env::temp_dir().join("agent-session-tui-acp-cfg");
    let _ = std::fs::create_dir_all(&tmp_cfg);
    let _ = std::fs::write(tmp_cfg.join("mcp-config.json"), "{}");

    // Build args: -p "<prompt>" -s --allow-all-tools --config-dir <tmp>
    let mut args: Vec<String> = vec![
        "-p".to_string(),
        prompt,
        "-s".to_string(),
        "--allow-all-tools".to_string(),
        "--config-dir".to_string(),
        tmp_cfg.to_string_lossy().to_string(),
    ];
    // Add extra args (e.g., --model gpt-4o-mini for cost control)
    // but filter out ACP-specific flags
    for arg in &cfg.extra_args {
        if arg != "--acp" && arg != "--stdio" {
            args.push(arg.clone());
        }
    }

    crate::log::info(&format!("ACP: running {} -p ... -s --allow-all-tools --config-dir {:?}", cfg.command, tmp_cfg));

    let output = Command::new(&cfg.command)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env_clear()
        .envs(clean_env)
        .output()
        .map_err(|e| format!("Failed to run '{}': {}", cfg.command, e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Command failed ({}): {}", output.status, &stderr.chars().take(200).collect::<String>()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    crate::log::info(&format!("ACP: response ({} chars): {}", stdout.len(), &stdout.chars().take(500).collect::<String>()));

    parse_suggestions(&stdout)
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
