//! Client for a hosted tab auto-grouping service.
//!
//! **This endpoint is undocumented and not a published third-party API.** There
//! is no contract, SLA, stable schema, or data-handling terms, and it can change
//! or disappear without notice. Every call path therefore degrades to the local
//! engine on failure, so grouping never breaks.
//!
//! ## What is sent
//!
//! Session titles and working directories (as `file:///` URIs), for one
//! representative per local cluster — near-duplicate sessions collapse to a
//! single entry first. The service is Microsoft-hosted, so this data leaves the
//! machine; that is the trade for the group names it returns.
//!
//! The working directory is included because it measurably improves grouping:
//! the service was built for browser tabs, where the URL is a strong signal, and
//! it uses this field the same way. On 40 real sessions it merged two
//! arbitrarily-split groups into one and produced better names.
//!
//! Summaries, file contents, and chat transcripts are never sent.
//!
//! ## Observed protocol
//!
//! Verified empirically against the live service:
//!
//! - Success is `200` with `Content-Type: text/event-stream`, a newline-
//!   delimited sequence of JSON objects, terminated by a literal `["DONE"]`.
//! - Each line re-sends the **entire cumulative group list**, not a delta —
//!   only the last data line matters.
//! - Errors are **never** streamed and never terminate with `["DONE"]`. They
//!   are plain JSON/text with a real status code (`400`, `405`, `500`, and a
//!   non-standard `667` for some missing required fields). Parsing must
//!   therefore be gated behind a status check.
//! - An input item with no thematic peers may be **silently omitted** from the
//!   response, so results must be reconciled against the input set.

use serde::{Deserialize, Serialize};

use crate::acp::AiSuggestion;

const ENDPOINT: &str =
    "https://edge.microsoft.com/taggrouptitlegeneration/api/AutoGrouping/groupingacstreaming";

/// Stream terminator emitted after the final data line on success.
const DONE_SENTINEL: &str = r#"["DONE"]"#;

