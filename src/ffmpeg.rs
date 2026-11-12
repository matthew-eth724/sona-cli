//! argv only. No shell, no string interpolation into a command line.
use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default)]
pub struct Invocation {
    pub args: Vec<OsString>,
}

/// Where an ffmpeg tool came from, so `doctor` can say it.
#[derive(Clone, Debug, PartialEq)]
pub enum ProgramSource {
    Env,
    BesideBinary,
    Path,
}

/// Resolves `ffmpeg`/`ffprobe`, in the order that makes a bundled copy win:
/// `SPLICE_FFMPEG` (a directory or a file), then a sibling of this executable,
/// then whatever is on PATH.
///
/// The sibling rule is what lets the npm package ship its own ffmpeg: put it in
/// `bin/` next to `splice` and it is found without touching PATH or the caller.
pub fn program(name: &str) -> (PathBuf, ProgramSource) {
    let exe_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    if let Ok(dir_or_file) = std::env::var("SPLICE_FFMPEG") {
        let p = PathBuf::from(dir_or_file);
        if p.is_dir() {
            let candidate = p.join(&exe_name);
            if candidate.is_file() {
                return (candidate, ProgramSource::Env);
            }
        } else if p.is_file() && name == "ffmpeg" {
            return (p, ProgramSource::Env);
        } else if p.is_file() {
            // SPLICE_FFMPEG pointing at ffmpeg: look for its neighbour for probe
            if let Some(dir) = p.parent() {
                let candidate = dir.join(&exe_name);
                if candidate.is_file() {
                    return (candidate, ProgramSource::Env);
                }
            }
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(&exe_name);
            if candidate.is_file() {
                return (candidate, ProgramSource::BesideBinary);
            }
        }
    }
    (PathBuf::from(exe_name), ProgramSource::Path)
}

pub fn ffmpeg() -> PathBuf {
    program("ffmpeg").0
}

pub fn ffprobe() -> PathBuf {
    program("ffprobe").0
}

impl Invocation {
    pub fn new() -> Invocation {
        Invocation {
            args: vec![
                program("ffmpeg").0.into_os_string(),
                "-y".into(),
                "-hide_banner".into(),
                "-loglevel".into(),
                "error".into(),
            ],
        }
    }

    pub fn push(&mut self, a: impl Into<OsString>) -> &mut Self {
        self.args.push(a.into());
        self
    }

    pub fn input(&mut self, path: &Path) -> &mut Self {
        self.push("-i").push(crate::paths::arg(path))
    }

