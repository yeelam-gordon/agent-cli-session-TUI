use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;

/// Simple JSON-based archive store for tracking hidden/archived session IDs.
/// Stores only the set of archived session IDs — everything else
/// comes fresh from provider scans.
pub struct ArchiveStore {
    path: PathBuf,
    archived: HashSet<String>,
}

impl ArchiveStore {
    pub fn open(path: &Path) -> Result<Self> {
        let archived = if path.exists() {
            let text = std::fs::read_to_string(path)?;
            serde_json::from_str(&text).unwrap_or_default()
        } else {
            HashSet::new()
        };
        Ok(Self {
            path: path.to_path_buf(),
            archived,
        })
    }

    /// Check if a session is archived.
    pub fn is_archived(&self, provider_name: &str, provider_session_id: &str) -> bool {
        self.archived
            .contains(&Self::key(provider_name, provider_session_id))
    }

    /// Archive a session.
    pub fn archive(&mut self, provider_name: &str, provider_session_id: &str) -> Result<()> {
        self.archived
            .insert(Self::key(provider_name, provider_session_id));
        self.save()
    }

    /// Unarchive a session.
    #[allow(dead_code)]
    pub fn unarchive(&mut self, provider_name: &str, provider_session_id: &str) -> Result<()> {
        self.archived
            .remove(&Self::key(provider_name, provider_session_id));
        self.save()
    }

    fn key(provider_name: &str, provider_session_id: &str) -> String {
        format!("{}:{}", provider_name, provider_session_id)
    }

    fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(&self.archived)?;
        std::fs::write(&self.path, text)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("agent-session-tui-test");
        fs::create_dir_all(&dir).unwrap();
        dir.join(format!("{}-{}.json", name, std::process::id()))
    }

    #[test]
    fn open_nonexistent_creates_empty() {
        let path = temp_path("empty");
        let _ = fs::remove_file(&path);
        let store = ArchiveStore::open(&path).unwrap();
        assert!(!store.is_archived("copilot", "abc123"));
    }

    #[test]
    fn archive_and_check() {
        let path = temp_path("check");
        let _ = fs::remove_file(&path);
        let mut store = ArchiveStore::open(&path).unwrap();
        store.archive("copilot", "abc123").unwrap();
        assert!(store.is_archived("copilot", "abc123"));
        assert!(!store.is_archived("copilot", "other"));
        assert!(!store.is_archived("claude", "abc123"));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn archive_persists_to_disk() {
        let path = temp_path("persist");
        let _ = fs::remove_file(&path);
        {
            let mut store = ArchiveStore::open(&path).unwrap();
            store.archive("copilot", "sess1").unwrap();
        }
        let store = ArchiveStore::open(&path).unwrap();
        assert!(store.is_archived("copilot", "sess1"));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn unarchive_removes() {
        let path = temp_path("unarchive");
        let _ = fs::remove_file(&path);
        let mut store = ArchiveStore::open(&path).unwrap();
        store.archive("copilot", "abc123").unwrap();
        assert!(store.is_archived("copilot", "abc123"));
        store.unarchive("copilot", "abc123").unwrap();
        assert!(!store.is_archived("copilot", "abc123"));
        let _ = fs::remove_file(&path);
    }
}
