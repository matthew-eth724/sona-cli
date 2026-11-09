//! ffprobe, cached. Every media fact the compiler needs comes from here.
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::time::{Rational, Time};

#[derive(Clone, Debug)]
pub struct Media {
    pub path: PathBuf,
    pub duration: Time,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<Rational>,
    pub has_video: bool,
    pub has_audio: bool,
}

#[derive(Default)]
pub struct MediaIndex {
    cache: HashMap<PathBuf, Option<Media>>,
}

impl MediaIndex {
    pub fn get(&mut self, path: &Path) -> Result<&Media> {
        let key = path.to_path_buf();
        if !self.cache.contains_key(&key) {
            let probed = probe(path).ok();
            self.cache.insert(key.clone(), probed);
        }
        match self.cache.get(&key) {
            Some(Some(m)) => Ok(m),
            _ => bail!("cannot probe `{}`", path.display()),
        }
    }
}

#[derive(Deserialize)]
struct ProbeOut {
    #[serde(default)]
    format: Option<ProbeFormat>,
    #[serde(default)]
    streams: Vec<ProbeStream>,
}

#[derive(Deserialize)]
struct ProbeFormat {
    #[serde(default)]
    duration: Option<String>,
}

#[derive(Deserialize)]
struct ProbeStream {
    #[serde(default)]
    codec_type: String,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    avg_frame_rate: Option<String>,
}

pub fn probe(path: &Path) -> Result<Media> {
    if !path.exists() {
        bail!("media_missing: {} does not exist", path.display());
    }
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(path)
        .output()
        .context("ffprobe not found on PATH")?;
    if !out.status.success() {
        bail!("unreadable media: {}", path.display());
    }
    let parsed: ProbeOut =
        serde_json::from_slice(&out.stdout).context("ffprobe returned unparseable json")?;
    let duration = parsed
        .format
        .and_then(|f| f.duration)
        .and_then(|d| d.parse::<f64>().ok())
        .map(Time::from_secs_f64)
        .unwrap_or(Time::ZERO);
    let video = parsed.streams.iter().find(|s| s.codec_type == "video");
    let audio = parsed.streams.iter().any(|s| s.codec_type == "audio");
    Ok(Media {
        path: path.to_path_buf(),
        duration,
        width: video.and_then(|s| s.width),
        height: video.and_then(|s| s.height),
        fps: video
            .and_then(|s| s.avg_frame_rate.as_deref())
            .and_then(parse_rate),
        has_video: video.is_some(),
        has_audio: audio,
    })
}

fn parse_rate(s: &str) -> Option<Rational> {
    let (a, b) = s.split_once('/')?;
    let (num, den): (u32, u32) = (a.parse().ok()?, b.parse().ok()?);
    if num == 0 || den == 0 {
        return None;
    }
    Some(Rational::new(num, den))
}
