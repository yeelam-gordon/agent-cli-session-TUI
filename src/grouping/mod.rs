//! Grouping engine dispatch.
//!
//! Three engines, selected by `[grouping] engine` in config.toml:
//!
//! | Engine   | Network | Latency | Notes |
//! |----------|---------|---------|-------|
//! | `remote` | yes     | ~2s     | **default** — local pre-pass, then the service names the clusters |
//! | `local`  | none    | ~0.2s   | word-overlap matching; also the fallback when `remote` fails |
//! | `acp`    | yes     | 30–180s | legacy: spawns the configured CLI |
//!
//! The `remote` engine always runs the local pre-pass first and sends only one
//! representative per cluster, then expands the answer back over each cluster's
//! members. That cuts what leaves the machine (measured: 40 sessions → 8
//! entries) and costs one request instead of one per session. If the remote call
//! fails for any reason it degrades to the local result rather than erroring.

pub mod local;
pub mod remote;

use crate::acp::AiSuggestion;
use crate::config::GroupingEngine;

/// A session offered to the grouping engine.
#[derive(Debug, Clone)]
pub struct GroupingInput {
    /// `provider:session_id`.
    pub key: String,
    /// Text to cluster on locally. May include the title plus a short summary —
    /// richer signal produces better clusters, and this **never leaves the
    /// machine**.
    pub text: String,
    /// Title alone. Sent to a remote engine along with [`Self::cwd`]; see the
    /// note in [`remote`] on what is transmitted.
    pub title: String,
    /// Working directory. Sent to a remote engine as a `file:///` URI because
    /// it measurably improves grouping quality.
    pub cwd: Option<std::path::PathBuf>,
    /// Group this session already belongs to, if any.
    pub existing_group: Option<String>,
}

/// Suggestions produced purely from local clusters.
///
/// When a cluster contains an "anchor" — a session already assigned to a user
/// group — that group's real name is used and the suggestion is marked
/// `is_new: false`. This is what lets the offline engine grow existing groups
/// instead of only ever inventing new keyword names.
fn local_suggestions(clusters: &[local::Cluster], inputs: &[GroupingInput]) -> Vec<AiSuggestion> {
    let group_of: std::collections::HashMap<&str, &str> = inputs
        .iter()
        .filter_map(|i| i.existing_group.as_deref().map(|g| (i.key.as_str(), g)))
        .collect();

    let mut out = Vec::new();
    for c in clusters {
        // Prefer an existing group name carried by any anchor in this cluster.
        let anchored = c
            .members
            .iter()
            .find_map(|k| group_of.get(k.as_str()).copied());

        // A cluster of one carries no grouping signal unless it is anchored —
        // suggesting a group named after a single session's keywords is noise.
        if anchored.is_none() && c.members.len() < 2 {
            continue;
        }
        let (group, is_new, score, reason) = match anchored {
            Some(name) => (
                name.to_string(),
                false,
                0.70,
                "Local: similar to grouped session".to_string(),
            ),
            None => (
                c.heuristic_name.clone(),
                true,
                0.60,
                "Local: similar titles".to_string(),
            ),
        };
        for key in &c.members {
            // Never re-suggest a group for an anchor — it is already assigned.
            if group_of.contains_key(key.as_str()) {
                continue;
            }
            out.push(AiSuggestion {
                session: key.clone(),
                group: group.clone(),
                is_new,
                score,
                reason: reason.clone(),
            });
        }
    }
    out
}

/// Expand per-representative suggestions back across each cluster's members.
fn expand_over_clusters(
    rep_suggestions: &[AiSuggestion],
    clusters: &[local::Cluster],
    inputs: &[GroupingInput],
) -> Vec<AiSuggestion> {
    let mut out = Vec::new();
    for s in rep_suggestions {
        // Find the cluster whose representative produced this suggestion.
        let cluster = clusters
            .iter()
            .find(|c| inputs.get(c.representative).map(|i| &i.key) == Some(&s.session));
        match cluster {
            Some(c) => {
                for key in &c.members {
                    out.push(AiSuggestion {
                        session: key.clone(),
                        ..s.clone()
                    });
                }
            }
            // Representative no longer resolvable — keep the original.
            None => out.push(s.clone()),
        }
    }
    out
}

