//! The project document: the whole state, one writer.
use crate::media::MediaIndex;
use crate::report::Diagnostic;
use crate::time::{Rational, Time, TimeRange};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub version: u32,
    pub canvas: Canvas,
    pub tracks: Tracks,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub fps: Rational,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Tracks {
    #[serde(default)]
    pub video: Vec<Clip>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioBed>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overlays: Vec<Overlay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captions: Option<Captions>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Clip {
    pub id: String,
    pub source: String,
    pub src_in: Time,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src_out: Option<Time>,
    #[serde(default = "one")]
    pub speed: f64,
    #[serde(default)]
    pub transform: Transform,
    #[serde(default)]
    pub gain_db: f32,
}

fn one() -> f64 {
    1.0
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transform {
    #[default]
    Fit,
    Fill,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudioBed {
    pub source: String,
    #[serde(default)]
    pub gain_db: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Overlay {
    Text(TextOverlay),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextOverlay {
    pub id: String,
    pub content: String,
    pub at: TimeRange,
    #[serde(default)]
    pub anchor: Anchor,
    #[serde(default = "default_size")]
    pub size: u32,
    #[serde(default = "default_color")]
    pub color: String,
}

fn default_size() -> u32 {
    48
}
fn default_color() -> String {
    "white".into()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    Top,
    #[default]
    Bottom,
    Center,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Captions {
    pub source: String,
    #[serde(default = "yes")]
    pub burn: bool,
}

fn yes() -> bool {
    true
}

impl Project {
    pub fn new(width: u32, height: u32, fps: Rational) -> Project {
        Project {
            version: VERSION,
            canvas: Canvas { width, height, fps },
            tracks: Tracks::default(),
        }
    }

    /// Total program duration: clips in order, each divided by its speed.
    /// `src_out: None` means "to the end of the source", so this needs media.
    pub fn duration(&self, media: &mut MediaIndex) -> Time {
        let mut total = 0i64;
        for c in &self.tracks.video {
            if let Ok(m) = media.get(Path::new(&c.source)) {
                total += c.out_duration(m.duration).micros();
            }
        }
        Time::from_micros(total)
    }

    pub fn next_id(&self, prefix: &str) -> String {
        let mut n = 1;
        loop {
            let candidate = format!("{prefix}_{n}");
            let taken = self.tracks.video.iter().any(|c| c.id == candidate)
                || self.tracks.overlays.iter().any(|o| match o {
                    Overlay::Text(t) => t.id == candidate,
                });
            if !taken {
                return candidate;
            }
            n += 1;
        }
    }

    /// Hard errors only. Things that need media facts are `diagnose`.
    pub fn validate(&self) -> Result<()> {
        if self.version != VERSION {
            bail!("unsupported project version {} (expected {})", self.version, VERSION);
        }
        if self.canvas.width == 0 || self.canvas.height == 0 {
            bail!("canvas has zero size");
        }
        for c in &self.tracks.video {
            if c.speed <= 0.0 {
                bail!("clip {} has speed <= 0", c.id);
            }
            if let Some(out) = c.src_out {
                if out < c.src_in {
                    bail!("clip {} ends before it starts", c.id);
                }
            }
        }
        Ok(())
    }

    /// Warnings that need the sources on disk. Never blocks a render.
    pub fn diagnose(&self, media: &mut MediaIndex) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        let mut placed = Time::ZERO;
        for (i, c) in self.tracks.video.iter().enumerate() {
            let path = Path::new(&c.source);
            match media.get(path) {
                Ok(m) => {
                    if let Some(end) = c.src_out {
                        if end > m.duration {
                            out.push(Diagnostic::new(
                                "clip_past_media",
                                format!("tracks.video[{i}]"),
                                format!(
                                    "{} is {} long but clip {} ends at {}",
                                    c.source, m.duration, c.id, end
                                ),
                            ));
                        }
                    }
                    if !m.has_video {
                        out.push(Diagnostic::new(
                            "clip_has_no_video",
                            format!("tracks.video[{i}]"),
                            format!("{} has no video stream", c.source),
                        ));
                    }
                }
                Err(e) => out.push(Diagnostic::new(
                    "media_missing",
                    format!("tracks.video[{i}].source"),
                    format!("{e}"),
                )),
            }
            let span = media
                .get(path)
                .map(|m| c.out_duration(m.duration))
                .unwrap_or(Time::ZERO);
            placed = Time::from_micros(placed.micros() + span.micros());
        }
        if let Some(caps) = &self.tracks.captions {
            if !Path::new(&caps.source).exists() {
                out.push(Diagnostic::new(
                    "captions_missing",
                    "tracks.captions.source",
                    format!("{} does not exist", caps.source),
                ));
            }
        }
        for (i, o) in self.tracks.overlays.iter().enumerate() {
            let Overlay::Text(t) = o;
            if t.at.end > placed && placed > Time::ZERO {
                out.push(Diagnostic::new(
                    "text_past_end",
                    format!("tracks.overlays[{i}].at"),
                    format!(
                        "text {} ends at {} but the timeline ends at {}",
                        t.id, t.at.end, placed
                    ),
                ));
            }
        }
        out
    }
}

impl Clip {
    /// How much of the source file this clip consumes. Clamped to what the
    /// media actually has, so a reported duration matches the rendered one.
    pub fn source_span(&self, media_duration: Time) -> Time {
        self.src_out
            .map(|o| o.min(media_duration))
            .unwrap_or(media_duration)
            .saturating_sub(self.src_in)
    }

    /// Duration this clip contributes to the timeline, after speed.
    pub fn out_duration(&self, media_duration: Time) -> Time {
        Time::from_secs_f64(self.source_span(media_duration).secs_f64() / self.speed)
    }
}

fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(p)
    }
}

/// Owns `sona.json`. Every command opens, mutates, saves through this.
pub struct Store {
    pub path: PathBuf,
    pub project: Project,
}

impl Store {
    pub fn create(path: &Path, width: u32, height: u32, fps: Rational) -> Result<Store> {
        if path.exists() {
            bail!("{} already exists", path.display());
        }
        let store = Store {
            path: absolute(path),
            project: Project::new(width, height, fps),
        };
        store.save()?;
        fs::create_dir_all(store.cache_dir())?;
        Ok(store)
    }

    pub fn open(path: &Path) -> Result<Store> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("no project at {} (run `sona init`)", path.display()))?;
        let project: Project = serde_json::from_str(&raw)
            .with_context(|| format!("{} is not a valid project", path.display()))?;
        project.validate()?;
        Ok(Store {
            path: absolute(path),
            project,
        })
    }

    pub fn dir(&self) -> PathBuf {
        self.path.parent().unwrap_or(Path::new(".")).to_path_buf()
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.dir().join(".sona").join("cache")
    }

    pub fn resolve(&self, rel: &str) -> PathBuf {
        let p = Path::new(rel);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.dir().join(p)
        }
    }

    /// Atomic replace: write beside, then rename. On Windows the rename
    /// replaces because we never hold the destination open.
    pub fn save(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.project)?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json + "\n")?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}
