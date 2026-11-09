//! argv only. No shell, no string interpolation into a command line.
use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default)]
pub struct Invocation {
    pub args: Vec<OsString>,
}

impl Invocation {
    pub fn new() -> Invocation {
        Invocation {
            args: vec!["ffmpeg".into(), "-y".into(), "-hide_banner".into(), "-loglevel".into(), "error".into()],
        }
    }

    pub fn push(&mut self, a: impl Into<OsString>) -> &mut Self {
        self.args.push(a.into());
        self
    }

    pub fn input(&mut self, path: &Path) -> &mut Self {
        self.push("-i").push(crate::paths::arg(path))
    }

    /// Shell-quoted, for `sona plan` and for humans reading a failure.
    pub fn render(&self) -> String {
        self.args
            .iter()
            .map(|a| {
                let s = a.to_string_lossy();
                if s.is_empty() || s.chars().any(|c| " '\\\"$:|&;<>()*?".contains(c)) {
                    format!("'{}'", s.replace('\'', "'\\''"))
                } else {
                    s.into_owned()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Runs ffmpeg with stderr captured to a file (so a full pipe cannot deadlock)
/// and a hard wall-clock timeout.
pub fn run(inv: &Invocation, timeout: Duration) -> Result<()> {
    let stderr_path = std::env::temp_dir().join(format!("sona-ffmpeg-{}.log", std::process::id()));
    let stderr = File::create(&stderr_path)?;
    let mut child = Command::new(&inv.args[0])
        .args(&inv.args[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr))
        .spawn()
        .context("ffmpeg not found on PATH")?;

    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            if status.success() {
                let _ = std::fs::remove_file(&stderr_path);
                return Ok(());
            }
            let mut log = String::new();
            File::open(&stderr_path)?.read_to_string(&mut log).ok();
            let tail: Vec<&str> = log
                .lines()
                .filter(|l| !l.trim().is_empty())
                .rev()
                .take(3)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let _ = std::fs::remove_file(&stderr_path);
            bail!("ffmpeg failed: {}", tail.join(" | "));
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = std::fs::remove_file(&stderr_path);
            bail!("ffmpeg timed out after {}s", timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}

#[derive(Clone, Debug, Default)]
pub struct Capabilities {
    pub libx264: bool,
    pub libass: bool,
    pub drawtext: bool,
    pub version: String,
}

impl Capabilities {
    pub fn missing(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if !self.libx264 {
            v.push("libx264 encoder");
        }
        if !self.libass {
            v.push("libass (subtitles filter)");
        }
        if !self.drawtext {
            v.push("freetype (drawtext filter)");
        }
        v
    }
}

pub fn capabilities() -> Capabilities {
    let filters = Command::new("ffmpeg")
        .args(["-hide_banner", "-filters"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let encoders = Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let version = Command::new("ffmpeg")
        .arg("-version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).lines().next().map(str::to_string))
        .unwrap_or_default();
    Capabilities {
        libx264: encoders.contains("libx264"),
        libass: filters.contains(" subtitles ") || filters.contains(" subtitles\t"),
        drawtext: filters.contains(" drawtext "),
        version,
    }
}

/// Resolved font file for drawtext, so a render is not at the mercy of a
/// machine's fontconfig defaults without anyone noticing.
pub fn resolve_font() -> Option<String> {
    let out = Command::new("fc-match")
        .args(["-f", "%{file}", "sans:bold"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}