/// One tab-like item in the request. Field names match the service exactly.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TargetTab {
    group_id: String,
    group_name: String,
    opener_tab_id: String,
    tab_id: String,
    title: String,
    /// Always empty — see the privacy note in the module docs.
    url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GroupingRequest {
    /// Must be present; an empty string is accepted. Omitting it is rejected.
    experiment_id: String,
    language: String,
    target_group: Vec<TargetTab>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResultGroup {
    #[allow(dead_code)]
    group_id: String,
    group_name: String,
    tab_id_list: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GroupingResponse {
    resulting_groups_list: Vec<ResultGroup>,
}

/// One session offered to the service for grouping.
#[derive(Debug, Clone)]
pub struct RemoteInput {
    /// Session key (`provider:session_id`) this item stands for.
    pub session_key: String,
    /// Title text to classify.
    pub title: String,
    /// Working directory, sent as a `file:///` URI.
    ///
    /// The service was built for browser tabs, where the URL is a strong
    /// grouping signal, and it uses this the same way. Measured on 40 real
    /// sessions: supplying it merged two arbitrarily-split benchmark groups
    /// into one and produced better names ("Benchmark Analysis" vs
    /// "Benchmark Prompts" + "Benchmark Prompts Alt").
    pub cwd_uri: Option<String>,
    /// Existing group this session already belongs to, if any.
    ///
    /// Supplying it makes the service **preserve that group verbatim** and fold
    /// newly-submitted matching items into it rather than inventing a new name.
    /// This is how "assign into an existing group" is expressed.
    pub existing_group: Option<String>,
}

/// Convert a filesystem path to a `file:///` URI.
///
/// Windows `D:\Demo\my app` becomes `file:///D:/Demo/my%20app`; POSIX
/// `/home/u/x` becomes `file:///home/u/x`. Characters that would break URI
/// parsing are percent-encoded; `/`, `:`, `.`, `-`, `_`, and `~` are kept so
/// the result stays readable and the service can tokenize it.
pub fn path_to_file_uri(path: &std::path::Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file:///");
    // A POSIX path already starts with `/`; don't double it.
    let body = s.strip_prefix('/').unwrap_or(&s);
    for ch in body.chars() {
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '/' | ':' | '.' | '-' | '_' | '~' => out.push(ch),
            _ => {
                let mut buf = [0u8; 4];
                for b in ch.encode_utf8(&mut buf).as_bytes() {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
        }
    }
    out
}

/// Extract the final cumulative snapshot from a successful response body.
///
/// Returns `Ok(None)` when the stream carried no data lines. Tolerates a
/// missing `["DONE"]` sentinel (truncated stream) by using the last line that
/// parses, so a partial result is still usable.
fn parse_stream(body: &str) -> Result<Option<Vec<ResultGroup>>, String> {
    let mut last: Option<Vec<ResultGroup>> = None;
    let mut saw_line = false;
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line == DONE_SENTINEL {
            continue;
        }
        saw_line = true;
        // Superseded snapshots are expected; keep the newest that parses.
        if let Ok(resp) = serde_json::from_str::<GroupingResponse>(line) {
            last = Some(resp.resulting_groups_list);
        }
    }
    if last.is_none() && saw_line {
        return Err("no parseable group snapshot in response stream".to_string());
    }
    Ok(last)
}

/// Convert the service's group→tabs shape into per-session suggestions.
///
/// `inputs` is indexed by `tabId`, which we assign as the 1-based position.
/// Items the service omitted are simply absent from the result — callers treat
/// them as "leave ungrouped".
fn to_suggestions(
    groups: &[ResultGroup],
    inputs: &[RemoteInput],
    existing_groups: &[String],
) -> Vec<AiSuggestion> {
    let mut out = Vec::new();
    for g in groups {
        let name = g.group_name.trim();
        if name.is_empty() {
            continue;
        }
        let is_new = !existing_groups.iter().any(|e| e.eq_ignore_ascii_case(name));
        for tab_id in &g.tab_id_list {
            // tabId is 1-based; guard against anything the service echoes back
            // that we did not send.
            let idx = match tab_id.parse::<usize>() {
                Ok(n) if n >= 1 && n <= inputs.len() => n - 1,
                _ => continue,
            };
            out.push(AiSuggestion {
                session: inputs[idx].session_key.clone(),
                group: name.to_string(),
                is_new,
                // The service returns no confidence score. Use a fixed,
                // deliberately non-authoritative value — suggestions are
                // reviewed by the user before being applied.
                score: 0.75,
                reason: "Auto-grouping service".to_string(),
            });
        }
    }
    out
}

/// Build the request body for `inputs`.
fn build_request(inputs: &[RemoteInput], language: &str) -> GroupingRequest {
    // Stable synthetic ids for pre-existing groups, so members of the same
    // existing group share a groupId and the service preserves it.
    let mut group_ids: Vec<&str> = Vec::new();
    let target_group = inputs
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let (group_id, group_name) = match item.existing_group.as_deref() {
                Some(name) => {
                    let pos = match group_ids.iter().position(|g| *g == name) {
                        Some(p) => p,
                        None => {
                            group_ids.push(name);
                            group_ids.len() - 1
                        }
                    };
                    // Offset so ids never collide with the "-1" sentinel.
                    ((pos + 1).to_string(), name.to_string())
                }
                None => ("-1".to_string(), String::new()),
            };
            TargetTab {
                group_id,
                group_name,
                opener_tab_id: "-1".to_string(),
                tab_id: (i + 1).to_string(),
                title: item.title.clone(),
                // Empty when no cwd is known — the key must still be present,
                // since omitting it makes the service return HTTP 500.
                url: item.cwd_uri.clone().unwrap_or_default(),
            }
        })
        .collect();

    GroupingRequest {
        experiment_id: String::new(),
        language: language.to_string(),
        target_group,
    }
}

