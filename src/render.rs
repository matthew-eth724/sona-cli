//! Executes a plan, reusing cache entries whose key matches.
//!
//! Progress goes to stderr as one compact JSON object per line, so an agent
//! whose stderr is a pipe can tell a long render from a hung process. stdout
//! keeps exactly one JSON object.
use anyhow::{bail, Context, Result};
use std::time::{Duration, Instant};

use crate::graph::RenderPlan;

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct Summary {
    pub total: usize,
    pub ran: usize,
    pub reused: usize,
}

fn complete(path: &std::path::Path) -> bool {
    std::fs::metadata(path).map(|m| m.len() > 0).unwrap_or(false)
}

/// Render progress, reported on stderr as one compact JSON object per line.
///
/// `step` lines (start, then `ran`/`reused`) are always emitted unless the
/// progress is silenced; ffmpeg `out_time_us` tick lines only when `ticks()`
/// is true.
pub struct Progress {
    steps: bool,
    ticks: bool,
}

impl Progress {
    /// Nothing at all: `--no-progress`, `SPLICE_PROGRESS=0`.
    pub fn silent() -> Progress {
        Progress { steps: false, ticks: false }
    }

    /// Step lines always; ffmpeg tick lines only when `ticks`, which the
    /// caller decides from a TTY, `SPLICE_PROGRESS=verbose`, or `--progress`.
    pub fn watch(ticks: bool) -> Progress {
        Progress { steps: true, ticks }
    }

    pub fn ticks(&self) -> bool {
        self.ticks
    }

    /// Step lines always. `#[cfg(test)]` so cli.rs can assert the gate.
    #[cfg(test)]
    pub fn steps(&self) -> bool {
        self.steps
    }

    /// One compact JSON line on stderr, flushed before returning. No-op when
    /// `!self.steps`.
    pub fn step(&self, index: usize, total: usize, label: &str, state: &str, ms: Option<u128>) {
        if !self.steps {
            return;
        }
        eprintln!("{}", step_line(index, total, label, state, ms));
    }

    /// No-op when `!self.ticks`.
    pub fn ffmpeg(&self, index: usize, total: usize, out_time_us: i64) {
        if !self.ticks {
            return;
        }
        eprintln!("{}", ffmpeg_line(index, total, out_time_us));
    }
}

/// One compact JSON object per line, keys in the documented order.
fn step_line(index: usize, total: usize, label: &str, state: &str, ms: Option<u128>) -> String {
    let label = serde_json::to_string(label).unwrap();
    let state = serde_json::to_string(state).unwrap();
    match ms {
        Some(ms) => format!(
            r##"{{"progress":"step","index":{index},"total":{total},"label":{label},"state":{state},"ms":{ms}}}"##
        ),
        None => format!(
            r##"{{"progress":"step","index":{index},"total":{total},"label":{label},"state":{state}}}"##
        ),
    }
}

fn ffmpeg_line(index: usize, total: usize, out_time_us: i64) -> String {
    format!(
        r##"{{"progress":"ffmpeg","index":{index},"total":{total},"out_time_us":{out_time_us}}}"##
    )
}

pub fn execute(plan: &RenderPlan, timeout: Duration, progress: &Progress) -> Result<Summary> {
    let mut summary = Summary {
        total: plan.steps.len(),
        ..Default::default()
    };
    for (index, step) in plan.steps.iter().enumerate() {
        let index = index + 1; // 1-based, for the progress lines
        progress.step(index, summary.total, &step.label, "start", None);
        if let Some(side) = &step.sidecar {
            crate::cache::write(&side.path, &side.body)
                .with_context(|| format!("writing {}", side.path.display()))?;
        }
        if complete(&step.output) {
            summary.reused += 1;
            progress.step(index, summary.total, &step.label, "reused", Some(0));
            continue;
        }
        // ffmpeg creates its output file before it fails. A failed step must not
        // leave a zero-byte artefact that the next run trusts as a cache hit.
        let _ = std::fs::remove_file(&step.output);
        let started = Instant::now();
        let result = if progress.ticks() {
            let mut forward = |line: &str| {
                if let Some(us) = line
                    .strip_prefix("out_time_us=")
                    .and_then(|s| s.trim().parse::<i64>().ok())
                {
                    progress.ffmpeg(index, summary.total, us);
                }
            };
            crate::ffmpeg::run_with_progress(&step.invocation, timeout, &mut forward)
                .with_context(|| format!("step `{}` failed", step.label))
        } else {
            crate::ffmpeg::run(&step.invocation, timeout)
                .with_context(|| format!("step `{}` failed", step.label))
        };
        if let Err(e) = result {
            let _ = std::fs::remove_file(&step.output);
            return Err(e);
        }
        if !complete(&step.output) {
            bail!("step `{}` produced no usable output", step.label);
        }
        let ms = Some(started.elapsed().as_millis());
        summary.ran += 1;
        progress.step(index, summary.total, &step.label, "ran", ms);
    }
    if let Some(parent) = plan.out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::copy(plan.last_output(), &plan.out)
        .with_context(|| format!("copying result to {}", plan.out.display()))?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_line_parses_back_and_carries_the_documented_keys() {
        let line = step_line(1, 4, "normalize clip_1", "start", None);
        assert_eq!(
            line,
            r#"{"progress":"step","index":1,"total":4,"label":"normalize clip_1","state":"start"}"#
        );
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v.get("progress").and_then(|v| v.as_str()), Some("step"));
        assert_eq!(v.get("index").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(v.get("total").and_then(|v| v.as_u64()), Some(4));
        assert_eq!(v.get("label").and_then(|v| v.as_str()), Some("normalize clip_1"));
        assert_eq!(v.get("state").and_then(|v| v.as_str()), Some("start"));
        assert!(v.get("ms").is_none());

        let line = step_line(1, 4, "normalize clip_1", "ran", Some(1234));
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v.get("ms").and_then(|v| v.as_u64()), Some(1234));

        let line = step_line(2, 4, "composite 2 layers", "reused", Some(0));
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v.get("state").and_then(|v| v.as_str()), Some("reused"));
        assert_eq!(v.get("ms").and_then(|v| v.as_u64()), Some(0));
    }

    #[test]
    fn ffmpeg_line_carries_out_time_us() {
        let line = ffmpeg_line(1, 4, 1500000);
        assert_eq!(
            line,
            r#"{"progress":"ffmpeg","index":1,"total":4,"out_time_us":1500000}"#
        );
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v.get("progress").and_then(|v| v.as_str()), Some("ffmpeg"));
        assert_eq!(v.get("index").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(v.get("total").and_then(|v| v.as_u64()), Some(4));
        assert_eq!(v.get("out_time_us").and_then(|v| v.as_i64()), Some(1500000));
    }

    #[test]
    fn watch_gates_steps_and_ticks() {
        assert!(Progress::watch(false).steps());
        assert!(!Progress::watch(false).ticks());
        assert!(Progress::watch(true).steps());
        assert!(Progress::watch(true).ticks());
        assert!(!Progress::silent().steps());
        assert!(!Progress::silent().ticks());
    }
}
