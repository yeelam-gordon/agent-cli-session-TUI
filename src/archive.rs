use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;

/// Simple JSON-based archive store. Replaces the SQLite sessions.db.
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
        self.archived.contains(&Self::key(provider_name, provider_session_id))
    }

    /// Archive a session.
    pub fn archive(&mut self, provider_name: &str, provider_session_id: &str) -> Result<()> {
        self.archived.insert(Self::key(provider_name, provider_session_id));
        self.save()
    }

    /// Unarchive a session.
    #[allow(dead_code)]
    pub fn unarchive(&mut self, provider_name: &str, provider_session_id: &str) -> Result<()> {
        self.archived.remove(&Self::key(provider_name, provider_session_id));
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
