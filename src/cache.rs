//! Content-addressed step cache. `.sona/` is never authoritative: delete it any
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