    /// Shell-quoted, for `splice plan` and for humans reading a failure.
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
    let stderr_path = std::env::temp_dir().join(format!("splice-ffmpeg-{}.log", std::process::id()));
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

/// Keys ffmpeg emits on stderr that are progress telemetry, not real errors.
/// They must not displace the actual failure line in the error tail.
const PROGRESS_KEYS: &[&str] = &[
    "out_time_us=", "out_time_ms=", "out_time=", "speed=", "progress=",
    "frame=", "fps=", "bitrate=", "total_size=", "dup_frames=", "drop_frames=",
    "stream_",
];

fn is_progress_line(line: &str) -> bool {
    PROGRESS_KEYS.iter().any(|k| line.starts_with(k))
}

/// `run`, plus ffmpeg `-progress pipe:2` blocks delivered line-by-line to
/// `on_line`. The three progress options are appended at spawn time only;
/// `inv` itself is never mutated, so `plan` output and cache keys are
/// unchanged. stderr is drained on a thread into a bounded buffer (the last
/// 200 lines) so a full pipe cannot deadlock the child, and the buffer is the
/// error tail when a step fails.
pub fn run_with_progress(
    inv: &Invocation,
    timeout: Duration,
    mut on_line: &mut dyn FnMut(&str),
) -> Result<()> {
    let mut args: Vec<OsString> = inv.args[1..].to_vec();
    args.push("-progress".into());
    args.push("pipe:2".into());
    args.push("-nostats".into());
    let mut child = Command::new(&inv.args[0])
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("ffmpeg not found on PATH")?;

    let stderr = child.stderr.take().expect("stderr is piped");
    let (tx, rx) = std::sync::mpsc::channel();
    // The last 200 non-progress lines on stderr, kept for the error tail.
    let tail = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let tail_thread = tail.clone();
    let reader = std::thread::spawn(move || {
        let mut buf = std::io::BufReader::new(stderr);
        let mut line = String::new();
        let tail = tail_thread;
        loop {
            line.clear();
            match buf.read_line(&mut line) {
                Ok(0) => return,
                Ok(_) => {
                    let end = line.trim_end().to_string();
                    if !is_progress_line(&end) {
                        let mut t = tail.lock().unwrap();
                        t.push(end.clone());
                        if t.len() > 200 {
                            t.remove(0);
                        }
                    }
                    let _ = tx.send(end);
                }
                Err(_) => return,
            }
        }
    });

    let start = Instant::now();
    loop {
        while let Ok(line) = rx.try_recv() {
            if line.starts_with("out_time_us=") || line.starts_with("progress=") {
                on_line(&line);
            }
        }
        if let Some(status) = child.try_wait()? {
            for line in rx.try_iter() {
                if line.starts_with("out_time_us=") || line.starts_with("progress=") {
                    on_line(&line);
                }
            }
            if status.success() {
                return Ok(());
            }
            // The child has exited, so EOF is guaranteed: join the reader
            // before reading the tail so nothing is missing or truncated.
            let _ = reader.join();
            let tail = tail.lock().unwrap();
            let tail: Vec<&str> = tail
                .iter()
                .map(|s| s.as_str())
                .filter(|l| !l.trim().is_empty())
                .rev()
                .take(3)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            bail!("ffmpeg failed: {}", tail.join(" | "));
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            bail!("ffmpeg timed out after {}s", timeout.as_secs());
        }
        match rx.recv_timeout(Duration::from_millis(40)) {
            Ok(line) => {
                if line.starts_with("out_time_us=") || line.starts_with("progress=") {
                    on_line(&line);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                // stderr hit EOF without the child reaped yet: sleep once to
                // match run's poll interval instead of busy-waiting.
                std::thread::sleep(Duration::from_millis(40));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
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
    /// What the tool actually needs. `drawtext` used to be on this list; titles
    /// are ASS now, so a build without it is fine — reporting it as a failure
    /// would reject the very binary this tool now ships.
    pub fn missing(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if !self.libx264 {
            v.push("libx264 encoder");
        }
        if !self.libass {
            v.push("libass (subtitles and ass filters)");
        }
        v
    }
}

pub fn capabilities() -> Capabilities {
    let filters = Command::new(ffmpeg())
        .args(["-hide_banner", "-filters"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let encoders = Command::new(ffmpeg())
        .args(["-hide_banner", "-encoders"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let version = Command::new(ffmpeg())
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

/// Where the font came from, so `doctor` can say so instead of guessing.
/// The family name libass needs in an ASS style, from the same lookup that
/// resolves the file for anything that wants a path.
pub fn font_family() -> String {
    use std::sync::OnceLock;
    static FAMILY: OnceLock<String> = OnceLock::new();
    FAMILY
        .get_or_init(|| {
            let out = Command::new("fc-match")
                .args(["-f", "%{family}", "sans"])
                .output();
            if let Ok(out) = out {
                if out.status.success() {
                    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    // fc-match returns a comma-separated preference list
                    if let Some(first) = s.split(',').next() {
                        if !first.trim().is_empty() {
                            return first.trim().to_string();
                        }
                    }
                }
            }
            "sans".to_string()
        })
        .clone()
}

pub fn font_pick() -> Option<(&'static str, String)> {
    use std::sync::OnceLock;
    static PICK: OnceLock<Option<(&'static str, String)>> = OnceLock::new();
    PICK.get_or_init(pick_font).clone()
}

pub fn resolve_font() -> Option<String> {
    font_pick().map(|(_, path)| path)
}

/// Font resolution in the order that respects the machine, then falls back to
/// fonts that ship with the OS.
///
/// `fc-match` is a fontconfig tool: present on Linux, absent on Windows unless
/// MSYS is installed, and often absent on macOS. Relying on it alone meant text
/// overlays failed on Windows with `no_font` while the rest of the tool worked.
fn pick_font() -> Option<(&'static str, String)> {
    if let Ok(p) = std::env::var("SPLICE_FONT") {
        if Path::new(&p).is_file() {
            return Some(("SPLICE_FONT", p));
        }
    }
    if let Ok(out) = Command::new("fc-match").args(["-f", "%{file}", "sans:bold"]).output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() && Path::new(&s).is_file() {
                return Some(("fc-match", s));
            }
        }
    }
    first_existing(&font_candidates()).map(|p| ("built-in path", p))
}

/// Fonts that a fresh install of each OS already has. No download, no config.
pub fn font_candidates() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        let root = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
        for f in ["segoeui.ttf", "arial.ttf", "calibri.ttf", "tahoma.ttf"] {
            v.push(PathBuf::from(&root).join("Fonts").join(f));
        }
    }
    if cfg!(target_os = "macos") {
        for p in [
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            "/System/Library/Fonts/Helvetica.ttc",
            "/Library/Fonts/Arial.ttf",
            "/System/Library/Fonts/Supplemental/Verdana.ttf",
        ] {
            v.push(PathBuf::from(p));
        }
    }
    for p in [
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
    ] {
        v.push(PathBuf::from(p));
    }
    v
}

pub fn first_existing(paths: &[PathBuf]) -> Option<String> {
    paths
        .iter()
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_include_the_platform_it_was_built_for() {
        let c = font_candidates();
        assert!(!c.is_empty());
        if cfg!(windows) {
            assert!(c.iter().any(|p| p.to_string_lossy().contains("Windows")));
        }
        if cfg!(target_os = "macos") {
            assert!(c.iter().any(|p| p.to_string_lossy().contains("/System/Library/Fonts")));
        }
        assert!(c.iter().any(|p| p.to_string_lossy().contains("dejavu")));
    }

    #[test]
    fn first_existing_takes_the_first_real_file() {
        let dir = std::env::temp_dir().join(format!("splice-font-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let real = dir.join("a.ttf");
        std::fs::write(&real, b"not a real font").unwrap();
        let missing = dir.join("gone.ttf");
        let got = first_existing(&[missing.clone(), real.clone()]);
        assert_eq!(got.as_deref(), Some(real.to_str().unwrap()));
        assert!(first_existing(&[missing]).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn progress_keys_filter_all_thirteen_real_ffmpeg_keys() {
        let real = [
            "frame=12",
            "fps=0.0",
            "stream_0_0_q=-1.0",
            "stream_0_1_q=-1.0",
            "bitrate=N/A",
            "total_size=N/A",
            "out_time_us=2000000",
            "out_time_ms=2000000",
            "out_time=00:00:02.000000",
            "dup_frames=0",
            "drop_frames=0",
            "speed=1.02x",
            "progress=end",
        ];
        for line in real {
            assert!(is_progress_line(line), "{line} must be treated as progress");
        }
    }

    #[test]
    fn progress_keys_do_not_swallow_real_errors() {
        let errors = [
            "Error while opening encoder for output stream #0:0",
            "Failed to open file 'nonexistent.mp4'",
            "Invalid argument",
        ];
        for err in errors {
            assert!(!is_progress_line(err), "{err} must not look like progress");
        }
    }
}
