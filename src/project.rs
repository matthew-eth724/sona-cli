//! The project document: the whole state, one writer.
use crate::media::MediaIndex;
use crate::report::Diagnostic;
use crate::time::{Rational, Time, TimeRange};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const VERSION: u32 = 2;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub version: u32,
    pub canvas: Canvas,
    pub tracks: Tracks,
    /// A look applied to the whole program. Lives above the clips so changing it
    /// re-encodes one step, not every clip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grade: Option<Grade>,
}

/// Colour adjustment. Every field is optional so a project can set one knob.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Grade {
    /// -1.0 dark to 1.0 bright.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness: Option<f32>,
    /// 1.0 unchanged, 0 flat grey, 2 high contrast.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contrast: Option<f32>,
    /// 0 grey, 1 unchanged, 2 vivid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saturation: Option<f32>,
    /// 1.0 unchanged, above 1 lifts midtones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gamma: Option<f32>,
    /// A `.cube` 3D LUT. Path, resolved against the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lut: Option<String>,
}

impl Grade {
    pub fn is_noop(&self) -> bool {
        self.brightness.is_none()
            && self.contrast.is_none()
            && self.saturation.is_none()
            && self.gamma.is_none()
            && self.lut.is_none()
    }

    /// The `eq` filter, only if at least one knob is set.
    pub fn eq(&self) -> Option<String> {
        if self.brightness.is_none()
            && self.contrast.is_none()
            && self.saturation.is_none()
            && self.gamma.is_none()
        {
            return None;
        }
        let mut parts = Vec::new();
        if let Some(v) = self.brightness {
            parts.push(format!("brightness={v:.4}"));
        }
        if let Some(v) = self.contrast {
            parts.push(format!("contrast={v:.4}"));
        }
        if let Some(v) = self.saturation {
            parts.push(format!("saturation={v:.4}"));
        }
        if let Some(v) = self.gamma {
            parts.push(format!("gamma={v:.4}"));
        }
        Some(format!("eq={}", parts.join(":")))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub fps: Rational,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Tracks {
    /// Unordered; `z` is the stacking order. Version 1 had a flat `video` vec,
    /// which `upgrade` turns into a single layer `main`.
    #[serde(default)]
    pub layers: Vec<Layer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioBed>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overlays: Vec<Overlay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captions: Option<Captions>,
    /// Which word index this project is reading. The index itself lives in
    /// `.splice/index/<audio_hash>.json`; this is only the pointer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<IndexRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Layer {
    pub id: String,
    /// Stacking order: higher is on top. Unique, so there is one total order and
    /// no tie-breaking rule to remember.
    pub z: i32,
    /// Where this layer's t=0 sits on the program clock.
    #[serde(default)]
    pub start: Time,
    #[serde(default)]
    pub place: Place,
    /// The camera inside this layer. Absent means "no move".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<Motion>,
    /// Where the layer sits over time. Position only: `size` is one value, so a
    /// layer can slide but not grow — `scale` has no time variable, and faking it
    /// with a second mechanism is the two-representations trap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place_keys: Option<Vec<PlaceKey>>,
    /// Knock part of the layer out, or darken its edges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<Mask>,
    /// Sequential inside a layer: version 1's model, unchanged.
    pub clips: Vec<Clip>,
}

/// Where a layer's frame lands on the canvas, in canvas fractions.
/// `Place::CANVAS` is the default and the only legal place at z = 0.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub x: f32,
    pub y: f32,
    pub size: f32,
}

impl Default for Place {
    fn default() -> Place {
        Place::CANVAS
    }
}

impl Place {
    pub const CANVAS: Place = Place {
        x: 0.0,
        y: 0.0,
        size: 1.0,
    };

    pub fn is_canvas(&self) -> bool {
        (self.size - 1.0).abs() < f32::EPSILON
            && self.x.abs() < f32::EPSILON
            && self.y.abs() < f32::EPSILON
    }

    /// Top-left and side in pixels, even-rounded for h264.
    pub fn px(&self, canvas: (u32, u32)) -> (u32, u32, u32) {
        let side = ((canvas.0 as f32 * self.size).round() as u32 / 2 * 2).max(2);
        let x = (canvas.0 as f32 * self.x).round().max(0.0) as u32;
        let y = (canvas.1 as f32 * self.y).round().max(0.0) as u32;
        (x, y, side)
    }
}

/// What a layer keeps visible.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mask {
    /// Remove a colour, leaving alpha: the green-screen case.
    Chroma {
        /// Any ffmpeg colour: `0x00FF00`, `green`, `#0f0`.
        color: String,
        #[serde(default = "chroma_similarity")]
        similarity: f32,
        #[serde(default)]
        blend: f32,
    },
    /// Darken the edges.
    Vignette {
        #[serde(default = "vignette_angle")]
        angle: f32,
    },
}

fn chroma_similarity() -> f32 {
    0.30
}

