//! Snapshots of `splice.json`, so a wrong edit is one command away from gone.
//!
//! The document is a single small file, which makes this cheap: before a
//! mutation writes, the previous version is copied into `.splice/history/`. Undo
//! restores it byte for byte and drops the entry, so repeated undo walks back
//! through the session.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::project::{Project, Store};

pub const VERSION: u32 = 1;
/// Enough to walk back a working session without turning the cache into a log.
pub const KEEP: usize = 50;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub version: u32,
    pub seq: u64,
    pub at: u64,
    pub command: String,
    pub project: Project,
}

impl Entry {
    pub fn clips(&self) -> usize {
        self.project.clips().count()
    }

    pub fn overlays(&self) -> usize {
        self.project.tracks.overlays.len()
    }

    pub fn has_captions(&self) -> bool {
        self.project.tracks.captions.is_some()
    }

    pub fn has_index(&self) -> bool {
        self.project.tracks.index.is_some()
    }

    pub fn render(&self) -> String {
        let mut s = serde_json::to_string_pretty(&self.project).unwrap_or_default();
        s.push('\n');
        s
    }

    /// `3m ago`, so a listing reads without a date library.
    pub fn age(&self) -> String {
        let now = now_secs();
        let d = now.saturating_sub(self.at);
        match d {
            0..=1 => "just now".into(),
            2..=59 => format!("{d}s ago"),
            60..=3599 => format!("{}m ago", d / 60),
            3600..=86399 => format!("{}h ago", d / 3600),
            _ => format!("{}d ago", d / 86400),
        }
    }
}

pub fn dir(project: &Path) -> PathBuf {
    project
        .parent()
        .unwrap_or(Path::new("."))
        .join(".splice")
        .join("history")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn entry_files(project: &Path) -> Result<Vec<(u64, PathBuf)>> {
    let d = dir(project);
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(&d) else {
        return Ok(out);
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        if let Some(seq) = p
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.parse::<u64>().ok())
        {
            out.push((seq, p));
        }
    }
    out.sort_by_key(|(seq, _)| *seq);
    Ok(out)
}

/// Snapshot the project as it stands on disk, labelled with the command that is
/// about to change it. Called once per mutation, before the change.
pub fn record(store: &Store, command: &str) -> Result<u64> {
    let files = entry_files(&store.path)?;
    let seq = files.last().map(|(s, _)| *s + 1).unwrap_or(1);
    let entry = Entry {
        version: VERSION,
        seq,
        at: now_secs(),
        command: command.to_string(),
        project: store.project.clone(),
    };
    let d = dir(&store.path);
    std::fs::create_dir_all(&d)?;
    let path = d.join(format!("{seq:05}.json"));
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string(&entry)?)?;
    std::fs::rename(&tmp, &path)?;

    // Prune the oldest beyond the window.
    let files = entry_files(&store.path)?;
    if files.len() > KEEP {
        for (_, p) in files.iter().take(files.len() - KEEP) {
            let _ = std::fs::remove_file(p);
        }
    }
    Ok(seq)
}

/// Newest first.
pub fn list(project: &Path) -> Result<Vec<Entry>> {
    let mut out = Vec::new();
    for (_, path) in entry_files(project)?.into_iter().rev() {
        if let Ok(raw) = std::fs::read_to_string(&path) {
            if let Ok(e) = serde_json::from_str::<Entry>(&raw) {
                out.push(e);
            }
        }
    }
    Ok(out)
}

/// Restore the newest snapshot that differs from what is on disk. Entries that
/// would restore an identical document (a no-op mutation, or a command that
/// failed after its snapshot) are consumed and skipped.
///
/// Deliberately does not open the store: undo has to work when `splice.json` is
/// the thing that got broken.
pub fn undo(project: &Path) -> Result<Option<Entry>> {
    let current = std::fs::read_to_string(project).unwrap_or_default();
    let entries = list(project)?;
    if entries.is_empty() {
        return Ok(None);
    }
    for entry in entries {
        let restored = entry.render();
        let path = dir(project).join(format!("{:05}.json", entry.seq));
        if restored == current {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        let tmp = project.with_extension("json.tmp");
        std::fs::write(&tmp, &restored)
            .with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, project)
            .with_context(|| format!("replacing {}", project.display()))?;
        let _ = std::fs::remove_file(&path);
        return Ok(Some(entry));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Store;
    use crate::time::Rational;

    fn scratch() -> (PathBuf, Store) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "splice-hist-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::create(&dir.join("splice.json"), 320, 240, Rational::new(30, 1)).unwrap();
        (dir, store)
    }

    #[test]
    fn undo_restores_the_previous_document_and_pops() {
        let (dir, store) = scratch();
        record(&store, "add").unwrap();
        let mut changed = store.project.clone();
        changed.canvas.width = 640;
        std::fs::write(&store.path, serde_json::to_string_pretty(&changed).unwrap() + "\n").unwrap();

        let restored = undo(&store.path).unwrap().expect("an entry");
        assert_eq!(restored.command, "add");
        let back: Project =
            serde_json::from_str(&std::fs::read_to_string(&store.path).unwrap()).unwrap();
        assert_eq!(back.canvas.width, 320, "restored the pre-add canvas");

        // the entry is consumed, so a second undo has nothing left
        assert!(undo(&store.path).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn identical_snapshots_are_skipped_and_dropped() {
        let (dir, store) = scratch();
        record(&store, "noop-1").unwrap();
        record(&store, "noop-2").unwrap();
        // nothing changed on disk, so there is nothing to go back to
        assert!(undo(&store.path).unwrap().is_none());
        assert!(list(&store.path).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn undo_works_on_a_broken_document() {
        let (dir, store) = scratch();
        record(&store, "add").unwrap();
        std::fs::write(&store.path, "{ this is not json").unwrap();
        assert!(undo(&store.path).unwrap().is_some());
        let _: Project =
            serde_json::from_str(&std::fs::read_to_string(&store.path).unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn listing_is_newest_first_with_labels() {
        let (dir, store) = scratch();
        record(&store, "add").unwrap();
        record(&store, "trim").unwrap();
        let l = list(&store.path).unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].command, "trim");
        assert_eq!(l[1].command, "add");
        assert!(!l[0].age().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
