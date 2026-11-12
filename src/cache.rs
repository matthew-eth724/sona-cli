//! Content-addressed step cache. `.splice/` is never authoritative: delete it any
//! time and the next render rebuilds it.
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub fn hash(parts: &[String]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update([0u8]);
    }
    let digest: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    digest[..16].to_string()
}

/// Identity of an input file: path, size, mtime. Cheap, and enough to notice a
/// re-exported source without hashing gigabytes.
pub fn stat_token(path: &Path) -> String {
    match std::fs::metadata(path) {
        Ok(m) => {
            let mtime = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis())
                .unwrap_or(0);
            format!("{}|{}|{}", path.display(), m.len(), mtime)
        }
        Err(_) => format!("{}|missing", path.display()),
    }
}

pub fn path(cache_dir: &Path, key: &str, ext: &str) -> PathBuf {
    cache_dir.join(format!("{key}.{ext}"))
}

pub fn write(path: &Path, body: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, body)
}

/// `(bytes, files)` under `dir`. A missing directory is `(0, 0)`, not an
/// error: `.splice/` is never authoritative, so `cache info` must not fail
/// just because nothing has been rendered yet.
///
/// Only regular files count: `symlink_metadata` does not follow symlinks, so
/// a link to a large file outside the cache never inflates the numbers, and a
/// dangling link is simply skipped (it stays on disk; `remove_file` unlinks
/// links themselves, never their targets).
pub fn usage(dir: &Path) -> (u64, u64) {
    let mut bytes = 0u64;
    let mut files = 0u64;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = std::fs::symlink_metadata(&path) else { continue };
            if metadata.file_type().is_file() {
                bytes += metadata.len();
                files += 1;
            }
        }
    }
    (bytes, files)
}

/// Every regular file directly under `dir`; the cache is flat. Unreadable
/// entries are skipped, not fatal. Symlinks are never listed, so they are
/// never reclaimed by prune either.
pub fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = std::fs::symlink_metadata(&path) else { continue };
            if metadata.file_type().is_file() {
                out.push(path);
            }
        }
    }
    out
}

/// `doctor` warns above this. Advice, not a limit: the cache is never
/// authoritative, so this only ever costs a re-render.
pub const WARN_BYTES: u64 = 1 << 30;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_of_a_missing_dir_is_zero_not_an_error() {
        let gone = std::env::temp_dir().join(format!("splice-cache-usage-{}", std::process::id()));
        assert_eq!(usage(&gone), (0, 0));
    }

    #[test]
    fn usage_counts_a_written_file_at_its_real_size() {
        let dir = std::env::temp_dir().join(format!("splice-cache-usage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("deadbeef.mp4");
        std::fs::write(&file, b"hello cache").unwrap();
        assert_eq!(usage(&dir), (11, 1));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