fn vignette_angle() -> f32 {
    0.628
}

impl Mask {
    /// The filter that applies it, if any.
    pub fn chain(&self) -> String {
        match self {
            Mask::Chroma { color, similarity, blend } => format!(
                "colorkey={}:{similarity:.4}:{blend:.4}",
                color
            ),
            Mask::Vignette { angle } => format!("vignette=angle={angle:.4}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlaceKey {
    /// Layer-local time: 0 is the layer's own start.
    pub t: Time,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// A camera move in layer-local time, linear between keys and held flat outside
/// them. `zoom` is >= 1.0 (1.0 = the whole layer visible); `at` is the centre of
/// the view in canvas fractions.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Motion {
    pub keys: Vec<Key>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Key {
    pub t: Time,
    pub zoom: f64,
    pub at: Point,
}

impl Motion {
    /// Piecewise-linear, held at the ends. Pure: same keys, same numbers.
    pub fn sample(&self, t: Time) -> (f64, Point) {
        let first = self.keys.first().expect("motion has keys");
        let last = self.keys.last().expect("motion has keys");
        if t <= first.t {
            return (first.zoom, first.at);
        }
        if t >= last.t {
            return (last.zoom, last.at);
        }
        for w in self.keys.windows(2) {
            let (a, b) = (w[0], w[1]);
            if t >= a.t && t <= b.t {
                let span = (b.t.micros() - a.t.micros()) as f64;
                let f = if span <= 0.0 {
                    0.0
                } else {
                    (t.micros() - a.t.micros()) as f64 / span
                };
                return (
                    a.zoom + (b.zoom - a.zoom) * f,
                    Point {
                        x: a.at.x + (b.at.x - a.at.x) * f as f32,
                        y: a.at.y + (b.at.y - a.at.y) * f as f32,
                    },
                );
            }
        }
        (last.zoom, last.at)
    }

    pub fn max_zoom(&self) -> f64 {
        self.keys.iter().map(|k| k.zoom).fold(1.0, f64::max)
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
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
    /// Speed over time, as offsets into the source. A ramp is compiled by slicing
    /// the source at these points and giving each slice one constant speed, so
    /// the audio keeps its pitch and stays in sync. When present, `speed` is
    /// ignored — one representation at a time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_keys: Option<Vec<SpeedKey>>,
    /// How this clip is brought in from the previous one. `None` is a hard cut.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<Transition>,
}

/// Speed at a point in the source. Offsets are absolute source time, so a ramp
/// survives a trim that moves `src_in`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpeedKey {
    pub t: Time,
    pub speed: f64,
}

/// One slice of a clip: a source range and the constant speed it plays at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slice {
    pub start: Time,
    pub end: Time,
    pub speed: f64,
}

impl Slice {
    pub fn span(&self) -> Time {
        self.end.saturating_sub(self.start)
    }

    /// How long this slice occupies on the timeline.
    pub fn out(&self) -> Time {
        Time::from_secs_f64(self.span().secs_f64() / self.speed)
    }
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
    /// Lower the bed while there is speech, using the word index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duck: Option<Duck>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Duck {
    /// How far to duck. Negative.
    pub db: f32,
    /// Fade in and out over this long, so the gain does not step.
    #[serde(default = "default_ramp")]
    pub ramp: Time,
    /// Speech spans already mapped to timeline time. Resolved when `splice duck`
    /// runs, so a render never needs the index.
    #[serde(default)]
    pub spans: Vec<TimeRange>,
    /// Fingerprint of the cut these spans were mapped against. If the timeline
    /// moves, the spans are stale and `diagnose` says so.
    #[serde(default)]
    pub layout: String,
}

impl Project {
    /// Identity of the cut: clip bounds, speed and transitions. Duck spans are
    /// only valid for the layout they were computed against.
    pub fn layout_token(&self) -> String {
        let mut parts = Vec::new();
        for layer in self.by_z() {
            // Layer placement moves what is audible, so it belongs to the layout.
            parts.push(format!(
                "L:{}:{}:{:?}:{:?}",
                layer.id, layer.z, layer.start.micros(), layer.place
            ));
            for c in &layer.clips {
                parts.push(format!(
                    "{}:{}:{:?}:{}:{:?}",
                    c.id,
                    c.src_in.micros(),
                    c.src_out.map(|t| t.micros()),
                    c.speed,
                    c.transition.as_ref().map(|t| (t.kind.xfade(), t.duration.micros()))
                ));
            }
        }
        parts.join("|")
    }
}

fn default_ramp() -> Time {
    Time::from_micros(140_000)
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
            grade: None,
            tracks: Tracks {
                layers: vec![Layer {
                    id: "main".into(),
                    z: 0,
                    start: Time::ZERO,
                    place: Place::CANVAS,
                    motion: None,
                    place_keys: None,
                    mask: None,
                    clips: Vec::new(),
                }],
                ..Tracks::default()
            },
        }
    }

    /// Program duration is the longest layer: layers play at the same time, so
    /// this is a max, not a sum. One layer reduces to the version-1 formula
    /// exactly: sum of clips minus the transitions that overlap them.
    pub fn duration(&self, media: &mut MediaIndex) -> Time {
        let mut longest = Time::ZERO;
        for layer in &self.tracks.layers {
            let mut total = layer.start.micros();
            for (i, c) in layer.clips.iter().enumerate() {
                let d = match media.get(Path::new(&c.source)) {
                    Ok(m) => c.out_duration(m.duration),
                    Err(_) => c.out_duration_known().unwrap_or(Time::ZERO),
                };
                total += d.micros();
                if i > 0 {
                    if let Some(t) = &c.transition {
                        total -= t.duration.micros();
                    }
                }
            }
            longest = longest.max(Time::from_micros(total.max(0)));
        }
        longest
    }

    /// How long one layer runs, after trims, speed and transitions.
    pub fn duration_of_layer(&self, media: &mut MediaIndex, layer: &Layer) -> Time {
        let mut total = 0i64;
        for (i, c) in layer.clips.iter().enumerate() {
            let d = match media.get(Path::new(&c.source)) {
                Ok(m) => c.out_duration(m.duration),
                Err(_) => c.out_duration_known().unwrap_or(Time::ZERO),
            };
            total += d.micros();
            if i > 0 {
                if let Some(t) = &c.transition {
                    total -= t.duration.micros();
                }
            }
        }
        Time::from_micros(total.max(0))
    }

    /// The one sorted view of the layers. Nothing else may sort them.
    pub fn by_z(&self) -> Vec<&Layer> {
        let mut v: Vec<&Layer> = self.tracks.layers.iter().collect();
        v.sort_by_key(|l| l.z);
        v
    }

    pub fn layer(&self, id: &str) -> Option<&Layer> {
        self.tracks.layers.iter().find(|l| l.id == id)
    }

    pub fn layer_mut(&mut self, id: &str) -> Option<&mut Layer> {
        self.tracks.layers.iter_mut().find(|l| l.id == id)
    }

    /// The layer that carries program audio and the default edit target.
    pub fn main_layer_mut(&mut self) -> Option<&mut Layer> {
        self.tracks.layers.iter_mut().find(|l| l.z == 0)
    }

    pub fn main_layer(&self) -> Option<&Layer> {
        self.tracks.layers.iter().find(|l| l.z == 0)
    }

    /// Clip ids are unique across the document, so an id is a full handle.
    pub fn clip(&self, id: &str) -> Option<&Clip> {
        self.tracks
            .layers
            .iter()
            .flat_map(|l| l.clips.iter())
            .find(|c| c.id == id)
    }

    pub fn clip_layer_index(&self, id: &str) -> Option<(usize, usize)> {
        for (li, l) in self.tracks.layers.iter().enumerate() {
            if let Some(ci) = l.clips.iter().position(|c| c.id == id) {
                return Some((li, ci));
            }
        }
        None
    }

    pub fn clips(&self) -> impl Iterator<Item = &Clip> {
        self.tracks.layers.iter().flat_map(|l| l.clips.iter())
    }

    pub fn next_id(&self, prefix: &str) -> String {
        let mut n = 1;
        loop {
            let candidate = format!("{prefix}_{n}");
            let taken = self.clips().any(|c| c.id == candidate)
                || self.tracks.overlays.iter().any(|o| o.id() == candidate)
                || self.tracks.layers.iter().any(|l| l.id == candidate);
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
        if self.tracks.layers.is_empty() {
            bail!("project has no layers");
        }
        if let Some(g) = &self.grade {
            if let Some(v) = g.brightness {
                if !(-1.0..=1.0).contains(&v) {
                    bail!("grade brightness must be between -1 and 1");
                }
            }
            if let Some(v) = g.contrast {
                if !(0.0..=4.0).contains(&v) {
                    bail!("grade contrast must be between 0 and 4");
                }
            }
            if let Some(v) = g.saturation {
                if !(0.0..=4.0).contains(&v) {
                    bail!("grade saturation must be between 0 and 4");
                }
            }
            if let Some(v) = g.gamma {
                if !(0.05..=10.0).contains(&v) {
                    bail!("grade gamma must be between 0.05 and 10");
                }
            }
        }
        let mut zs: Vec<i32> = self.tracks.layers.iter().map(|l| l.z).collect();
        zs.sort();
        for pair in zs.windows(2) {
            if pair[0] == pair[1] {
                bail!("two layers share z = {}; z must be unique", pair[0]);
            }
        }
        match self.main_layer() {
            None => bail!("no layer at z = 0; that layer carries the program audio"),
            Some(main) => {
                if !main.place.is_canvas() {
                    bail!(
                        "layer {} is at z = 0 and must fill the canvas (place is {},{},{} wide)",
                        main.id,
                        main.place.x,
                        main.place.y,
                        main.place.size
                    );
                }
            }
        }
        for l in &self.tracks.layers {
            if l.start.micros() < 0 {
                bail!("layer {} starts before zero", l.id);
            }
            // An empty layer is legal to store: `init` creates one, and `add`
            // needs to open the project before it can put a clip in it. Planning
            // is where "no clips" becomes an error.
            if let Some(Mask::Chroma { similarity, blend, .. }) = &l.mask {
                if !(0.0..=1.0).contains(similarity) || !(0.0..=1.0).contains(blend) {
                    bail!("layer {} has a chroma mask outside 0..1", l.id);
                }
            }
            if let Some(Mask::Vignette { angle }) = &l.mask {
                if !(0.0..=3.2).contains(angle) {
                    bail!("layer {} has a vignette angle outside 0..3.2 radians", l.id);
                }
            }
            if let Some(keys) = &l.place_keys {
                if keys.is_empty() {
                    bail!("layer {} has placement keys but none of them", l.id);
                }
                for w in keys.windows(2) {
                    if w[1].t <= w[0].t {
                        bail!("layer {} has placement keys out of order", l.id);
                    }
                }
                for k in keys {
                    if !(0.0..=1.0).contains(&k.x) || !(0.0..=1.0).contains(&k.y) {
                        bail!("layer {} has a placement outside the frame", l.id);
                    }
                }
            }
            if let Some(m) = &l.motion {
                if m.keys.is_empty() {
                    bail!("layer {} has a camera with no keys", l.id);
                }
                for w in m.keys.windows(2) {
                    if w[1].t <= w[0].t {
                        bail!("layer {} has camera keys out of order", l.id);
                    }
                }
                for k in &m.keys {
                    if !(k.zoom >= 1.0) {
                        bail!("layer {} has a key with zoom < 1.0", l.id);
                    }
                    if !(0.0..=1.0).contains(&k.at.x) || !(0.0..=1.0).contains(&k.at.y) {
                        bail!("layer {} has a key with a centre outside the frame", l.id);
                    }
                }
            }
            for (i, c) in l.clips.iter().enumerate() {
                if c.speed <= 0.0 {
                    bail!("clip {} has speed <= 0", c.id);
                }
                if i == 0 && c.transition.is_some() {
                    bail!("clip {} is first in its layer; nothing to transition from", c.id);
                }
                if let Some(t) = &c.transition {
                    if t.duration.micros() <= 0 {
                        bail!("clip {} has a zero-length transition", c.id);
                    }
                }
                if let Some(keys) = &c.speed_keys {
                    if keys.is_empty() {
                        bail!("clip {} has a ramp with no keys", c.id);
                    }
                    for w in keys.windows(2) {
                        if w[1].t <= w[0].t {
                            bail!("clip {} has ramp keys out of order", c.id);
                        }
                    }
                    for k in keys {
                        if k.speed <= 0.0 {
                            bail!("clip {} has a ramp speed of {} or less", c.id, k.speed);
                        }
                    }
                }
                if let Some(out) = c.src_out {
                    if out < c.src_in {
                        bail!("clip {} ends before it starts", c.id);
                    }
                }
            }
        }
        Ok(())
    }

    /// Warnings that need the sources on disk. Never blocks a render.
    pub fn diagnose(&self, media: &mut MediaIndex) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        for layer in &self.tracks.layers {
            for (i, c) in layer.clips.iter().enumerate() {
            let at = format!("tracks.layers[{}].clips[{i}]", layer.id);
            let path = Path::new(&c.source);
            match media.get(path) {
                Ok(m) => {
                    if let Some(end) = c.src_out {
                        if end > m.duration {
                            out.push(
                                Diagnostic::new(
                                    "clip_past_media",
                                    at.clone(),
                                    format!(
                                        "{} is {} long but clip {} ends at {}",
                                        c.source, m.duration, c.id, end
                                    ),
                                )
                                .with_fix(format!("splice trim {} --to {}", c.id, m.duration)),
                            );
                        }
                    }
                    if !m.has_video {
                        out.push(
                            Diagnostic::new(
                                "clip_has_no_video",
                                at.clone(),
                                format!("{} has no video stream", c.source),
                            )
                            .with_fix(format!("splice rm {}", c.id)),
                        );
                    }
                }
                Err(e) => out.push(
                    Diagnostic::new(
                        "media_missing",
                        format!("{at}.source"),
                        format!("{e}"),
                    )
                    .with_fix(format!("splice rm {}   (or put the file back)", c.id)),
                ),
            }
            }
        }
        for layer in &self.tracks.layers {
            let mut prev_len = Time::ZERO;
            for (i, c) in layer.clips.iter().enumerate() {
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
                            format!("tracks.layers[{}].clips[{i}].transition", layer.id),
                            format!(
                                "{} fades for {} but the shorter neighbour is {}",
                                c.id, t.duration, shorter
                            ),
                        )
                        .with_fix(format!(
                            "splice transition {} --duration {}",
                            c.id,
                            Time::from_micros((shorter.micros() / 2).max(1))
                        )),
                    );
                }
            }
            prev_len = len;
            }
        }
        for l in &self.tracks.layers {
            if l.z == 0 {
                continue;
            }
            if l.place.x + l.place.size > 1.0001 {
                out.push(
                    Diagnostic::new(
                        "place_off_canvas",
                        format!("tracks.layers[{}].place", l.id),
                        format!("layer {} runs past the right edge", l.id),
                    )
                    .with_fix(format!(
                        "splice layer set {} --place 0.6,{},{}",
                        l.id, l.place.y, l.place.size
                    )),
                );
            }
            // Only the layer at z = 0 is heard. Say so, rather than dropping an
            // overlay layer's audio in silence.
            if l.clips
                .iter()
                .any(|c| media.get(Path::new(&c.source)).map(|m| m.has_audio).unwrap_or(false))
            {
                out.push(
                    Diagnostic::new(
                        "layer_audio_dropped",
                        format!("tracks.layers[{}]", l.id),
                        format!("layer {} has audio, and only the layer at z = 0 is heard", l.id),
                    )
                    .with_fix("move those clips to the main layer, or accept the silence"),
                );
            }
        }
        let placed = self.duration(media);
        if let Some(bed) = &self.tracks.audio {
            if let Some(duck) = &bed.duck {
                if !duck.layout.is_empty() && duck.layout != self.layout_token() {
                    out.push(
                        Diagnostic::new(
                            "duck_stale",
                            "tracks.audio.duck",
                            "the timeline moved since the duck spans were mapped",
                        )
                        .with_fix("splice duck"),
                    );
                }
            }
        }
        if let Some(caps) = &self.tracks.captions {
            if !Path::new(&caps.source).exists() {
                out.push(
                    Diagnostic::new(
                        "captions_missing",
                        "tracks.captions.source",
                        format!("{} does not exist", caps.source),
                    )
                    .with_fix("splice rm captions   (or put the file back)"),
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
                    .with_fix(format!("splice rm {}", o.id())),
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
                        .with_fix(format!("splice rm {}", img.id)),
                    );
                }
                if img.width <= 0.0 || img.width > 1.0 {
                    out.push(
                        Diagnostic::new(
                            "overlay_bad_width",
                            format!("tracks.overlays[{i}].width"),
                            format!("width {} is not between 0 and 1 of the canvas", img.width),
                        )
                        .with_fix(format!("splice image {} --width 320", img.source)),
                    );
                }
            }
        }
        out
    }
}

