//! Local, zero-egress grouping pre-pass.
//!
//! Clusters sessions by token overlap on title + summary, then picks one
//! representative per cluster. Two reasons this runs before any network call:
//!
//! 1. **Privacy** — only representatives are ever sent to a remote engine, so
//!    near-duplicate sessions collapse to a single title. Measured on real data:
//!    40 sessions → 8 representatives.
//! 2. **Offline capability** — when no remote engine is enabled, the clusters
//!    themselves are the grouping, named heuristically.
//!
//! Token overlap beats working-directory grouping here. Measured on 40 real
//! sessions: cwd → 24 fragments (benchmark harnesses create sibling per-run
//! repos), token-Jaccard → 8 clusters that match human intuition.

use std::collections::{HashMap, HashSet};

/// Words carrying no grouping signal in agent session titles.
const STOPWORDS: &[&str] = &[
    "the", "a", "an", "from", "for", "and", "to", "of", "in", "on", "with", "complete", "read",
    "run", "then", "is", "it", "this", "that", "at", "by", "as", "be", "are", "was", "session",
    "task", "using", "use", "get", "set", "new", "old", "via", "into", "out", "up",
];

/// Minimum Jaccard similarity for two sessions to join the same cluster.
/// 0.5 was tuned on real session data: lower merges unrelated work, higher
/// fails to collapse near-identical benchmark runs.
const JACCARD_THRESHOLD: f32 = 0.5;

/// A cluster of sessions that are near-duplicates of each other.
#[derive(Debug, Clone)]
pub struct Cluster {
    /// Session keys (`provider:session_id`) belonging to this cluster.
    pub members: Vec<String>,
    /// Index into the caller's slice for the member chosen to represent the
    /// cluster to a remote engine.
    pub representative: usize,
    /// Heuristic name derived from the cluster's most common tokens.
    pub heuristic_name: String,
}

/// Split `text` into lowercase, stopword-filtered, deduplicated tokens.
pub fn tokenize(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2 && !STOPWORDS.contains(w))
        .map(|w| w.to_string())
        .collect()
}

/// Jaccard similarity — |intersection| / |union|. Returns 0.0 if either side
/// is empty so untitled sessions never merge with anything.
pub fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f32;
    let union = a.union(b).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

/// Disjoint-set find with path compression.
fn find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

/// Build a heuristic name from the most frequent tokens across a cluster.
///
/// Deliberately modest: this produces keyword-ish names like
/// `benchmark-prompt-files`, not fluent labels. It exists so the offline path
/// always yields *something* the user can rename with `e`.
fn heuristic_name(token_sets: &[&HashSet<String>]) -> String {
    let mut freq: HashMap<&str, usize> = HashMap::new();
    for set in token_sets {
        for tok in set.iter() {
            *freq.entry(tok.as_str()).or_insert(0) += 1;
        }
    }
    let mut ranked: Vec<(&str, usize)> = freq.into_iter().collect();
    // Sort by descending frequency, then alphabetically so the name is stable
    // across runs rather than depending on HashMap iteration order.
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let name: Vec<&str> = ranked.iter().take(3).map(|(t, _)| *t).collect();
    if name.is_empty() {
        "ungrouped".to_string()
    } else {
        name.join("-")
    }
}