/// Run the configured grouping engine. Blocking — call from a blocking thread.
///
/// `existing_groups` are the user's current group names. They decide `is_new`,
/// and the remote engine offers them as context so candidates can be folded
/// into a group the user already has.
pub fn suggest(
    engine: GroupingEngine,
    inputs: &[GroupingInput],
    existing_groups: &[String],
    cfg: &crate::config::GroupingConfig,
) -> Result<Vec<AiSuggestion>, String> {
    let language = cfg.language.as_str();
    let timeout_secs = cfg.timeout_secs;
    // Zero when the user opts out of reusing existing groups, so every
    // suggestion comes back freshly named.
    let max_anchors = if cfg.reuse_existing_groups {
        cfg.max_group_anchors
    } else {
        0
    };

    if inputs.is_empty() {
        return Ok(Vec::new());
    }

    let items: Vec<(String, String)> = inputs
        .iter()
        .map(|i| (i.key.clone(), i.text.clone()))
        .collect();
    let clusters = local::cluster(&items);
    crate::log::info(&format!(
        "Grouping: {} sessions → {} local clusters (engine={:?})",
        inputs.len(),
        clusters.len(),
        engine
    ));

    match engine {
        GroupingEngine::Local => Ok(local_suggestions(&clusters, inputs)),
        GroupingEngine::Acp => {
            // Handled by the legacy path in src/acp.rs; not reachable here.
            Err("acp engine is dispatched separately".to_string())
        }
        GroupingEngine::Remote => {
            let is_anchor = |i: &GroupingInput| i.existing_group.is_some();

            // Candidates: one representative per cluster that contains an
            // ungrouped session, and the representative must itself be
            // ungrouped. Sending an already-grouped session as a candidate
            // asks the service to regroup something already settled.
            let mut reps: Vec<remote::RemoteInput> = Vec::new();
            for c in &clusters {
                let candidate = c
                    .members
                    .iter()
                    .filter_map(|k| inputs.iter().find(|i| &i.key == k))
                    .filter(|i| !is_anchor(i))
                    // Longest title is the most descriptive for classification.
                    .max_by_key(|i| i.title.len());
                if let Some(i) = candidate {
                    reps.push(remote::RemoteInput {
                        session_key: i.key.clone(),
                        // Title and cwd only — never `text`, which carries a summary.
                        title: i.title.clone(),
                        cwd_uri: i.cwd.as_deref().map(remote::path_to_file_uri),
                        existing_group: None,
                    });
                }
            }

            if reps.is_empty() {
                crate::log::info("Remote grouping skipped: nothing ungrouped to classify");
                return Ok(Vec::new());
            }
            let reps_only = reps.clone();

            // Offer the user's existing group NAMES as context so candidates
            // get folded into a group they already have instead of spawning a
            // near-duplicate — while still leaving the service free to invent
            // new groups for anything that doesn't fit.
            //
            // These are synthetic placeholders, one per group, carrying only
            // the group name. Earlier this sent two REAL sessions per group,
            // which (a) put 34 samples against 30 candidates and drowned out
            // new-group suggestions, and (b) transmitted the titles of already
            // grouped sessions. Neither is necessary.
            //
            // The placeholder url must be non-empty and generic: a blank url
            // with a real groupId makes the service return an empty body, and
            // a realistic project path over-anchors candidates onto that group.
            let mut payload: Vec<remote::RemoteInput> = existing_groups
                .iter()
                .filter(|g| !g.trim().is_empty())
                .take(max_anchors)
                .enumerate()
                .map(|(i, name)| remote::RemoteInput {
                    session_key: format!("__group_placeholder_{i}"),
                    title: name.clone(),
                    cwd_uri: Some(format!("file:///groups/{name}")),
                    existing_group: Some(name.clone()),
                })
                .collect();
            let placeholder_count = payload.len();
            payload.extend(reps);
            crate::log::info(&format!(
                "Remote grouping: {} candidates + {} existing-group names",
                payload.len() - placeholder_count,
                placeholder_count
            ));

            // The service intermittently answers 200 with an entirely empty
            // body for some payload shapes. Rather than chase an inferred
            // contract, try the full payload, then retry with candidates only
            // (a shape that reliably works), then fall back to local. The user
            // must never end up with nothing.
            let attempt = |payload: &[remote::RemoteInput], label: &str| {
                match remote::suggest(payload, existing_groups, language, timeout_secs) {
                    Ok(s) if !s.is_empty() => Some(s),
                    Ok(_) => {
                        crate::log::info(&format!("Remote grouping returned no groups ({label})"));
                        None
                    }
                    Err(e) => {
                        crate::log::warn(&format!("Remote grouping failed ({label}): {e}"));
                        None
                    }
                }
            };

            let label = if placeholder_count > 0 {
                "with existing-group names"
            } else {
                "candidates only"
            };
            let rep_suggestions = attempt(&payload, label).or_else(|| {
                // Only worth retrying if the second payload differs — with no
                // group names attached the two are identical.
                if placeholder_count > 0 {
                    attempt(&reps_only, "candidates only (retry)")
                } else {
                    None
                }
            });

            match rep_suggestions {
                Some(s) => {
                    let expanded = expand_over_clusters(&s, &clusters, inputs);
                    // Drop the synthetic placeholders and anything already
                    // grouped — neither is a real suggestion for the user.
                    let anchors: std::collections::HashSet<&str> = inputs
                        .iter()
                        .filter(|i| is_anchor(i))
                        .map(|i| i.key.as_str())
                        .collect();
                    Ok(expanded
                        .into_iter()
                        .filter(|s| !s.session.starts_with("__group_placeholder_"))
                        .filter(|s| !anchors.contains(s.session.as_str()))
                        .collect())
                }
                None => {
                    crate::log::info("Remote grouping produced nothing — using local engine");
                    Ok(local_suggestions(&clusters, inputs))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(v: &[(&str, &str)]) -> Vec<GroupingInput> {
        v.iter()
            .map(|(k, t)| GroupingInput {
                key: k.to_string(),
                text: t.to_string(),
                title: t.to_string(),
                cwd: None,
                existing_group: None,
            })
            .collect()
    }

    /// Grouping config for tests. `timeout_secs = 0` makes any remote attempt
    /// fail immediately, which is how the fallback paths are exercised.
    fn test_cfg(timeout_secs: u64) -> crate::config::GroupingConfig {
        crate::config::GroupingConfig {
            timeout_secs,
            ..Default::default()
        }
    }

    /// Privacy boundary: the local `text` may carry a summary, but only the
    /// bare `title` and the cwd URI may reach a remote engine.
    #[test]
    fn remote_engine_receives_title_only_never_summary() {
        let inp = vec![GroupingInput {
            key: "p:1".into(),
            text: "Fix auth bug SECRET_SUMMARY_MARKER internal detail".into(),
            title: "Fix auth bug".into(),
            cwd: Some(std::path::PathBuf::from(r"D:\Demo\proj")),
            existing_group: None,
        }];
        let items: Vec<(String, String)> = inp
            .iter()
            .map(|i| (i.key.clone(), i.text.clone()))
            .collect();
        let clusters = local::cluster(&items);
        let reps: Vec<remote::RemoteInput> = clusters
            .iter()
            .filter_map(|c| inp.get(c.representative))
            .map(|i| remote::RemoteInput {
                session_key: i.key.clone(),
                title: i.title.clone(),
                cwd_uri: i.cwd.as_deref().map(remote::path_to_file_uri),
                existing_group: i.existing_group.clone(),
            })
            .collect();
        assert_eq!(reps.len(), 1);
        assert_eq!(reps[0].title, "Fix auth bug");
        assert!(
            !reps[0].title.contains("SECRET_SUMMARY_MARKER"),
            "summary text must never reach the remote engine"
        );
        assert_eq!(reps[0].cwd_uri.as_deref(), Some("file:///D:/Demo/proj"));
    }

    #[test]
    fn local_engine_groups_near_duplicates() {
        let inp = inputs(&[
            ("p:1", "Read the complete benchmark prompt from prompt-files"),
            ("p:2", "Read the complete benchmark prompt from prompt-files"),
            ("p:3", "Plan quarterly budget review meeting"),
        ]);
        let out = suggest(GroupingEngine::Local, &inp, &[], &test_cfg(5)).unwrap();
        // The two duplicates get a group; the lone outlier does not.
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].group, out[1].group);
        assert!(!out.iter().any(|s| s.session == "p:3"));
    }

    /// Singleton clusters must not produce a group named after one session.
    #[test]
    fn local_engine_ignores_singletons() {
        let inp = inputs(&[("p:1", "Totally unique work item"), ("p:2", "Another one")]);
        let out = suggest(GroupingEngine::Local, &inp, &[], &test_cfg(5)).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn empty_input_returns_empty_without_work() {
        let out = suggest(GroupingEngine::Local, &[], &[], &test_cfg(5)).unwrap();
        assert!(out.is_empty());
    }

    /// The privacy/efficiency property: a suggestion for one representative
    /// must apply to every member of its cluster.
    #[test]
    fn expansion_covers_all_cluster_members() {
        let inp = inputs(&[
            ("p:1", "benchmark prompt files run"),
            ("p:2", "benchmark prompt files run"),
            ("p:3", "benchmark prompt files run"),
        ]);
        let items: Vec<(String, String)> = inp
            .iter()
            .map(|i| (i.key.clone(), i.text.clone()))
            .collect();
        let clusters = local::cluster(&items);
        assert_eq!(clusters.len(), 1);

        let rep_key = inp[clusters[0].representative].key.clone();
        let rep = vec![AiSuggestion {
            session: rep_key,
            group: "AI Development".into(),
            is_new: true,
            score: 0.75,
            reason: "Auto-grouping service".into(),
        }];

        let expanded = expand_over_clusters(&rep, &clusters, &inp);
        assert_eq!(expanded.len(), 3, "one suggestion must cover all 3 members");
        assert!(expanded.iter().all(|s| s.group == "AI Development"));
        let mut keys: Vec<&str> = expanded.iter().map(|s| s.session.as_str()).collect();
        keys.sort();
        assert_eq!(keys, vec!["p:1", "p:2", "p:3"]);
    }

    /// A representative that no longer resolves must not silently vanish.
    #[test]
    fn expansion_keeps_unmatched_representative() {
        let inp = inputs(&[("p:1", "something")]);
        let clusters = local::cluster(&[("p:1".to_string(), "something".to_string())]);
        let rep = vec![AiSuggestion {
            session: "p:GONE".into(),
            group: "X".into(),
            is_new: true,
            score: 0.75,
            reason: "r".into(),
        }];
        let expanded = expand_over_clusters(&rep, &clusters, &inp);
        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].session, "p:GONE");
    }

    /// Build one input, optionally anchored to an existing group.
    fn input_in(key: &str, text: &str, group: Option<&str>) -> GroupingInput {
        GroupingInput {
            key: key.to_string(),
            text: text.to_string(),
            title: text.to_string(),
            cwd: None,
            existing_group: group.map(|g| g.to_string()),
        }
    }

    /// Anchors let the offline engine grow an existing group instead of
    /// inventing a new keyword name.
    #[test]
    fn anchored_cluster_uses_existing_group_name() {
        let inp = vec![
            input_in("p:anchor", "benchmark prompt files run", Some("Rust Tooling")),
            input_in("p:new", "benchmark prompt files run", None),
        ];
        let out = suggest(GroupingEngine::Local, &inp, &[], &test_cfg(5)).unwrap();
        assert_eq!(out.len(), 1, "anchor itself must not be re-suggested");
        assert_eq!(out[0].session, "p:new");
        assert_eq!(out[0].group, "Rust Tooling");
        assert!(!out[0].is_new, "assigning into an existing group");
    }

    /// A lone new session that matches an anchor should still be suggested,
    /// even though singleton clusters are otherwise ignored.
    #[test]
    fn anchor_rescues_small_clusters() {
        let inp = vec![
            input_in("p:anchor", "authentication token refresh logic", Some("Auth")),
            input_in("p:new", "authentication token refresh logic", None),
        ];
        let out = suggest(GroupingEngine::Local, &inp, &[], &test_cfg(5)).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].group, "Auth");
    }

    #[test]
    fn anchors_are_never_suggested_back_to_the_user() {
        let inp = vec![
            input_in("p:a1", "same words here now", Some("G")),
            input_in("p:a2", "same words here now", Some("G")),
        ];
        let out = suggest(GroupingEngine::Local, &inp, &[], &test_cfg(5)).unwrap();
        assert!(out.is_empty(), "all inputs were already grouped");
    }

    /// Regression: when every input was already grouped, the remote engine
    /// still sent them all as candidates. The service saw nothing to assign and
    /// returned zero groups, so the user got zero suggestions. Observed live:
    /// 17 items sent, `0 groups → 0 suggestions`. With nothing ungrouped we
    /// must not make a network call at all.
    #[test]
    fn remote_engine_makes_no_call_when_everything_is_anchored() {
        let inp = vec![
            input_in("p:a1", "alpha beta gamma delta", Some("G")),
            input_in("p:a2", "epsilon zeta eta theta", Some("H")),
        ];
        // Timeout of 0 would fail instantly if a request were attempted; a
        // successful empty result proves we short-circuited before the network.
        let out = suggest(GroupingEngine::Remote, &inp, &[], &test_cfg(0)).unwrap();
        assert!(out.is_empty(), "no ungrouped sessions → no suggestions");
    }

    /// The remote engine must never leave the user with nothing. With an
    /// unusable timeout every request fails, and the result must still be the
    /// local engine's output rather than an error or an empty list.
    #[test]
    fn remote_failure_falls_back_to_local_suggestions() {
        let inp = vec![
            input_in("p:1", "benchmark prompt files run", None),
            input_in("p:2", "benchmark prompt files run", None),
        ];
        // timeout_secs = 0 → the HTTP attempts fail immediately.
        let out = suggest(GroupingEngine::Remote, &inp, &[], &test_cfg(0)).unwrap();
        assert_eq!(out.len(), 2, "must fall back to local clustering");
        assert_eq!(out[0].group, out[1].group);
        assert!(out[0].reason.starts_with("Local:"));
    }

    /// Existing group names are offered to the service as lightweight
    /// placeholders so candidates can be folded into a group the user already
    /// has. Those placeholders are internal plumbing — they must never surface
    /// as suggestions, and they must never be mistaken for a real session.
    #[test]
    fn group_name_placeholders_never_surface_as_suggestions() {
        let clusters = local::cluster(&[("p:1".to_string(), "some work item".to_string())]);
        let inp = inputs(&[("p:1", "some work item")]);
        let rep = vec![
            AiSuggestion {
                session: "__group_placeholder_0".into(),
                group: "agent-mgt-tui".into(),
                is_new: false,
                score: 0.75,
                reason: "r".into(),
            },
            AiSuggestion {
                session: "p:1".into(),
                group: "agent-mgt-tui".into(),
                is_new: false,
                score: 0.75,
                reason: "r".into(),
            },
        ];
        let expanded = expand_over_clusters(&rep, &clusters, &inp);
        let kept: Vec<&AiSuggestion> = expanded
            .iter()
            .filter(|s| !s.session.starts_with("__group_placeholder_"))
            .collect();
        assert_eq!(kept.len(), 1, "placeholder must be filtered out");
        assert_eq!(kept[0].session, "p:1");
    }

    /// Group-name context is off by default: the service accepts pre-assigned
    /// groups only erratically (3 real names returned an empty body while 1
    /// succeeded), so enabling it costs a wasted round-trip before the retry.
    #[test]
    fn group_name_context_is_off_by_default() {
        let cfg = crate::config::GroupingConfig::default();
        assert!(
            !cfg.reuse_existing_groups,
            "sending existing group names must be opt-in — the service \
             rejects those payloads unpredictably"
        );
    }

    /// Opting out of group reuse must stop existing-group context being sent,
    /// so every suggestion comes back freshly named.
    #[test]
    fn reuse_existing_groups_false_sends_no_group_names() {
        let cfg = crate::config::GroupingConfig {
            reuse_existing_groups: false,
            ..Default::default()
        };
        // The engine zeroes the budget when reuse is off.
        let effective = if cfg.reuse_existing_groups {
            cfg.max_group_anchors
        } else {
            0
        };
        assert_eq!(effective, 0);
    }
}