/// Call the Remote grouping service. Blocking — run it on a blocking thread.
///
/// Returns an error on any transport failure or non-2xx status; callers are
/// expected to fall back to the local engine rather than surface a hard failure.
pub fn suggest(
    inputs: &[RemoteInput],
    existing_groups: &[String],
    language: &str,
    timeout_secs: u64,
) -> Result<Vec<AiSuggestion>, String> {
    if inputs.is_empty() {
        // An empty targetGroup is rejected by the service; don't bother asking.
        return Ok(Vec::new());
    }

    let req = build_request(inputs, language);
    let body = serde_json::to_string(&req).map_err(|e| format!("serialize failed: {e}"))?;

    crate::log::info(&format!(
        "Remote grouping: POST {} items ({} bytes)",
        inputs.len(),
        body.len()
    ));

    let start = std::time::Instant::now();
    // ureq 3 defaults to the Rustls provider even when only `native-tls` is
    // compiled in, and panics at request time. Select the provider explicitly.
    let agent = ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .build(),
        )
        .timeout_global(Some(std::time::Duration::from_secs(timeout_secs)))
        .build()
        .new_agent();

    let mut resp = agent
        .post(ENDPOINT)
        .header("Content-Type", "application/json")
        .send(&body)
        .map_err(|e| format!("Remote grouping request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(format!("Remote grouping returned HTTP {status}"));
    }

    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("Remote grouping read failed: {e}"))?;

    let groups = parse_stream(&text)?.unwrap_or_default();
    let suggestions = to_suggestions(&groups, inputs, existing_groups);

    crate::log::info(&format!(
        "Remote grouping: {} groups → {} suggestions in {:.1}s",
        groups.len(),
        suggestions.len(),
        start.elapsed().as_secs_f64()
    ));

    Ok(suggestions)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(v: &[(&str, &str)]) -> Vec<RemoteInput> {
        v.iter()
            .map(|(k, t)| RemoteInput {
                session_key: k.to_string(),
                title: t.to_string(),
                cwd_uri: None,
                existing_group: None,
            })
            .collect()
    }

    #[test]
    fn windows_path_becomes_a_file_uri() {
        let uri = path_to_file_uri(std::path::Path::new(r"D:\Demo\agent-session-tui"));
        assert_eq!(uri, "file:///D:/Demo/agent-session-tui");
    }

    #[test]
    fn posix_path_does_not_get_a_doubled_slash() {
        let uri = path_to_file_uri(std::path::Path::new("/home/u/proj"));
        assert_eq!(uri, "file:///home/u/proj");
    }

    /// Spaces and other unsafe characters must be percent-encoded, or the
    /// service sees a malformed URI.
    #[test]
    fn unsafe_characters_are_percent_encoded() {
        let uri = path_to_file_uri(std::path::Path::new(r"D:\My Docs\a?b#c"));
        assert!(!uri.contains(' '), "raw space must not survive: {uri}");
        assert!(uri.contains("%20"), "space must be encoded: {uri}");
        assert!(!uri.contains('?'), "raw ? must not survive: {uri}");
        assert!(!uri.contains('#'), "raw # must not survive: {uri}");
    }

    #[test]
    fn non_ascii_paths_are_encoded_without_panicking() {
        let uri = path_to_file_uri(std::path::Path::new(r"D:\项目\café"));
        assert!(uri.starts_with("file:///D:/"));
        assert!(uri.contains('%'));
        assert!(uri.is_ascii(), "URI must be ASCII-safe: {uri}");
    }

    /// The cwd URI is what the service uses as its strongest grouping signal,
    /// so it must actually reach the request body.
    #[test]
    fn request_carries_the_cwd_uri() {
        let inp = vec![RemoteInput {
            session_key: "p:1".into(),
            title: "work".into(),
            cwd_uri: Some("file:///D:/Demo/proj".into()),
            existing_group: None,
        }];
        let req = build_request(&inp, "en-US");
        assert_eq!(req.target_group[0].url, "file:///D:/Demo/proj");
    }

    /// Sessions with no known cwd must still serialize a `url` key — omitting
    /// it entirely makes the service return HTTP 500.
    #[test]
    fn missing_cwd_still_emits_an_empty_url_key() {
        let req = build_request(&inputs(&[("p:1", "a")]), "en-US");
        assert_eq!(req.target_group[0].url, "");
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains(r#""url":"""#));
    }

    /// Summaries must never reach the service — only title and cwd.
    #[test]
    fn request_carries_no_summary_text() {
        let inp = vec![RemoteInput {
            session_key: "p:1".into(),
            title: "Fix auth bug".into(),
            cwd_uri: Some("file:///D:/proj".into()),
            existing_group: None,
        }];
        let json = serde_json::to_string(&build_request(&inp, "en-US")).unwrap();
        assert!(json.contains("Fix auth bug"));
        assert!(!json.contains("SECRET"));
    }

    #[test]
    fn parse_stream_keeps_last_cumulative_snapshot() {
        // The service re-sends the full list each line; only the last counts.
        let body = concat!(
            r#"{"resultingGroupsList":[{"groupId":"1","groupName":"Dev","tabIdList":["1"],"explanation":null}]}"#,
            "\n",
            r#"{"resultingGroupsList":[{"groupId":"1","groupName":"Dev","tabIdList":["1","2"],"explanation":null}]}"#,
            "\n",
            r#"["DONE"]"#,
            "\n"
        );
        let groups = parse_stream(body).unwrap().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].tab_id_list, vec!["1", "2"]);
    }

    #[test]
    fn parse_stream_tolerates_missing_done_sentinel() {
        let body = r#"{"resultingGroupsList":[{"groupId":"1","groupName":"Dev","tabIdList":["1"],"explanation":null}]}"#;
        let groups = parse_stream(body).unwrap().unwrap();
        assert_eq!(groups.len(), 1);
    }

    #[test]
    fn parse_stream_empty_body_yields_none() {
        assert!(parse_stream("").unwrap().is_none());
        assert!(parse_stream("\n\n").unwrap().is_none());
    }

    /// Error responses are plain JSON, never streamed. They must not be
    /// mistaken for a result.
    #[test]
    fn parse_stream_rejects_problem_details_body() {
        let body = r#"{"errors":{"":["The input was not valid."]},"title":"One or more validation errors occurred.","status":400}"#;
        assert!(parse_stream(body).is_err());
    }

    /// Regression: a lone outlier can be omitted from the response entirely.
    /// Missing ids must be dropped silently, never panic or mis-map.
    #[test]
    fn omitted_input_items_produce_no_suggestion() {
        let inp = inputs(&[("p:1", "a"), ("p:2", "b"), ("p:3", "c")]);
        let groups = vec![ResultGroup {
            group_id: "1".into(),
            group_name: "Dev".into(),
            tab_id_list: vec!["1".into(), "2".into()],
        }];
        let out = to_suggestions(&groups, &inp, &[]);
        assert_eq!(out.len(), 2);
        assert!(!out.iter().any(|s| s.session == "p:3"));
    }

    /// Out-of-range or garbage tabIds must never index out of bounds.
    #[test]
    fn out_of_range_tab_ids_are_ignored() {
        let inp = inputs(&[("p:1", "a")]);
        let groups = vec![ResultGroup {
            group_id: "1".into(),
            group_name: "Dev".into(),
            tab_id_list: vec!["0".into(), "99".into(), "abc".into(), "1".into()],
        }];
        let out = to_suggestions(&groups, &inp, &[]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].session, "p:1");
    }

    #[test]
    fn existing_group_name_marks_suggestion_as_not_new() {
        let inp = inputs(&[("p:1", "a")]);
        let groups = vec![ResultGroup {
            group_id: "1".into(),
            group_name: "Rust Tooling".into(),
            tab_id_list: vec!["1".into()],
        }];
        let out = to_suggestions(&groups, &inp, &["rust tooling".to_string()]);
        assert!(!out[0].is_new, "case-insensitive match with existing group");

        let out2 = to_suggestions(&groups, &inp, &["Something Else".to_string()]);
        assert!(out2[0].is_new);
    }

    #[test]
    fn blank_group_names_are_skipped() {
        let inp = inputs(&[("p:1", "a")]);
        let groups = vec![ResultGroup {
            group_id: "1".into(),
            group_name: "   ".into(),
            tab_id_list: vec!["1".into()],
        }];
        assert!(to_suggestions(&groups, &inp, &[]).is_empty());
    }

    /// Privacy invariant — no URL may ever be transmitted.
    #[test]
    fn request_never_includes_a_url() {
        let inp = inputs(&[("p:1", "secret-repo work"), ("p:2", "other")]);
        let req = build_request(&inp, "en-US");
        assert!(req.target_group.iter().all(|t| t.url.is_empty()));
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains(r#""url":"""#));
    }

    /// `experimentId` must be present (empty is fine) — omitting it is rejected
    /// by the service with a non-standard 667.
    #[test]
    fn request_includes_required_experiment_id_key() {
        let req = build_request(&inputs(&[("p:1", "a")]), "en-US");
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains(r#""experimentId":"""#));
    }

    #[test]
    fn tab_ids_are_one_based_and_sequential() {
        let req = build_request(&inputs(&[("p:1", "a"), ("p:2", "b")]), "en-US");
        assert_eq!(req.target_group[0].tab_id, "1");
        assert_eq!(req.target_group[1].tab_id, "2");
    }

    /// Members of the same existing group must share one groupId so the
    /// service preserves the group instead of renaming or splitting it.
    #[test]
    fn existing_groups_share_a_stable_group_id() {
        let inp = vec![
            RemoteInput {
                session_key: "p:1".into(),
                title: "a".into(),
                cwd_uri: None,
                existing_group: Some("Rust Tooling".into()),
            },
            RemoteInput {
                session_key: "p:2".into(),
                title: "b".into(),
                cwd_uri: None,
                existing_group: Some("Rust Tooling".into()),
            },
            RemoteInput {
                session_key: "p:3".into(),
                title: "c".into(),
                cwd_uri: None,
                existing_group: None,
            },
        ];
        let req = build_request(&inp, "en-US");
        assert_eq!(req.target_group[0].group_id, req.target_group[1].group_id);
        assert_ne!(req.target_group[0].group_id, "-1");
        assert_eq!(req.target_group[0].group_name, "Rust Tooling");
        // Ungrouped items use the -1 sentinel so the service may assign them.
        assert_eq!(req.target_group[2].group_id, "-1");
        assert_eq!(req.target_group[2].group_name, "");
    }

    #[test]
    fn empty_input_short_circuits_without_network() {
        let out = suggest(&[], &[], "en-US", 1).unwrap();
        assert!(out.is_empty());
    }

    /// End-to-end check against the LIVE service.
    ///
    /// `#[ignore]` because it requires network and depends on an undocumented
    /// third-party endpoint — it must never gate CI. Run deliberately with:
    /// `cargo test --lib grouping::remote::tests::live -- --ignored --nocapture`
    #[test]
    #[ignore = "requires network; hits an undocumented third-party endpoint"]
    fn live_endpoint_groups_real_titles() {
        let inp = vec![
            RemoteInput {
                session_key: "p:1".into(),
                title: "Fix clippy warnings in the provider scan".into(),
                cwd_uri: Some(path_to_file_uri(std::path::Path::new(
                    r"D:\Demo\agent-session-tui",
                ))),
                existing_group: None,
            },
            RemoteInput {
                session_key: "p:2".into(),
                title: "Add regression test for lock file detection".into(),
                cwd_uri: Some(path_to_file_uri(std::path::Path::new(
                    r"D:\Demo\agent-session-tui",
                ))),
                existing_group: None,
            },
            RemoteInput {
                session_key: "p:3".into(),
                title: "Free up C drive space".into(),
                cwd_uri: Some(path_to_file_uri(std::path::Path::new(r"C:\Users\me"))),
                existing_group: None,
            },
            RemoteInput {
                session_key: "p:4".into(),
                title: "Fix taskbar responsiveness".into(),
                cwd_uri: Some(path_to_file_uri(std::path::Path::new(r"C:\Users\me"))),
                existing_group: None,
            },
        ];
        let out = suggest(&inp, &[], "en-US", 30).expect("live call failed");
        assert!(!out.is_empty(), "expected at least one suggestion");
        for s in &out {
            assert!(
                inp.iter().any(|i| i.session_key == s.session),
                "returned a session key we never sent: {}",
                s.session
            );
            assert!(!s.group.trim().is_empty(), "group name must not be blank");
        }
        eprintln!("live suggestions: {out:#?}");
    }
}