/// Cluster `items` (session key + text to cluster on) by token overlap.
///
/// Returns one [`Cluster`] per group of near-duplicates. Singletons produce
/// single-member clusters — they are never dropped.
pub fn cluster(items: &[(String, String)]) -> Vec<Cluster> {
    let n = items.len();
    if n == 0 {
        return Vec::new();
    }
    let token_sets: Vec<HashSet<String>> = items.iter().map(|(_, text)| tokenize(text)).collect();

    let mut parent: Vec<usize> = (0..n).collect();
    for i in 0..n {
        for j in (i + 1)..n {
            if jaccard(&token_sets[i], &token_sets[j]) > JACCARD_THRESHOLD {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                if ri != rj {
                    parent[ri] = rj;
                }
            }
        }
    }

    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().push(i);
    }

    let mut clusters: Vec<Cluster> = groups
        .into_values()
        .map(|idxs| {
            let sets: Vec<&HashSet<String>> = idxs.iter().map(|&i| &token_sets[i]).collect();
            // Represent the cluster with its longest text — the most
            // descriptive member, and the one a remote engine can best classify.
            let representative = *idxs
                .iter()
                .max_by_key(|&&i| items[i].1.len())
                .unwrap_or(&idxs[0]);
            Cluster {
                members: idxs.iter().map(|&i| items[i].0.clone()).collect(),
                representative,
                heuristic_name: heuristic_name(&sets),
            }
        })
        .collect();

    // Largest clusters first, then by representative index for determinism.
    clusters.sort_by(|a, b| {
        b.members
            .len()
            .cmp(&a.members.len())
            .then_with(|| a.representative.cmp(&b.representative))
    });
    clusters
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter()
            .map(|(k, t)| (k.to_string(), t.to_string()))
            .collect()
    }

    #[test]
    fn tokenize_strips_stopwords_and_short_words() {
        let toks = tokenize("Read the complete benchmark prompt");
        assert!(toks.contains("benchmark"));
        assert!(toks.contains("prompt"));
        assert!(!toks.contains("the"), "stopword must be removed");
        assert!(!toks.contains("read"), "stopword must be removed");
    }

    #[test]
    fn jaccard_empty_never_matches() {
        let empty = HashSet::new();
        let full = tokenize("benchmark prompt");
        assert_eq!(jaccard(&empty, &full), 0.0);
        assert_eq!(jaccard(&empty, &empty), 0.0);
    }

    /// The load-bearing case: near-identical benchmark titles must collapse.
    /// Measured on real data — 33 such sessions became one representative.
    #[test]
    fn near_duplicates_collapse_into_one_cluster() {
        let input = items(&[
            (
                "p:1",
                "Read the complete benchmark prompt from prompt-files 6-3",
            ),
            (
                "p:2",
                "Read the complete benchmark prompt from prompt-files 6-3",
            ),
            (
                "p:3",
                "Read the complete benchmark prompt from prompt-files 6-3",
            ),
        ]);
        let clusters = cluster(&input);
        assert_eq!(clusters.len(), 1, "identical titles must form one cluster");
        assert_eq!(clusters[0].members.len(), 3);
    }

    /// Unrelated sessions must NOT be merged — this is the failure mode that
    /// made cwd-based grouping unusable.
    #[test]
    fn unrelated_sessions_stay_separate() {
        let input = items(&[
            ("p:1", "Free Up C Drive Space"),
            ("p:2", "Investigate Auto-Grouping Replacement"),
            ("p:3", "Review Nikola Features"),
        ]);
        let clusters = cluster(&input);
        assert_eq!(clusters.len(), 3, "unrelated titles must not merge");
    }

    #[test]
    fn every_input_appears_in_exactly_one_cluster() {
        let input = items(&[
            ("p:1", "Fix clippy warnings in provider scan"),
            ("p:2", "Fix clippy warnings in provider scan"),
            ("p:3", "Plan quarterly budget review"),
            ("p:4", ""),
        ]);
        let clusters = cluster(&input);
        let mut seen: Vec<String> = clusters.iter().flat_map(|c| c.members.clone()).collect();
        seen.sort();
        assert_eq!(seen, vec!["p:1", "p:2", "p:3", "p:4"]);
    }

    #[test]
    fn empty_input_yields_no_clusters() {
        assert!(cluster(&[]).is_empty());
    }

    #[test]
    fn representative_is_the_longest_text() {
        // Jaccard here is 3/4 = 0.75, comfortably above the threshold, so the
        // two do cluster and the longer text must win as representative.
        let input = items(&[
            ("p:1", "benchmark prompt files"),
            ("p:2", "benchmark prompt files extended"),
        ]);
        let clusters = cluster(&input);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].representative, 1);
    }

    /// Guards the threshold itself: partial overlap below 0.5 must NOT merge.
    /// `{benchmark,prompt}` vs `{benchmark,prompt,extended,detail,here}` = 0.4.
    #[test]
    fn weak_overlap_does_not_merge() {
        let input = items(&[
            ("p:1", "benchmark prompt"),
            ("p:2", "benchmark prompt extended detail here"),
        ]);
        assert_eq!(cluster(&input).len(), 2);
    }

    /// Names must not depend on HashMap iteration order.
    #[test]
    fn heuristic_name_is_deterministic() {
        let input = items(&[
            ("p:1", "benchmark prompt files evaluation"),
            ("p:2", "benchmark prompt files evaluation"),
        ]);
        let a = cluster(&input)[0].heuristic_name.clone();
        for _ in 0..5 {
            assert_eq!(cluster(&input)[0].heuristic_name, a);
        }
    }

    #[test]
    fn untitled_sessions_do_not_merge_together() {
        let input = items(&[("p:1", ""), ("p:2", ""), ("p:3", "")]);
        let clusters = cluster(&input);
        assert_eq!(clusters.len(), 3, "empty titles must stay separate");
    }
}
