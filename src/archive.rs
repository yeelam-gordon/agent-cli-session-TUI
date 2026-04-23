use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread;
use std::time::Duration;

use anyhow::Result;

/// Write-back buffered archive store.
///
/// Mutations (`archive`, `unarchive`) update the in-memory set instantly and
/// wake a background persist worker. The worker coalesces bursts within a
/// short quiet window and writes the JSON atomically (tmp + rename). This
/// keeps the UI thread responsive under rapid 'a' spam — no matter how
/// many keypresses arrive, at most one write-per-quiet-window hits disk.
///
/// On shutdown, `flush_blocking()` drains any pending write synchronously
/// so no buffered state is lost when the process exits.
///
/// Fallback: if no persist worker has been spawned (e.g. in unit tests),
/// `archive`/`unarchive` fall back to a synchronous write so behaviour
/// remains correct without the worker.
pub struct ArchiveStore {
    path: PathBuf,
    archived: HashSet<String>,
    signal: Option<Arc<PersistSignal>>,
}

struct PersistSignal {
    state: Mutex<PersistState>,
    cvar: Condvar,
}

#[derive(Default)]
struct PersistState {
    dirty: bool,
    shutdown: bool,
}

/// Coalesce window: every mutation within this window after a wake-up
/// piggybacks on the same disk write. Short enough that users never
/// perceive persistence lag; long enough to fold rapid 'a' spam.
const COALESCE_WINDOW: Duration = Duration::from_millis(150);

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
            signal: None,
        })
    }

    /// Spawn the background persist worker. Must be called once after the
    /// store has been wrapped in `Arc<Mutex<Self>>`. Subsequent `archive` /
    /// `unarchive` calls become write-back buffered: they mark dirty and
    /// return immediately; the worker flushes to disk after a short
    /// coalesce window.
    pub fn spawn_persist_worker(arc_self: &Arc<Mutex<Self>>) {
        let signal = Arc::new(PersistSignal {
            state: Mutex::new(PersistState::default()),
            cvar: Condvar::new(),
        });
        {
            let mut s = arc_self.lock().expect("archive mutex poisoned");
            s.signal = Some(signal.clone());
        }
        let weak = Arc::downgrade(arc_self);
        thread::Builder::new()
            .name("archive-persist".into())
            .spawn(move || persist_worker_loop(weak, signal))
            .expect("failed to spawn archive-persist thread");
    }

    /// Check if a session is archived.
    #[allow(dead_code)]
    pub fn is_archived(&self, provider_name: &str, provider_session_id: &str) -> bool {
        self.archived
            .contains(&Self::key(provider_name, provider_session_id))
    }

    /// Archive a session. Write-back buffered when a persist worker is
    /// attached — returns immediately after updating the in-memory set.
    /// Falls back to a synchronous write when no worker is attached.
    pub fn archive(&mut self, provider_name: &str, provider_session_id: &str) -> Result<()> {
        let key = Self::key(provider_name, provider_session_id);
        let was_new = self.archived.insert(key.clone());
        let result = self.mark_dirty_or_save();
        match &result {
            Ok(()) => crate::log::info(&format!(
                "archive: {} (new={}) total={} buffered",
                key,
                was_new,
                self.archived.len()
            )),
            Err(e) => crate::log::warn(&format!(
                "archive: {} buffer/save FAILED: {}",
                key, e
            )),
        }
        result
    }

    /// Unarchive a session. Same buffered behaviour as `archive`.
    pub fn unarchive(&mut self, provider_name: &str, provider_session_id: &str) -> Result<()> {
        let key = Self::key(provider_name, provider_session_id);
        let was_present = self.archived.remove(&key);
        let result = self.mark_dirty_or_save();
        match &result {
            Ok(()) => crate::log::info(&format!(
                "unarchive: {} (was_present={}) total={} buffered",
                key,
                was_present,
                self.archived.len()
            )),
            Err(e) => crate::log::warn(&format!(
                "unarchive: {} buffer/save FAILED: {}",
                key, e
            )),
        }
        result
    }

    /// Return a cheap clone of the archived-key set.
    ///
    /// Used by scan threads to filter active/hidden sessions without
    /// holding the archive mutex for the duration of a multi-second scan.
    pub fn snapshot_keys(&self) -> HashSet<String> {
        self.archived.clone()
    }

    /// Synchronously write the current state to disk and signal the
    /// persist worker to exit. Called on process shutdown so no buffered
    /// mutation is lost when the process dies.
    pub fn flush_blocking(&self) -> Result<()> {
        let save_result = self.save();
        if let Some(signal) = &self.signal {
            if let Ok(mut state) = signal.state.lock() {
                state.dirty = false;
                state.shutdown = true;
                signal.cvar.notify_all();
            }
        }
        match &save_result {
            Ok(()) => crate::log::info(&format!(
                "archive: flush_blocking total={} saved",
                self.archived.len()
            )),
            Err(e) => crate::log::warn(&format!(
                "archive: flush_blocking FAILED: {}",
                e
            )),
        }
        save_result
    }

    fn mark_dirty_or_save(&self) -> Result<()> {
        match &self.signal {
            Some(signal) => {
                if let Ok(mut state) = signal.state.lock() {
                    state.dirty = true;
                    signal.cvar.notify_one();
                }
                Ok(())
            }
            // No worker attached (unit test / early init). Fall back to
            // synchronous write so behaviour is still correct.
            None => self.save(),
        }
    }

    fn key(provider_name: &str, provider_session_id: &str) -> String {
        format!("{}:{}", provider_name, provider_session_id)
    }

    /// Atomic write: serialize → write to `.json.tmp` → `rename` to final
    /// path. Resilient to process kill mid-write and to AV / OneDrive
    /// file-locking windows on Windows (rename is atomic on NTFS).
    fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(&self.archived)?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, &text)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

