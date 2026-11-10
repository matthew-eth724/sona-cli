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
    /// Which word index this project is reading. The index itself lives in
    /// `.sona/index/<audio_hash>.json`; this is only the pointer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<IndexRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IndexRef {
    pub audio_hash: String,
    pub source: String,
    pub transcript: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub words: usize,
    #[serde(default, skip_serializing_if = "is_false")]
    pub approximate: bool,
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
    /// How this clip is brought in from the previous one. `None` is a hard cut.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<Transition>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transition {
    pub kind: TransitionKind,
    pub duration: Time,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    Crossfade,
    /// Fade out to black, then in: `fadeblack` in ffmpeg.
    DipToBlack,
}

impl TransitionKind {
    pub fn xfade(self) -> &'static str {
        match self {
            TransitionKind::Crossfade => "fade",
            TransitionKind::DipToBlack => "fadeblack",
        }
    }

    pub fn parse(raw: &str) -> Option<TransitionKind> {
        match raw {
            "crossfade" | "fade" => Some(TransitionKind::Crossfade),
            "dip_to_black" | "dip-to-black" | "dip" | "fadeblack" => {
                Some(TransitionKind::DipToBlack)
            }
            _ => None,
        }
    }
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
    Image(ImageOverlay),
}

impl Overlay {
    pub fn id(&self) -> &str {
        match self {
            Overlay::Text(t) => &t.id,
            Overlay::Image(i) => &i.id,
        }
    }

    pub fn at(&self) -> TimeRange {
        match self {
            Overlay::Text(t) => t.at,
            Overlay::Image(i) => i.at,
        }
    }

    pub fn is_image(&self) -> bool {
        matches!(self, Overlay::Image(_))
    }
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

/// Also used to place image overlays, where the corners are what a logo or a
/// lower third actually wants.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    Top,
    TopLeft,
    TopRight,
    #[default]
    Bottom,
    BottomLeft,
    BottomRight,
    Center,
}

/// A PNG over the picture: logo, lower third, sting.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageOverlay {
    pub id: String,
    pub source: String,
    pub at: TimeRange,
    #[serde(default)]
    pub anchor: Anchor,
    /// Width as a fraction of the canvas width, so the same project renders
    /// correctly at preview size and at full size.
    pub width: f32,
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    /// Distance from the canvas edge, also as a fraction of the width.
    #[serde(default = "default_margin")]
    pub margin: f32,
}

fn default_opacity() -> f32 {
    1.0
}
fn default_margin() -> f32 {
    0.03
}

impl ImageOverlay {
    /// Overlay width in pixels for a given canvas, always even.
    pub fn px_width(&self, canvas_width: u32) -> u32 {
        let px = (canvas_width as f32 * self.width).round().max(2.0) as u32;
        (px / 2) * 2
    }

    pub fn px_margin(&self, canvas_width: u32) -> u32 {
        (canvas_width as f32 * self.margin).round().max(0.0) as u32
    }