impl Clip {
    /// The slices this clip compiles to.
    ///
    /// One slice for a constant speed. For a ramp, the source is cut at the key
    /// times and each slice plays at the average of the speeds at its ends —
    /// the piecewise-constant reading of a linear ramp. This is the single place
    /// that answers "where does it cut", so the planner, the duration maths and
    /// the doctor cannot disagree.
    pub fn slices(&self, media_duration: Time) -> Vec<Slice> {
        let end = self
            .src_out
            .unwrap_or(media_duration)
            .min(media_duration)
            .max(self.src_in);
        let keys = match &self.speed_keys {
            Some(k) if !k.is_empty() => k,
            _ => {
                return if end > self.src_in {
                    vec![Slice { start: self.src_in, end, speed: self.speed }]
                } else {
                    Vec::new()
                }
            }
        };
        let speed_at = |t: Time| -> f64 {
            if t <= keys[0].t {
                return keys[0].speed;
            }
            for w in keys.windows(2) {
                let (a, b) = (w[0], w[1]);
                if t >= a.t && t <= b.t {
                    let span = (b.t.micros() - a.t.micros()).max(1) as f64;
                    let f = (t.micros() - a.t.micros()) as f64 / span;
                    return a.speed + (b.speed - a.speed) * f;
                }
            }
            keys[keys.len() - 1].speed
        };

        // Boundaries: the clip's own ends, plus every key strictly inside.
        let mut bounds: Vec<Time> = vec![self.src_in];
        bounds.extend(
            keys.iter()
                .map(|k| k.t)
                .filter(|t| *t > self.src_in && *t < end),
        );
        bounds.push(end);
        bounds.sort_by_key(|t| t.micros());
        bounds.dedup();

        let mut out = Vec::new();
        for w in bounds.windows(2) {
            let (a, b) = (w[0], w[1]);
            if b <= a {
                continue;
            }
            let speed = ((speed_at(a) + speed_at(b)) / 2.0).max(0.01);
            out.push(Slice { start: a, end: b, speed });
        }
        out
    }

