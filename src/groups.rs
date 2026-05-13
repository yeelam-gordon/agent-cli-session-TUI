//! Session grouping — persistent multi-group assignment with provenance tracking.
//!
//! Storage: `groups.json` in `data_dir`, separate from `archived.json`.
//! Sessions can belong to multiple groups. Each assignment tracks its source
//! (human or AI model) and optional confidence score.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Per-group-assignment metadata: who assigned it and how confident.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupAssignment {
    /// `"human"` or `"ai:<model-name>"`.
    pub source: String,
    /// AI confidence score (0.0–1.0). Absent for human assignments.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
}

/// Entry for a single session in the group store.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionGroups {
    /// group_name → assignment metadata.
    #[serde(default)]
    pub groups: HashMap<String, GroupAssignment>,
}

/// Top-level groups.json schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupStore {
    #[serde(default = "default_version")]
    pub version: u32,
    /// session_key ("provider:session_id") → group assignments.
    #[serde(default)]
    pub sessions: HashMap<String, SessionGroups>,
    /// session_key → list of dismissed group suggestions.
    #[serde(default)]
    pub dismissed: HashMap<String, Vec<String>>,
    /// group_name → human-readable description (what this group is about).
    #[serde(default)]
    pub descriptions: HashMap<String, String>,
}

fn default_version() -> u32 {
    1
}

impl Default for GroupStore {
    fn default() -> Self {
        Self {
            version: 1,
            sessions: HashMap::new(),
            dismissed: HashMap::new(),
            descriptions: HashMap::new(),
        }
    }
}

/// Runtime wrapper that pairs the store with its file path.
pub struct GroupManager {
    path: PathBuf,
    store: GroupStore,
}