    /// ffmpeg overlay `x:y` for this anchor. `W`/`H` are the canvas, `w`/`h`
    /// the overlay's own size, `m` the margin.
    pub fn xy_expr(&self) -> (&'static str, &'static str) {
        match self.anchor {
            Anchor::Top => ("(W-w)/2", "m"),
            Anchor::TopLeft => ("m", "m"),
            Anchor::TopRight => ("W-w-m", "m"),
            Anchor::Center => ("(W-w)/2", "(H-h)/2"),
            Anchor::Bottom => ("(W-w)/2", "H-h-m"),
            Anchor::BottomLeft => ("m", "H-h-m"),
            Anchor::BottomRight => ("W-w-m", "H-h-m"),
        }
    }
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
        for (i, c) in self.tracks.video.iter().enumerate() {
            let d = match media.get(Path::new(&c.source)) {
                Ok(m) => c.out_duration(m.duration),
                Err(_) => c.out_duration_known().unwrap_or(Time::ZERO),
            };
            total += d.micros();
            // A transition overlaps its two neighbours, so it shortens the
            // program by its own duration. The first clip cannot have one.
            if i > 0 {
                if let Some(t) = &c.transition {
                    total -= t.duration.micros();
                }
            }
        }
        Time::from_micros(total.max(0))
    }

    pub fn next_id(&self, prefix: &str) -> String {
        let mut n = 1;
        loop {
            let candidate = format!("{prefix}_{n}");
            let taken = self.tracks.video.iter().any(|c| c.id == candidate)
                || self.tracks.overlays.iter().any(|o| o.id() == candidate);
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
        for (i, c) in self.tracks.video.iter().enumerate() {
            if c.speed <= 0.0 {
                bail!("clip {} has speed <= 0", c.id);
            }
            if i == 0 && c.transition.is_some() {
                bail!("clip {} is first; it has nothing to transition from", c.id);
            }
            if let Some(t) = &c.transition {
                if t.duration.micros() <= 0 {
                    bail!("clip {} has a zero-length transition", c.id);
                }
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
                            out.push(
                                Diagnostic::new(
                                    "clip_past_media",
                                    format!("tracks.video[{i}]"),
                                    format!(
                                        "{} is {} long but clip {} ends at {}",
                                        c.source, m.duration, c.id, end
                                    ),
                                )
                                .with_fix(format!("sona trim {} --to {}", c.id, m.duration)),
                            );
                        }
                    }
                    if !m.has_video {
                        out.push(
                            Diagnostic::new(
                                "clip_has_no_video",
                                format!("tracks.video[{i}]"),
                                format!("{} has no video stream", c.source),
                            )
                            .with_fix(format!("sona rm {}", c.id)),
                        );
                    }
                }
                Err(e) => out.push(
                    Diagnostic::new(
                        "media_missing",
                        format!("tracks.video[{i}].source"),
                        format!("{e}"),
                    )
                    .with_fix(format!("sona rm {}   (or put the file back)", c.id)),
                ),
            }
            let span = match media.get(path) {
                Ok(m) => c.out_duration(m.duration),
                Err(_) => c.out_duration_known().unwrap_or(Time::ZERO),
            };
            placed = Time::from_micros(placed.micros() + span.micros());
        }
        let mut prev_len = Time::ZERO;
        for (i, c) in self.tracks.video.iter().enumerate() {
            let len = match media.get(Path::new(&c.source)) {
                Ok(m) => c.out_duration(m.duration),
                Err(_) => c.out_duration_known().unwrap_or(Time::ZERO),
            };
            if let Some(t) = &c.transition {
                let shorter = len.min(prev_len);
                if t.duration > shorter {
                    out.push(
                        Diagnostic::new(
                            "transition_too_long",
                            format!("tracks.video[{i}].transition"),
                            format!(
                                "{} fades for {} but the shorter neighbour is {}",
                                c.id, t.duration, shorter
                            ),
                        )
                        .with_fix(format!(
                            "sona transition {} --duration {}",
                            c.id,
                            Time::from_micros((shorter.micros() / 2).max(1))
                        )),
                    );
                }
            }
            prev_len = len;
        }
        if let Some(caps) = &self.tracks.captions {
            if !Path::new(&caps.source).exists() {
                out.push(
                    Diagnostic::new(
                        "captions_missing",
                        "tracks.captions.source",
                        format!("{} does not exist", caps.source),
                    )
                    .with_fix("sona rm captions   (or put the file back)"),
                );
            }
        }
        for (i, o) in self.tracks.overlays.iter().enumerate() {
            let at = o.at();
            if at.end > placed && placed > Time::ZERO {
                let what = if o.is_image() { "image" } else { "text" };
                let code = if o.is_image() { "image_past_end" } else { "text_past_end" };
                out.push(
                    Diagnostic::new(
                        code,
                        format!("tracks.overlays[{i}].at"),
                        format!(
                            "{} {} ends at {} but the timeline ends at {}",
                            what,
                            o.id(),
                            at.end,
                            placed
                        ),
                    )
                    .with_fix(format!("sona rm {}", o.id())),
                );
            }
        }
        for (i, o) in self.tracks.overlays.iter().enumerate() {
            if let Overlay::Image(img) = o {
                if !Path::new(&img.source).exists() {
                    out.push(
                        Diagnostic::new(
                            "overlay_image_missing",
                            format!("tracks.overlays[{i}].source"),
                            format!("{} does not exist", img.source),
                        )
                        .with_fix(format!("sona rm {}", img.id)),
                    );
                }
                if img.width <= 0.0 || img.width > 1.0 {
                    out.push(
                        Diagnostic::new(
                            "overlay_bad_width",
                            format!("tracks.overlays[{i}].width"),
                            format!("width {} is not between 0 and 1 of the canvas", img.width),
                        )
                        .with_fix(format!("sona image {} --width 320", img.source)),
                    );
                }
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

    /// The same answer without the media, when `src_out` is known. Keeps
    /// duration and diagnostics honest for a project whose files moved.
    pub fn out_duration_known(&self) -> Option<Time> {
        self.src_out
            .map(|e| Time::from_secs_f64(e.saturating_sub(self.src_in).secs_f64() / self.speed))
    }
}

fn is_false(b: &bool) -> bool {
    !*b
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::MediaIndex;

    #[test]
    fn missing_media_names_the_command_that_fixes_it() {
        let mut p = Project::new(640, 360, Rational::new(30, 1));
        p.tracks.video.push(Clip {
            id: "clip_1".into(),
            source: "/nope/gone.mp4".into(),
            src_in: Time::ZERO,
            src_out: None,
            speed: 1.0,
            transform: Transform::Fit,
            gain_db: 0.0,
            transition: None,
        });
        let mut media = MediaIndex::default();
        let diags = p.diagnose(&mut media);
        let d = diags.iter().find(|d| d.code == "media_missing").expect("a warning");
        assert_eq!(
            d.fix.as_deref(),
            Some("sona rm clip_1   (or put the file back)")
        );
    }

    #[test]
    fn text_past_the_end_points_at_its_own_id() {
        let mut p = Project::new(640, 360, Rational::new(30, 1));
        // A 9s clip, with src_out known, so the timeline has a length even
        // though the file does not exist.
        p.tracks.video.push(Clip {
            id: "clip_1".into(),
            source: "/nope/gone.mp4".into(),
            src_in: Time::ZERO,
            src_out: Some(Time::from_secs_f64(9.0)),
            speed: 1.0,
            transform: Transform::Fit,
            gain_db: 0.0,
            transition: None,
        });
        p.tracks.overlays.push(Overlay::Text(TextOverlay {
            id: "text_1".into(),
            content: "late".into(),
            at: TimeRange::new(Time::from_secs_f64(0.0), Time::from_secs_f64(10.0)).unwrap(),
            anchor: Anchor::Bottom,
            size: 48,
            color: "white".into(),
        }));
        let mut media = MediaIndex::default();
        let diags = p.diagnose(&mut media);
        let d = diags.iter().find(|d| d.code == "text_past_end").expect("a warning");
        assert_eq!(d.fix.as_deref(), Some("sona rm text_1"));
    }

    fn clip_with(id: &str, out: f64, transition: Option<Transition>) -> Clip {
        Clip {
            id: id.into(),
            source: "/nope/gone.mp4".into(),
            src_in: Time::ZERO,
            src_out: Some(Time::from_secs_f64(out)),
            speed: 1.0,
            transform: Transform::Fit,
            gain_db: 0.0,
            transition,
        }
    }

    #[test]
    fn transitions_shorten_the_program() {
        let mut p = Project::new(640, 360, Rational::new(30, 1));
        p.tracks.video = vec![
            clip_with("clip_1", 3.0, None),
            clip_with(
                "clip_2",
                3.0,
                Some(Transition {
                    kind: TransitionKind::Crossfade,
                    duration: Time::from_secs_f64(0.6),
                }),
            ),
        ];
        let mut media = MediaIndex::default();
        // 3 + 3 - 0.6
        assert_eq!(p.duration(&mut media).micros(), 5_400_000);
    }

    #[test]
    fn a_transition_on_the_first_clip_is_rejected() {
        let mut p = Project::new(640, 360, Rational::new(30, 1));
        p.tracks.video = vec![clip_with(
            "clip_1",
            3.0,
            Some(Transition {
                kind: TransitionKind::Crossfade,
                duration: Time::from_secs_f64(0.5),
            }),
        )];
        let err = p.validate().unwrap_err().to_string();
        assert!(err.contains("nothing to transition from"), "{err}");
    }

    #[test]
    fn a_transition_longer_than_its_neighbour_warns_with_a_fix() {
        let mut p = Project::new(640, 360, Rational::new(30, 1));
        p.tracks.video = vec![
            clip_with("clip_1", 1.0, None),
            clip_with(
                "clip_2",
                5.0,
                Some(Transition {
                    kind: TransitionKind::Crossfade,
                    duration: Time::from_secs_f64(2.0),
                }),
            ),
        ];
        let mut media = MediaIndex::default();
        let d = p
            .diagnose(&mut media)
            .into_iter()
            .find(|d| d.code == "transition_too_long")
            .expect("a warning");
        assert_eq!(d.fix.as_deref(), Some("sona transition clip_2 --duration 00:00:00.500"));
    }

    #[test]
    fn every_warning_carries_a_fix() {
        // A diagnostic without a fix is a dead end for an agent.
        let warnings = [
            Diagnostic::new("clip_past_media", "x", "m"),
            Diagnostic::new("clip_has_no_video", "x", "m"),
            Diagnostic::new("media_missing", "x", "m"),
            Diagnostic::new("captions_missing", "x", "m"),
            Diagnostic::new("text_past_end", "x", "m"),
        ];
        for w in warnings {
            let _ = w;
        }
        let mut p = Project::new(640, 360, Rational::new(30, 1));
        p.tracks.video.push(Clip {
            id: "clip_1".into(),
            source: "/nope/gone.mp4".into(),
            src_in: Time::ZERO,
            src_out: Some(Time::from_secs_f64(9.0)),
            speed: 1.0,
            transform: Transform::Fit,
            gain_db: 0.0,
            transition: None,
        });
        p.tracks.captions = Some(Captions { source: "/nope/x.srt".into(), burn: true });
        let mut media = MediaIndex::default();
        for d in p.diagnose(&mut media) {
            assert!(d.fix.is_some(), "{} has no fix", d.code);
        }
    }
}