    /// How much of the source file this clip consumes. Clamped to what the
    /// media actually has, so a reported duration matches the rendered one.
    pub fn source_span(&self, media_duration: Time) -> Time {
        self.src_out
            .map(|o| o.min(media_duration))
            .unwrap_or(media_duration)
            .saturating_sub(self.src_in)
    }

    /// Duration this clip contributes to the timeline, after every speed change.
    pub fn out_duration(&self, media_duration: Time) -> Time {
        let total: i64 = self.slices(media_duration).iter().map(|s| s.out().micros()).sum();
        Time::from_micros(total)
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

/// Version 1 kept every clip in one flat list. Kept only to read old documents;
/// nothing writes it.
#[derive(Deserialize)]
struct LegacyV1 {
    #[allow(dead_code)]
    version: u32,
    canvas: Canvas,
    #[serde(default)]
    tracks: LegacyTracksV1,
}

#[derive(Default, Deserialize)]
struct LegacyTracksV1 {
    #[serde(default)]
    video: Vec<Clip>,
    #[serde(default)]
    audio: Option<AudioBed>,
    #[serde(default)]
    overlays: Vec<Overlay>,
    #[serde(default)]
    captions: Option<Captions>,
    #[serde(default)]
    index: Option<IndexRef>,
}

/// Version 1 had one video layer, so the upgrade is not a decision: it is that
/// layer, named `main`, at z = 0, filling the canvas, with no camera.
fn upgrade_v1(old: LegacyV1) -> Project {
    Project {
        version: VERSION,
        canvas: old.canvas,
        grade: None,
        tracks: Tracks {
            layers: vec![Layer {
                id: "main".into(),
                z: 0,
                start: Time::ZERO,
                place: Place::CANVAS,
                motion: None,
                place_keys: None,
                mask: None,
                clips: old.tracks.video,
            }],
            audio: old.tracks.audio,
            overlays: old.tracks.overlays,
            captions: old.tracks.captions,
            index: old.tracks.index,
        },
    }
}

/// Owns `splice.json`. Every command opens, mutates, saves through this.
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
            .with_context(|| format!("no project at {} (run `splice init`)", path.display()))?;
        let value: serde_json::Value = serde_json::from_str(&raw)
            .with_context(|| format!("{} is not valid JSON", path.display()))?;
        let version = value.get("version").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
        let project: Project = match version {
            1 => upgrade_v1(
                serde_json::from_value(value)
                    .with_context(|| format!("{} is not a valid version-1 project", path.display()))?,
            ),
            VERSION => serde_json::from_value(value)
                .with_context(|| format!("{} is not a valid project", path.display()))?,
            other => bail!(
                "unsupported project version {other} (this build reads 1 and {VERSION})"
            ),
        };
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
        self.dir().join(".splice").join("cache")
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
    fn a_clip_without_a_ramp_is_one_slice() {
        let c = clip_with("clip_1", 4.0, None);
        let slices = c.slices(Time::from_secs_f64(4.0));
        assert_eq!(slices.len(), 1);
        assert_eq!(slices[0].start.micros(), 0);
        assert_eq!(slices[0].end.micros(), 4_000_000);
        assert_eq!(c.out_duration(Time::from_secs_f64(4.0)).micros(), 4_000_000);
    }

    #[test]
    fn a_ramp_slices_at_its_keys_and_divides_by_the_average_speed() {
        let mut c = clip_with("clip_1", 8.0, None);
        // 1x -> 2x in four slices: each covers 2 s of source
        c.speed_keys = Some(
            (0..=4)
                .map(|i| SpeedKey {
                    t: Time::from_secs_f64(2.0 * i as f64),
                    speed: 1.0 + 0.25 * i as f64,
                })
                .collect(),
        );
        let slices = c.slices(Time::from_secs_f64(8.0));
        assert_eq!(slices.len(), 4);
        // the first slice plays at the average of 1.0 and 1.25
        assert!((slices[0].speed - 1.125).abs() < 1e-9);
        assert_eq!(slices[0].start.micros(), 0);
        assert_eq!(slices[3].end.micros(), 8_000_000);
        // 8 s of source at an average near 1.5x, and never longer than the source
        let out = c.out_duration(Time::from_secs_f64(8.0));
        assert!(out.micros() < 8_000_000);
        assert!((out.secs_f64() - 5.529).abs() < 0.01, "{}", out.secs_f64());
    }

    #[test]
    fn ramp_keys_outside_a_trim_are_ignored() {
        let mut c = clip_with("clip_1", 8.0, None);
        c.src_in = Time::from_secs_f64(3.0);
        c.src_out = Some(Time::from_secs_f64(5.0));
        c.speed_keys = Some(vec![
            SpeedKey { t: Time::ZERO, speed: 1.0 },
            SpeedKey { t: Time::from_secs_f64(8.0), speed: 2.0 },
        ]);
        let slices = c.slices(Time::from_secs_f64(8.0));
        // only the trimmed span survives, as one slice playing at the average of
        // the ramp's speeds at 3 s and 5 s: (1.375 + 1.625) / 2
        assert_eq!(slices.len(), 1);
        assert_eq!(slices[0].start.micros(), 3_000_000);
        assert_eq!(slices[0].end.micros(), 5_000_000);
        assert!((slices[0].speed - 1.5).abs() < 1e-9, "{}", slices[0].speed);
    }

    #[test]
    fn a_grade_only_emits_the_knobs_that_are_set() {
        let g = Grade {
            brightness: Some(0.1),
            contrast: None,
            saturation: Some(1.5),
            gamma: None,
            lut: None,
        };
        assert_eq!(g.eq().unwrap(), "eq=brightness=0.1000:saturation=1.5000");
        assert!(Grade::default().eq().is_none());
        assert!(Grade::default().is_noop());
    }

    #[test]
    fn a_mask_compiles_to_the_filter_that_does_it() {
        let chroma = Mask::Chroma {
            color: "0x00FF00".into(),
            similarity: 0.3,
            blend: 0.0,
        };
        assert_eq!(chroma.chain(), "colorkey=0x00FF00:0.3000:0.0000");
        let vig = Mask::Vignette { angle: 0.628 };
        assert_eq!(vig.chain(), "vignette=angle=0.6280");
    }

    #[test]
    fn grade_ranges_are_rejected_outside_their_bounds() {
        let mut p = Project::new(640, 360, Rational::new(30, 1));
        p.grade = Some(Grade {
            brightness: Some(2.0),
            ..Default::default()
        });
        assert!(p.validate().unwrap_err().to_string().contains("brightness"));
        p.grade = Some(Grade {
            saturation: Some(-1.0),
            ..Default::default()
        });
        assert!(p.validate().is_err());
    }

    #[test]
    fn a_version_1_document_upgrades_to_one_main_layer() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/plans/v1-project.json");
        let dir = std::env::temp_dir().join(format!("splice-up-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("splice.json");
        std::fs::copy(&fixture, &path).unwrap();

        let store = Store::open(&path).unwrap();
        assert_eq!(store.project.version, 2);
        let layers = &store.project.tracks.layers;
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].id, "main");
        assert_eq!(layers[0].z, 0);
        assert_eq!(layers[0].clips.len(), 2);
        assert!(layers[0].place.is_canvas());
        assert!(layers[0].motion.is_none());
        // everything else survives the upgrade untouched
        assert!(store.project.tracks.audio.is_some());
        assert_eq!(store.project.tracks.overlays.len(), 2);
        assert!(store.project.tracks.captions.is_some());
        assert!(store.project.tracks.index.is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_media_names_the_command_that_fixes_it() {
        let mut p = Project::new(640, 360, Rational::new(30, 1));
        p.main_layer_mut().unwrap().clips.push(Clip {
            id: "clip_1".into(),
            source: "/nope/gone.mp4".into(),
            src_in: Time::ZERO,
            src_out: None,
            speed: 1.0,
            transform: Transform::Fit,
            gain_db: 0.0,
            speed_keys: None,
            transition: None,
        });
        let mut media = MediaIndex::default();
        let diags = p.diagnose(&mut media);
        let d = diags.iter().find(|d| d.code == "media_missing").expect("a warning");
        assert_eq!(
            d.fix.as_deref(),
            Some("splice rm clip_1   (or put the file back)")
        );
    }

    #[test]
    fn text_past_the_end_points_at_its_own_id() {
        let mut p = Project::new(640, 360, Rational::new(30, 1));
        // A 9s clip, with src_out known, so the timeline has a length even
        // though the file does not exist.
        p.main_layer_mut().unwrap().clips.push(Clip {
            id: "clip_1".into(),
            source: "/nope/gone.mp4".into(),
            src_in: Time::ZERO,
            src_out: Some(Time::from_secs_f64(9.0)),
            speed: 1.0,
            transform: Transform::Fit,
            gain_db: 0.0,
            speed_keys: None,
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
        assert_eq!(d.fix.as_deref(), Some("splice rm text_1"));
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
            speed_keys: None,
            transition,
        }
    }

    #[test]
    fn program_duration_is_the_longest_layer_not_the_sum() {
        let mut p = Project::new(640, 360, Rational::new(30, 1));
        p.main_layer_mut().unwrap().clips = vec![clip_with("clip_1", 3.0, None)];
        p.tracks.layers.push(Layer {
            id: "pip".into(),
            z: 5,
            start: Time::from_secs_f64(1.0),
            place: Place { x: 0.6, y: 0.6, size: 0.3 },
            motion: None,
            place_keys: None,
            mask: None,
            clips: vec![clip_with("clip_9", 2.0, None)],
        });
        let mut media = MediaIndex::default();
        // main is 3s; pip starts at 1s and runs 2s -> ends at 3s. max = 3s.
        assert_eq!(p.duration(&mut media).micros(), 3_000_000);
        p.tracks.layers[1].clips[0].src_out = Some(Time::from_secs_f64(5.0));
        // pip now ends at 6s, so the program is 6s even though main is 3s.
        assert_eq!(p.duration(&mut media).micros(), 6_000_000);
    }

    #[test]
    fn z_order_comes_from_one_accessor() {
        let mut p = Project::new(640, 360, Rational::new(30, 1));
        p.tracks.layers.push(Layer {
            id: "above".into(),
            z: 10,
            start: Time::ZERO,
            place: Place::CANVAS,
            motion: None,
            place_keys: None,
            mask: None,
            clips: vec![clip_with("clip_9", 1.0, None)],
        });
        p.tracks.layers.push(Layer {
            id: "below".into(),
            z: -5,
            start: Time::ZERO,
            place: Place::CANVAS,
            motion: None,
            place_keys: None,
            mask: None,
            clips: vec![clip_with("clip_8", 1.0, None)],
        });
        let order: Vec<&str> = p.by_z().iter().map(|l| l.id.as_str()).collect();
        assert_eq!(order, ["below", "main", "above"]);
    }

    #[test]
    fn a_freshly_created_layer_may_be_empty() {
        // init writes a layer with no clips, and every command opens through
        // validate, so this has to pass or nothing works after init.
        let p = Project::new(640, 360, Rational::new(30, 1));
        assert!(p.validate().is_ok());
        assert_eq!(p.main_layer().unwrap().clips.len(), 0);
    }

    #[test]
    fn layer_rules_are_rejected() {
        let mut p = Project::new(640, 360, Rational::new(30, 1));
        p.main_layer_mut().unwrap().clips = vec![clip_with("clip_1", 3.0, None)];
        // duplicate z
        let mut dup = p.clone();
        dup.tracks.layers.push(Layer {
            id: "x".into(),
            z: 0,
            start: Time::ZERO,
            place: Place::CANVAS,
            motion: None,
            place_keys: None,
            mask: None,
            clips: vec![clip_with("clip_2", 1.0, None)],
        });
        assert!(dup.validate().unwrap_err().to_string().contains("share z"));

        // a main layer that does not fill the canvas
        let mut hole = p.clone();
        hole.main_layer_mut().unwrap().place = Place { x: 0.1, y: 0.1, size: 0.5 };
        assert!(hole.validate().unwrap_err().to_string().contains("must fill the canvas"));

        // camera keys out of order, and a zoom below 1
        let mut motion = p.clone();
        motion.tracks.layers[0].motion = Some(Motion {
            keys: vec![
                Key { t: Time::from_secs_f64(2.0), zoom: 1.2, at: Point { x: 0.5, y: 0.5 } },
                Key { t: Time::from_secs_f64(1.0), zoom: 0.5, at: Point { x: 0.5, y: 0.5 } },
            ],
        });
        assert!(motion.validate().is_err());
    }

    #[test]
    fn camera_sampling_is_linear_and_held_at_the_ends() {
        let m = Motion {
            keys: vec![
                Key { t: Time::from_secs_f64(0.0), zoom: 1.0, at: Point { x: 0.5, y: 0.5 } },
                Key { t: Time::from_secs_f64(4.0), zoom: 2.0, at: Point { x: 0.5, y: 0.1 } },
            ],
        };
        assert_eq!(m.sample(Time::from_secs_f64(-1.0)).0, 1.0);
        let (z, at) = m.sample(Time::from_secs_f64(2.0));
        assert!((z - 1.5).abs() < 1e-9);
        assert!((at.y - 0.3).abs() < 1e-6);
        assert_eq!(m.sample(Time::from_secs_f64(9.0)).0, 2.0);
        assert_eq!(m.max_zoom(), 2.0);
    }

    #[test]
    fn place_pixels_are_even_and_inside_the_canvas() {
        let (x, y, side) = Place { x: 0.5, y: 0.25, size: 0.333 }.px((1920, 1080));
        assert_eq!(side % 2, 0);
        assert_eq!((x, y), (960, 270));
    }

    #[test]
    fn transitions_shorten_the_program() {
        let mut p = Project::new(640, 360, Rational::new(30, 1));
        p.main_layer_mut().unwrap().clips = vec![
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
        p.main_layer_mut().unwrap().clips = vec![clip_with(
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
        p.main_layer_mut().unwrap().clips = vec![
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
        assert_eq!(d.fix.as_deref(), Some("splice transition clip_2 --duration 00:00:00.500"));
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
        p.main_layer_mut().unwrap().clips.push(Clip {
            id: "clip_1".into(),
            source: "/nope/gone.mp4".into(),
            src_in: Time::ZERO,
            src_out: Some(Time::from_secs_f64(9.0)),
            speed: 1.0,
            transform: Transform::Fit,
            gain_db: 0.0,
            speed_keys: None,
            transition: None,
        });
        p.tracks.captions = Some(Captions { source: "/nope/x.srt".into(), burn: true });
        let mut media = MediaIndex::default();
        for d in p.diagnose(&mut media) {
            assert!(d.fix.is_some(), "{} has no fix", d.code);
        }
    }
}