impl GroupManager {
    /// Open (or create) the group store at the given path.
    pub fn open(path: &Path) -> Self {
        let store = match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => GroupStore::default(),
        };
        Self {
            path: path.to_path_buf(),
            store,
        }
    }

    /// Save to disk. Errors are logged but never panic.
    pub fn save(&self) {
        match serde_json::to_string_pretty(&self.store) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&self.path, json) {
                    crate::log::info(&format!("Failed to save groups.json: {}", e));
                }
            }
            Err(e) => {
                crate::log::info(&format!("Failed to serialize groups: {}", e));
            }
        }
    }

    /// Get all group names for a session key, or empty.
    pub fn groups_for(&self, session_key: &str) -> Vec<String> {
        self.store
            .sessions
            .get(session_key)
            .map(|sg| sg.groups.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Assign a session to a group (human).
    pub fn assign_human(&mut self, session_key: &str, group: &str) {
        self.assign_inner(session_key, group, "human");
        self.save();
    }

    /// In-memory assignment without persisting. Used by the `--mock-data`
    /// demo flow so the synthetic groups never pollute the real groups.json.
    pub fn assign_in_memory(&mut self, session_key: &str, group: &str) {
        self.assign_inner(session_key, group, "human");
    }

    fn assign_inner(&mut self, session_key: &str, group: &str, source: &str) {
        let entry = self
            .store
            .sessions
            .entry(session_key.to_string())
            .or_default();
        entry.groups.insert(
            group.to_string(),
            GroupAssignment {
                source: source.to_string(),
                score: None,
            },
        );
    }

    /// Remove a session from a specific group.
    pub fn unassign(&mut self, session_key: &str, group: &str) {
        if let Some(entry) = self.store.sessions.get_mut(session_key) {
            entry.groups.remove(group);
            if entry.groups.is_empty() {
                self.store.sessions.remove(session_key);
            }
        }
        self.save();
    }

    /// Rename a group across all sessions.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn rename_group(&mut self, old_name: &str, new_name: &str) {
        // Update session assignments
        for sg in self.store.sessions.values_mut() {
            if let Some(assignment) = sg.groups.remove(old_name) {
                sg.groups.insert(new_name.to_string(), assignment);
            }
        }
        // Move description
        if let Some(desc) = self.store.descriptions.remove(old_name) {
            self.store.descriptions.insert(new_name.to_string(), desc);
        }
        // Update dismissed entries
        for dismissed_groups in self.store.dismissed.values_mut() {
            for g in dismissed_groups.iter_mut() {
                if g == old_name {
                    *g = new_name.to_string();
                }
            }
        }
        self.save();
    }

    /// Delete a group — removes the label from all sessions.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn delete_group(&mut self, group: &str) {
        for sg in self.store.sessions.values_mut() {
            sg.groups.remove(group);
        }
        // Clean up empty session entries.
        self.store.sessions.retain(|_, sg| !sg.groups.is_empty());
        self.save();
    }

    /// All known group names, sorted by descending member count.
    /// Ties broken by ascending name so the order is stable across redraws
    /// (otherwise the prompt strip "flips" two equal-count groups every
    /// frame because HashMap iteration is non-deterministic).
    /// Returns (group_name, session_count).
    pub fn all_groups(&self) -> Vec<(String, usize)> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for sg in self.store.sessions.values() {
            for name in sg.groups.keys() {
                *counts.entry(name.clone()).or_insert(0) += 1;
            }
        }
        let mut result: Vec<_> = counts.into_iter().collect();
        result.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        result
    }

    /// Check if a suggestion was previously dismissed.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_dismissed(&self, session_key: &str, group: &str) -> bool {
        self.store
            .dismissed
            .get(session_key)
            .is_some_and(|v| v.contains(&group.to_string()))
    }

    /// Check if this session has ANY dismissals (skip it entirely in AI suggestions).
    pub fn has_any_dismissal(&self, session_key: &str) -> bool {
        self.store
            .dismissed
            .get(session_key)
            .is_some_and(|v| !v.is_empty())
    }

    /// Record a dismissed suggestion.
    pub fn dismiss(&mut self, session_key: &str, group: &str) {
        self.store
            .dismissed
            .entry(session_key.to_string())
            .or_default()
            .push(group.to_string());
        self.save();
    }

    /// Set description for a group.
    pub fn set_group_description(&mut self, group: &str, desc: &str) {
        self.store.descriptions.insert(group.to_string(), desc.to_string());
        self.save();
    }

    /// Get description for a group, if any.
    pub fn get_group_description(&self, group: &str) -> Option<String> {
        self.store.descriptions.get(group).cloned()
    }

    /// All groups with their descriptions (for AI prompt).
    /// Stable order: descending count, ties broken by ascending name.
    pub fn all_groups_with_descriptions(&self) -> Vec<(String, usize, Option<String>)> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for sg in self.store.sessions.values() {
            for name in sg.groups.keys() {
                *counts.entry(name.clone()).or_insert(0) += 1;
            }
        }
        let mut result: Vec<_> = counts
            .into_iter()
            .map(|(name, count)| {
                let desc = self.store.descriptions.get(&name).cloned();
                (name, count, desc)
            })
            .collect();
        result.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn round_trip_empty() {
        let tmp = NamedTempFile::new().unwrap();
        let mgr = GroupManager::open(tmp.path());
        assert!(mgr.all_groups().is_empty());
        assert!(mgr.groups_for("copilot:abc").is_empty());
    }

    #[test]
    fn assign_and_retrieve() {
        let tmp = NamedTempFile::new().unwrap();
        let mut mgr = GroupManager::open(tmp.path());
        mgr.assign_human("copilot:abc", "agent-tui");
        mgr.assign_human("copilot:abc", "perf");
        mgr.assign_human("claude:def", "agent-tui");

        let groups = mgr.groups_for("copilot:abc");
        assert!(groups.contains(&"agent-tui".to_string()));
        assert!(groups.contains(&"perf".to_string()));
        assert_eq!(groups.len(), 2);

        let all = mgr.all_groups();
        assert_eq!(all.len(), 2); // agent-tui and perf
    }

    #[test]
    fn unassign_removes_group() {
        let tmp = NamedTempFile::new().unwrap();
        let mut mgr = GroupManager::open(tmp.path());
        mgr.assign_human("copilot:abc", "agent-tui");
        mgr.assign_human("copilot:abc", "perf");
        mgr.unassign("copilot:abc", "perf");

        let groups = mgr.groups_for("copilot:abc");
        assert_eq!(groups, vec!["agent-tui".to_string()]);
    }

    #[test]
    fn rename_group_updates_all_sessions() {
        let tmp = NamedTempFile::new().unwrap();
        let mut mgr = GroupManager::open(tmp.path());
        mgr.assign_human("copilot:abc", "old-name");
        mgr.assign_human("claude:def", "old-name");
        mgr.rename_group("old-name", "new-name");

        assert!(mgr.groups_for("copilot:abc").contains(&"new-name".to_string()));
        assert!(mgr.groups_for("claude:def").contains(&"new-name".to_string()));
        assert!(mgr.all_groups().iter().all(|(n, _)| n != "old-name"));
    }

    #[test]
    fn delete_group_cleans_up() {
        let tmp = NamedTempFile::new().unwrap();
        let mut mgr = GroupManager::open(tmp.path());
        mgr.assign_human("copilot:abc", "doomed");
        mgr.delete_group("doomed");

        assert!(mgr.groups_for("copilot:abc").is_empty());
        assert!(mgr.all_groups().is_empty());
    }

    #[test]
    fn persist_and_reload() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        {
            let mut mgr = GroupManager::open(&path);
            mgr.assign_human("copilot:abc", "agent-tui");
        }

        let mgr2 = GroupManager::open(&path);
        assert!(mgr2.groups_for("copilot:abc").contains(&"agent-tui".to_string()));
    }

    #[test]
    fn stale_session_key_does_not_crash() {
        let tmp = NamedTempFile::new().unwrap();
        // Write a groups.json referencing a session that won't exist at runtime.
        let json = r#"{"version":1,"sessions":{"copilot:gone":{"groups":{"old-project":{"source":"human"}}}},"dismissed":{}}"#;
        std::fs::write(tmp.path(), json).unwrap();

        let mgr = GroupManager::open(tmp.path());
        // Should not panic — just returns groups for a key that won't match any live session.
        assert_eq!(mgr.groups_for("copilot:gone"), vec!["old-project".to_string()]);
        assert!(mgr.groups_for("copilot:nonexistent").is_empty());
    }

    #[test]
    fn corrupt_json_falls_back_to_default() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "{{not valid json").unwrap();

        let mgr = GroupManager::open(tmp.path());
        assert!(mgr.all_groups().is_empty());
    }

    #[test]
    fn dismiss_and_check() {
        let tmp = NamedTempFile::new().unwrap();
        let mut mgr = GroupManager::open(tmp.path());
        assert!(!mgr.is_dismissed("copilot:abc", "infra"));
        mgr.dismiss("copilot:abc", "infra");
        assert!(mgr.is_dismissed("copilot:abc", "infra"));
    }
}
