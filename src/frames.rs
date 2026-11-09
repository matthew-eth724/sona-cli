//! Stills and contact sheets: how an agent actually sees its own output.
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::ffmpeg::Invocation;
use crate::paths::arg;
use crate::time::Time;

pub const REVIEW_WIDTH: u32 = 768;

/// One PNG per timestamp, scaled for a vision model to read cheaply.
pub fn stills(source: &Path, times: &[Time], out_dir: &Path) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(out_dir)?;
    let mut out = Vec::new();
    for t in times {
        let stamp = format!("{:09.3}", t.secs_f64()).replace('.', "-");
        let path = out_dir.join(format!("at-{stamp}.png"));
        let mut inv = Invocation::new();
        inv.push("-ss").push(format!("{:.3}", t.secs_f64()));
        inv.input(source);
        inv.push("-frames:v").push("1");
        inv.push("-vf")
            .push(format!("scale={REVIEW_WIDTH}:-2:force_original_aspect_ratio=decrease"));
        inv.push(arg(&path));
        crate::ffmpeg::run(&inv, std::time::Duration::from_secs(120))
            .with_context(|| format!("frame at {t}"))?;
        out.push(path);
    }
    Ok(out)
}

/// A grid of `count` evenly spaced frames with timecode burnt in.
pub fn contact_sheet(
    source: &Path,
    count: u32,
    duration: Time,
    out_path: &Path,
) -> Result<PathBuf> {
    let cols = 4u32;
    let rows = ((count as f64 / cols as f64).ceil() as u32).max(1);
    let fps = (count as f64 / duration.secs_f64().max(0.001)).max(0.01);
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let filter = format!(
        "fps={fps:.4},scale=480:-2,drawtext=font=sans:text='%{{pts\\:hms}}':fontsize=22:fontcolor=white:box=1:boxcolor=black@0.6:x=8:y=8,tile={cols}x{rows}"
    );
    let mut inv = Invocation::new();
    inv.input(source);
    inv.push("-vf").push(filter);
    inv.push("-frames:v").push("1");
    inv.push(arg(out_path));
    crate::ffmpeg::run(&inv, std::time::Duration::from_secs(180))
        .context("contact sheet")?;
    Ok(out_path.to_path_buf())
}

/// `25%`, `2s`, `1:02`.
pub fn parse_frame_times(raw: &str, duration: Time) -> Result<Vec<Time>> {
    let mut out = Vec::new();
    for piece in raw.split(',') {
        let p = piece.trim();
        if p.is_empty() {
            continue;
        }
        if let Some(pct) = p.strip_suffix('%') {
            let frac: f64 = pct.trim().parse().context("bad percentage")?;
            out.push(Time::from_secs_f64(duration.secs_f64() * frac / 100.0));
        } else {
            out.push(Time::parse(p)?);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentages_resolve_against_duration() {
        let d = Time::from_secs_f64(100.0);
        let t = parse_frame_times("25%,50", d).unwrap();
        assert_eq!(t[0].micros(), 25_000_000);
        assert_eq!(t[1].micros(), 50_000_000);
    }
}