/// Persist worker loop — runs on a dedicated thread. Wakes on `dirty` or
/// `shutdown`; coalesces bursts of mutations within `COALESCE_WINDOW`;
/// snapshots under a brief outer-mutex lock; writes atomically.
///
/// On write failure, re-marks dirty and backs off before retrying, so
/// transient errors (disk full, AV lock) eventually resolve.
fn persist_worker_loop(weak: Weak<Mutex<ArchiveStore>>, signal: Arc<PersistSignal>) {
    loop {
        // Wait for dirty or shutdown.
        let mut state = match signal.state.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        while !state.dirty && !state.shutdown {
            state = match signal.cvar.wait(state) {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
        }
        if state.shutdown {
            // `flush_blocking` has already written the final state.
            break;
        }
        // Consume dirty; any mutation that arrives after this point and
        // before the next write will set it again and trigger another pass.
        state.dirty = false;
        drop(state);

        // Coalesce: let additional mutations pile into the in-memory set
        // before we take a snapshot. This folds rapid 'a' spam into a
        // single disk write.
        thread::sleep(COALESCE_WINDOW);

        // Upgrade to a strong reference. If the store has been dropped
        // (process shutting down without calling flush_blocking), bail.
        let Some(arc) = weak.upgrade() else { break; };

        // Brief lock to snapshot both the path and the current set.
        let (path, snap) = {
            let guard = match arc.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            (guard.path.clone(), guard.archived.clone())
        };

        match save_snapshot(&path, &snap) {
            Ok(()) => crate::log::info(&format!(
                "archive: persist worker flushed total={}",
                snap.len()
            )),
            Err(e) => {
                crate::log::warn(&format!(
                    "archive: persist worker save FAILED: {} (will retry)",
                    e
                ));
                // Re-mark dirty and back off briefly so transient errors
                // don't spin the CPU.
                if let Ok(mut s) = signal.state.lock() {
                    s.dirty = true;
                    signal.cvar.notify_one();
                }
                thread::sleep(Duration::from_secs(1));
            }
        }
    }
    crate::log::info("archive: persist worker exited");
}

fn save_snapshot(path: &Path, archived: &HashSet<String>) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(archived)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
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
    fn archive_persists_to_disk_without_worker() {
        // With no persist worker attached, archive() falls back to a
        // synchronous write — essential for callers that don't spawn
        // the worker (e.g. one-shot tooling or tests).
        let path = temp_path("persist-sync");
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
    fn archive_persists_via_worker() {
        // With the worker attached, archive() returns immediately and the
        // worker flushes asynchronously. `flush_blocking` on shutdown
        // guarantees the write lands before we re-open.
        let path = temp_path("persist-worker");
        let _ = fs::remove_file(&path);
        {
            let store = ArchiveStore::open(&path).unwrap();
            let arc = Arc::new(Mutex::new(store));
            ArchiveStore::spawn_persist_worker(&arc);
            {
                let mut g = arc.lock().unwrap();
                g.archive("copilot", "sess-async").unwrap();
                g.archive("copilot", "sess-async-2").unwrap();
            }
            // Simulate shutdown: drains the buffer to disk synchronously.
            arc.lock().unwrap().flush_blocking().unwrap();
        }
        let store = ArchiveStore::open(&path).unwrap();
        assert!(store.is_archived("copilot", "sess-async"));
        assert!(store.is_archived("copilot", "sess-async-2"));
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

    #[test]
    fn rapid_archive_coalesces_under_worker() {
        // 100 rapid mutations under the worker should not block the
        // caller for anywhere near 100× the disk-write cost. They should
        // coalesce into a small handful of writes.
        let path = temp_path("coalesce");
        let _ = fs::remove_file(&path);
        let store = ArchiveStore::open(&path).unwrap();
        let arc = Arc::new(Mutex::new(store));
        ArchiveStore::spawn_persist_worker(&arc);
        let start = std::time::Instant::now();
        for i in 0..100 {
            arc.lock()
                .unwrap()
                .archive("copilot", &format!("sess-{i}"))
                .unwrap();
        }
        let elapsed = start.elapsed();
        // Buffered mutations must be near-instant per call — 100 calls
        // well under the single-write latency of a synchronous save.
        assert!(
            elapsed < Duration::from_millis(500),
            "100 buffered mutations took {:?} — expected <500ms",
            elapsed
        );
        arc.lock().unwrap().flush_blocking().unwrap();
        let store = ArchiveStore::open(&path).unwrap();
        assert_eq!(store.archived.len(), 100);
        let _ = fs::remove_file(&path);
    }
}
