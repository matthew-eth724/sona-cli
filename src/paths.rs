//! The only place a filesystem path becomes bytes of an ffmpeg argument.
//!
//! Two rules, one function each. Inputs pass as `OsString` and are never quoted
//! or re-encoded. Filter strings get a *relative* POSIX path with filter
//! metacharacters escaped, because `C:\` inside `filter_complex` is read as
//! `C` followed by an escape.
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

pub fn arg(p: &Path) -> OsString {
    p.as_os_str().to_os_string()
}

/// Path as ffmpeg's filter parser must see it: relative to `cwd` when possible,
/// forward slashes, `\ : '` escaped, wrapped in single quotes.
pub fn filter_escape(path: &Path, cwd: &Path) -> String {
    let rel = relative_to(path, cwd);
    let raw = rel.to_string_lossy().replace('\\', "/");
    let mut s = String::new();
    for c in raw.chars() {
        match c {
            ':' => s.push_str("\\:"),
            '\'' => s.push_str("\\'"),
            _ => s.push(c),
        }
    }
    format!("'{s}'")
}

fn relative_to(path: &Path, cwd: &Path) -> PathBuf {
    if let Ok(stripped) = path.strip_prefix(cwd) {
        return normalize(stripped);
    }
    normalize(path)
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.push("..");
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn input_args_are_verbatim() {
        assert_eq!(arg(&p("/music/clip one.mp4")), OsString::from("/music/clip one.mp4"));
    }

    #[test]
    fn paths_inside_project_become_relative() {
        let cwd = p("/work/proj");
        assert_eq!(
            filter_escape(&p("/work/proj/.splice/cache/x.srt"), &cwd),
            "'.splice/cache/x.srt'"
        );
    }

    #[test]
    fn windows_drive_letters_are_escaped() {
        // The bug this exists for: unescaped `C:` makes ffmpeg report
        // "No such filter" instead of a missing file.
        let cwd = p("/work/proj");
        let out = filter_escape(&p("C:\\videos\\my clip.mp4"), &cwd);
        assert_eq!(out, "'C\\:/videos/my clip.mp4'");
    }

    #[test]
    fn quotes_and_backslashes_survive() {
        let cwd = p("/w");
        assert!(filter_escape(&p("/w/a'b.srt"), &cwd).contains("\\'"));
    }
}
