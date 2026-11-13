//! Thin: parse, call the domain, print one JSON object.
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use serde_json::json;
use std::collections::HashSet;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::Duration;

use crate::{audio, frames, index, transcript};
use crate::graph;
use crate::media::MediaIndex;
use crate::project::{Anchor, AudioBed, Captions, Clip, Overlay, Store, TextOverlay, Transform};
use crate::render;
use crate::report::{fail, fail_from, ok};
use crate::time::{Rational, Time, TimeRange};

#[derive(Parser)]
#[command(name = "splice", version, about = "video editing CLI for agents")]
pub struct Cli {
    #[arg(long, global = true, default_value = "splice.json")]
    project: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum LayerCmd {
    /// Create a layer. `--place x,y,size` in canvas fractions.
    Add {
        id: String,
        #[arg(long)]
        z: i32,
        #[arg(long)]
        place: Option<String>,
        #[arg(long)]
        start: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Change a layer's stacking, placement or start.
    Set {
        id: String,
        #[arg(long)]
        z: Option<i32>,
        #[arg(long)]
        place: Option<String>,
        #[arg(long)]
        start: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove a layer and its clips.
    Rm {
        id: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Every layer with its order, start and placement.
    List,
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Set a field: `gemini.api_key`, `gemini.model`, `deepgram.model`, ...
    Set {
        /// <provider>.<field>, e.g. gemini.api_key
        key: String,
        /// The value, for non-secret fields like a model id.
        value: Option<String>,
        /// Read a secret from stdin. The only way a key may be typed.
        #[arg(long)]
        stdin: bool,
        /// Copy a secret out of this environment variable.
        #[arg(long)]
        from_env: Option<String>,
    },
    /// Show one field, with secrets reduced to a fingerprint.
    Get { key: String },
    /// Every provider: what is set, from where, and which model wins.
    List,
    /// Where the config file is, and whether it can be written.
    Path,
    /// Remove a field.
    Unset { key: String },
}

/// `splice cache info` / `splice cache prune [--all] [--dry-run]`.
#[derive(Subcommand)]
enum CacheCmd {
    /// What the cache holds: path, bytes, files. Needs no readable project.
    Info,
    /// Delete entries the current project's plans do not reference.
    Prune {
        /// Delete every entry, referenced or not. Needs no readable project.
        #[arg(long)]
        all: bool,
        /// Report what would be deleted, delete nothing.
        #[arg(long)]
        dry_run: bool,
    },
}

/// `splice ffmpeg install`: download a pinned, hash-checked ffmpeg and ffprobe pair into the splice cache.
#[derive(Subcommand)]
enum FfmpegCmd {
    /// Download a pinned, hash-checked ffmpeg and ffprobe into the splice cache.
    Install,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a project file.
    Init {
        #[arg(long, default_value = "1920x1080")]
        size: String,
        #[arg(long, default_value = "30")]
        fps: String,
    },
    /// Append (or insert) clips from media files.
    Add {
        files: Vec<PathBuf>,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long, default_value = "1.0")]
        speed: f64,
        #[arg(long)]
        at: Option<usize>,
        /// Crop to fill the canvas instead of padding to fit.
        #[arg(long)]
        fill: bool,
        /// Which layer to add to. Defaults to the one at z = 0.
        #[arg(long)]
        layer: Option<String>,
        /// Cut by word ids from the index: `w12` or `w12-w19`.
        #[arg(long)]
        words: Option<String>,
        /// Padding either side of a word range.
        #[arg(long, default_value = "120ms")]
        pad: String,
        /// Report the result without saving anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Show the timeline as JSON.
    List,
    /// Change a clip's in/out points.
    Trim {
        id: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        /// Cut by word ids from the index: `w12` or `w12-w19`.
        #[arg(long)]
        words: Option<String>,
        #[arg(long, default_value = "120ms")]
        pad: String,
        /// Report the result without saving anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Change a clip's playback speed (pitch preserved).
    Speed {
        id: String,
        speed: f64,
        /// Report the result without saving anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Move a clip to a position (0-based).
    Move {
        id: String,
        to: usize,
        /// Report the result without saving anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove a clip.
    Rm {
        id: String,
        /// Report the result without saving anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Add a text overlay.
    Text {
        content: String,
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        #[arg(long, default_value = "bottom")]
        pos: String,
        #[arg(long)]
        size: Option<u32>,
        #[arg(long)]
        color: Option<String>,
        /// Report the result without saving anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Place an image over the picture: a logo, a lower third, a sting.
    Image {
        source: PathBuf,
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        /// top, top-left, top-right, center, bottom, bottom-left, bottom-right.
        #[arg(long, default_value = "bottom-left")]
        pos: String,
        /// Width in pixels at the project canvas; stored as a fraction of it.
        #[arg(long)]
        width: Option<u32>,
        #[arg(long, default_value = "1.0")]
        opacity: f32,
        /// Distance from the canvas edge, in pixels at the project canvas.
        #[arg(long)]
        margin: Option<u32>,
        /// Report the result without saving anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Set (or clear) the transition that brings a clip in.
    Transition {
        id: String,
        /// crossfade or dip_to_black.
        #[arg(long, default_value = "crossfade")]
        kind: String,
        #[arg(long, default_value = "500ms")]
        duration: String,
        /// Remove the transition: a hard cut.
        #[arg(long)]
        none: bool,
        /// Report the result without saving anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Duck the music bed under speech, using the word index.
    Duck {
        /// How far to drop the bed. Negative.
        #[arg(long, default_value = "-12", allow_hyphen_values = true)]
        db: f32,
        #[arg(long, default_value = "140ms")]
        ramp: String,
        /// Words closer than this are one phrase.
        #[arg(long, default_value = "400ms")]
        gap: String,
        /// Grow each phrase by this so the duck leads the words slightly.
        #[arg(long, default_value = "80ms")]
        pad: String,
        /// Remove ducking.
        #[arg(long)]
        none: bool,
        /// Report the result without saving anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Set a music bed played under the clips.
    Audio {
        source: PathBuf,
        #[arg(long, default_value = "0", allow_hyphen_values = true)]
        gain: f32,
        /// Report the result without saving anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Attach an SRT to burn in.
    Caption {
        source: PathBuf,
        /// Report the result without saving anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove fillers and long silences from a clip, splitting it into what
    /// survives.
    Cut {
        id: String,
        /// Drop filler words (um, uh, hmm, ...).
        #[arg(long)]
        fillers: bool,
        /// Shorten every gap longer than this, e.g. `0.6s` or `>0.6s`.
        #[arg(long)]
        silences: Option<String>,
        /// How much of each long gap to keep, split at both ends.
        #[arg(long, default_value = "200ms")]
        keep: String,
        /// Air kept around a removed filler word, so words do not butt together.
        #[arg(long, default_value = "120ms")]
        pad: String,
        /// Report the cuts without touching the project.
        #[arg(long)]
        dry_run: bool,
    },
    /// Detect shot boundaries and write them into the index.
    Scenes {
        /// ffmpeg scene score that counts as a cut.
        #[arg(long, default_value = "0.4")]
        threshold: f64,
        /// Merge anything shorter than this; a flash is not a shot.
        #[arg(long, default_value = "500ms")]
        min: String,
        #[arg(long)]
        json: bool,
    },
    /// Transcribe a source's audio and attach the captions.
    ///
    /// Calls the provider chosen by `--provider`, or the first one configured.
    /// Needs a key: `splice config set gemini.api_key --stdin`.
    Transcribe {
        /// gemini or deepgram. Without it, the first configured provider wins.
        #[arg(long)]
        provider: Option<String>,
        /// Overrides the configured model for this call only.
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        language: Option<String>,
        /// Defaults to the first clip's source.
        #[arg(long)]
        audio: Option<PathBuf>,
        /// Confirm a transcription longer than the limit.
        #[arg(long)]
        yes: bool,
        /// Defaults to 10 minutes.
        #[arg(long, default_value = "10m")]
        max_duration: String,
        /// Where to write the subtitles, relative to the project.
        #[arg(long, default_value = "captions.srt")]
        srt: PathBuf,
        /// Also attach them for burn-in. Off by default: making captions and
        /// burning them into the picture are separate decisions.
        #[arg(long)]
        attach: bool,
        /// Report what would be sent, without sending it.
        #[arg(long)]
        dry_run: bool,
    },
    /// Build the word index for a source's audio (cached by audio hash).
    Index {
        #[arg(long)]
        from: PathBuf,
        #[arg(long)]
        audio: Option<PathBuf>,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        language: Option<String>,
        /// Which model produced this transcript; recorded, and a change forces
        /// a re-index because cached words from another model are not this one.
        #[arg(long)]
        model: Option<String>,
        /// Report the result without saving anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Print the indexed words in a range as compact text.
    View {
        #[arg(long)]
        range: String,
        #[arg(long)]
        json: bool,
    },
    /// Search the index for a word or phrase.
    Find {
        query: String,
        #[arg(long, default_value = "20")]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Summary plus diagnostics for the current project.
    Inspect,
    /// Render review stills or a contact sheet.
    Frames {
        #[arg(long)]
        at: Option<String>,
        #[arg(long)]
        sheet: Option<u32>,
        #[arg(short = 'o', long)]
        out: Option<PathBuf>,
        /// Read stills from the preview render instead of the final one.
        #[arg(long)]
        preview: bool,
    },
    /// Print the ffmpeg commands a render would run.
    Plan {
        /// Print the preview (proxy) plan instead of the final one.
        #[arg(long)]
        preview: bool,
        /// Include the loudness pass.
        #[arg(long)]
        loudnorm: bool,
        #[arg(long, default_value = "-14", allow_hyphen_values = true)]
        lufs: f32,
        #[arg(long, default_value = "-1", allow_hyphen_values = true)]
        true_peak: f32,
        #[arg(long, default_value = "11")]
        lra: f32,
    },
    /// Render the project. `--preview` writes a small proxy for cheap iteration.
    Render {
        #[arg(short = 'o', long)]
        out: Option<PathBuf>,
        #[arg(long, default_value = "3600")]
        timeout: u64,
        /// ffmpeg `out_time_us` tick lines on stderr. Step lines go to stderr
        /// unless silenced; ticks are on by default on a TTY, forced with this
        /// flag or `SPLICE_PROGRESS=verbose`, and silenced by
        /// `--no-progress` or `SPLICE_PROGRESS=0`.
        #[arg(long)]
        progress: bool,
        /// Silence render progress on stderr. Wins over `--progress`.
        #[arg(long)]
        no_progress: bool,
        /// Low resolution, low frame rate, own cache. For looking, not shipping.
        #[arg(long)]
        preview: bool,
        /// EBU R128 loudness targeting on the finished program.
        #[arg(long)]
        loudnorm: bool,
        /// Integrated loudness target in LUFS.
        #[arg(long, default_value = "-14", allow_hyphen_values = true)]
        lufs: f32,
        #[arg(long, default_value = "-1", allow_hyphen_values = true)]
        true_peak: f32,
        #[arg(long, default_value = "11")]
        lra: f32,
    },
    /// Add a key or set a model, in a per-user config file.
    Config {
        #[command(subcommand)]
        action: ConfigCmd,
    },
    /// Add, change or remove a video layer.
    Layer {
        #[command(subcommand)]
        action: LayerCmd,
    },
    /// Ramp a clip's speed from one value to another, in slices.
    Ramp {
        id: String,
        /// Speed at the start of the clip.
        #[arg(long)]
        from: f64,
        /// Speed at the end of the clip.
        #[arg(long)]
        to: f64,
        /// How many slices to divide the clip into. More is smoother and costs
        /// one render step each.
        #[arg(long, default_value = "4")]
        slices: usize,
        /// Remove the ramp, back to a constant speed.
        #[arg(long)]
        none: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Mask a layer: chroma key or vignette.
    Mask {
        /// The layer id.
        layer: String,
        /// Remove this colour, leaving alpha. `0x00FF00`, `green`, ...
        #[arg(long)]
        chroma: Option<String>,
        #[arg(long, default_value = "0.30")]
        similarity: f32,
        #[arg(long, default_value = "0.0")]
        blend: f32,
        /// Darken the layer's edges instead.
        #[arg(long)]
        vignette: bool,
        #[arg(long, default_value = "0.628")]
        angle: f32,
        /// Remove the mask.
        #[arg(long)]
        none: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Adjust the look of the whole program: brightness, contrast, LUT.
    Grade {
        #[arg(long, allow_hyphen_values = true)]
        brightness: Option<f32>,
        #[arg(long, allow_hyphen_values = true)]
        contrast: Option<f32>,
        #[arg(long, allow_hyphen_values = true)]
        saturation: Option<f32>,
        #[arg(long, allow_hyphen_values = true)]
        gamma: Option<f32>,
        /// A .cube 3D LUT.
        #[arg(long)]
        lut: Option<PathBuf>,
        /// Remove every adjustment.
        #[arg(long)]
        clear: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Set a camera keyframe on a layer: zoom and where it looks.
    Key {
        /// The layer id.
        layer: String,
        /// Where in the layer this key sits, e.g. `4s`.
        #[arg(long)]
        t: Option<String>,
        /// 1.0 shows the whole layer; 2.0 is twice as close.
        #[arg(long)]
        zoom: Option<f64>,
        /// Camera centre as `x,y` in 0..1 of the layer.
        #[arg(long)]
        at: Option<String>,
        /// Where the layer sits at this time: `x,y` in canvas fractions. Sliding
        /// is position only — the size does not animate.
        #[arg(long)]
        place: Option<String>,
        /// Show the keys instead of setting one.
        #[arg(long)]
        list: bool,
        /// Remove the key at this time.
        #[arg(long)]
        rm: Option<String>,
        /// Remove every key.
        #[arg(long)]
        clear: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// List the snapshots taken before each mutation, newest first.
    History {
        #[arg(long, default_value = "20")]
        limit: usize,
    },
    /// Restore the document as it was before the last N mutations.
    Undo {
        #[arg(long, default_value = "1")]
        steps: usize,
    },
    /// Check ffmpeg, fonts and credentials.
    Doctor,
    /// The render cache: `cache info` and `cache prune [--all] [--dry-run]`.
    Cache {
        #[command(subcommand)]
        cmd: CacheCmd,
    },
    /// Report whether a newer release exists, or install it with npm.
    Update {
        /// Only report; do not install anything.
        #[arg(long)]
        check: bool,
    },

    /// Fetch a verified static ffmpeg instead of using the system one.
    Ffmpeg {
        #[command(subcommand)]
        cmd: FfmpegCmd,
    },
}

pub fn run() -> ! {
    let cli = Cli::parse();
    if let Err(e) = dispatch(cli) {
        fail_from(e);
    }
    std::process::exit(0);
}

/// `--progress`/`SPLICE_PROGRESS=verbose` turn ffmpeg tick lines on;
/// `--no-progress`/`SPLICE_PROGRESS=0` silence everything and win;
/// otherwise step lines go to stderr and ticks come only on a TTY.
/// Pure so `src/cli.rs` can test it without touching process env.
fn progress_for(force: bool, quiet: bool, env: Option<&str>, tty: bool) -> render::Progress {
    if env == Some("0") || quiet {
        return render::Progress::silent();
    }
    if force || env == Some("verbose") || tty {
        return render::Progress::watch(true);
    }
    render::Progress::watch(false)
}

fn dispatch(cli: Cli) -> Result<()> {
    let path = cli.project.clone();
    match cli.cmd {
        Cmd::Init { size, fps } => {
            let (w, h) = parse_size(&size)?;
            let fps = Rational::parse(&fps)?;
            Store::create(&path, w, h, fps)?;
            ok(
                json!({
                    "project": path.display().to_string(),
                    "canvas": {"width": w, "height": h, "fps": fps.to_string()},
                }),
                &[],
            );
            Ok(())
        }
        Cmd::Add { files, from, to, speed, at, fill, layer, words, pad, dry_run } => {
            if files.is_empty() {
                bail!("usage: splice add <file>...");
            }
            let mut store = Store::open(&path)?;
            if !dry_run {
                crate::history::record(&store, "add")?;
            }
            let mut media = MediaIndex::default();
            if words.is_some() && (from.is_some() || to.is_some()) {
                bail!("usage: --words replaces --from/--to, pick one");
            }
            if words.is_some() && files.len() != 1 {
                bail!("usage: --words cuts one file; pass a single source");
            }
            let pad = Time::parse(&pad)?;
            let mut from = from.map(|s| Time::parse(&s)).transpose()?;
            let mut to = to.map(|s| Time::parse(&s)).transpose()?;
            if let (Some(f), Some(t)) = (from, to) {
                if t <= f {
                    bail!("--to must be after --from");
                }
            }
            let target = layer.unwrap_or_else(|| "main".to_string());
            if store.project.layer(&target).is_none() {
                bail!("usage: no layer {target}; `splice layer add {target} --z 1` first");
            }
            let main_len = store
                .project
                .layer(&target)
                .map(|l| l.clips.len())
                .unwrap_or(0);
            let insert_at = at.unwrap_or(main_len);
            if insert_at > main_len {
                bail!("--at {insert_at} is past the end of the timeline");
            }
            let mut added = Vec::new();
            for (offset, file) in files.iter().enumerate() {
                let abs = std::fs::canonicalize(file)
                    .with_context(|| format!("no such file: {}", file.display()))?;
                let m = media.get(&abs)?.clone();
                if !m.has_video {
                    bail!("{} has no video stream", file.display());
                }
                if let Some(spec) = &words {
                    let idx = project_index(&store)?;
                    if !same_file(&idx_source(&store), &abs) {
                        bail!(
                            "usage: the index describes {}, not {}; run `splice index --from <transcript> --audio {}`",
                            idx_source(&store).display(),
                            abs.display(),
                            abs.display()
                        );
                    }
                    let (first, last) = index::parse_word_range(spec)?;
                    let (ws, we) = index::span_for(&idx, first, last, pad)?;
                    from = Some(ws);
                    to = Some(we);
                }
                let id = store.project.next_id("clip");
                let clip = Clip {
                    transition: None,
                    speed_keys: None,
                    id: id.clone(),
                    source: store.stored_path(&abs),
                    src_in: from.unwrap_or(Time::ZERO),
                    src_out: to,
                    speed,
                    transform: if fill { Transform::Fill } else { Transform::Fit },
                    gain_db: 0.0,
                };
                let dur = clip.out_duration(m.duration);
                if let Some(l) = store.project.layer_mut(&target) {
                    l.clips.insert(insert_at + offset, clip);
                }
                added.push(json!({
                    "id": id,
                    "source": store.stored_path(&abs),
                    "duration": dur.to_string(),
                }));
            }
            finish(&mut store, &mut media, dry_run, json!({"added": added}))
        }
        Cmd::List => {
            let store = Store::open(&path)?;
            let mut media = MediaIndex::default();
            let duration = store.project.duration(&mut media);
            let clips: Vec<_> = store
                .project
                .main_layer()
                .map(|l| l.clips.as_slice())
                .unwrap_or(&[])
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let m = media.get(&store.resolve(&c.source)).ok().cloned();
                    let dur = m
                        .as_ref()
                        .map(|m| c.out_duration(m.duration))
                        .unwrap_or(Time::ZERO);
                    json!({
                        "index": i,
                        "id": c.id,
                        "source": c.source,
                        "in": c.src_in.to_string(),
                        "duration": dur.to_string(),
                        "speed": c.speed,
                        "missing": m.is_none(),
                    })
                })
                .collect();
            ok(json!({"clips": clips, "duration": duration.to_string()}), &[]);
            Ok(())
        }
        Cmd::Trim { id, from, to, words, pad, dry_run } => {
            let mut store = Store::open(&path)?;
            if !dry_run {
                crate::history::record(&store, "trim")?;
            }
            let mut media = MediaIndex::default();
            if words.is_some() && (from.is_some() || to.is_some()) {
                bail!("usage: --words replaces --from/--to, pick one");
            }
            let word_span = match &words {
                Some(spec) => {
                    let idx = project_index(&store)?;
                    let idx_src = idx_source(&store);
                    let clip_src = store.resolve(
                        &store
                            .project
                            .clip(&id)
                            .with_context(|| format!("usage: no clip {id}"))?
                            .source,
                    );
                    if !same_file(&idx_src, &clip_src) {
                        bail!(
                            "usage: the index describes {}, not {}; trim by --from/--to, or index that file",
                            idx_src.display(),
                            clip_src.display()
                        );
                    }
                    let (first, last) = index::parse_word_range(spec)?;
                    Some(index::span_for(&idx, first, last, Time::parse(&pad)?)?)
                }
                None => None,
            };
            let (li, ci) = store
                .project
                .clip_layer_index(&id)
                .with_context(|| format!("usage: no clip {id}"))?;
            let clip = &mut store.project.tracks.layers[li].clips[ci];
            if let Some((ws, we)) = word_span {
                clip.src_in = ws;
                clip.src_out = Some(we);
            }
            if let Some(f) = from {
                clip.src_in = Time::parse(&f)?;
            }
            if let Some(t) = to {
                clip.src_out = Some(Time::parse(&t)?);
            }
            if let Some(out) = clip.src_out {
                if out <= clip.src_in {
                    bail!("--to must be after --from");
                }
            }
            finish(&mut store, &mut media, dry_run, json!({"changed": [id], "invalidated": 1}))
        }
        Cmd::Speed { id, speed, dry_run } => {
            if speed <= 0.0 {
                bail!("usage: speed must be positive");
            }
            let mut store = Store::open(&path)?;
            if !dry_run {
                crate::history::record(&store, "speed")?;
            }
            let mut media = MediaIndex::default();
            let (li, ci) = store
                .project
                .clip_layer_index(&id)
                .with_context(|| format!("usage: no clip {id}"))?;
            let clip = &mut store.project.tracks.layers[li].clips[ci];
            clip.speed = speed;
            // A constant speed and a ramp are two ways to say the same thing.
            clip.speed_keys = None;
            finish(&mut store, &mut media, dry_run, json!({"changed": [id], "invalidated": 1}))
        }
        Cmd::Move { id, to, dry_run } => {
            let mut store = Store::open(&path)?;
            if !dry_run {
                crate::history::record(&store, "move")?;
            }
            let mut media = MediaIndex::default();
            let (li, pos) = store
                .project
                .clip_layer_index(&id)
                .with_context(|| format!("usage: no clip {id}"))?;
            let layer = &mut store.project.tracks.layers[li];
            let clip = layer.clips.remove(pos);
            let target = to.min(layer.clips.len());
            layer.clips.insert(target, clip);
            finish(&mut store, &mut media, dry_run, json!({"changed": [id], "invalidated": 2}))
        }
        Cmd::Rm { id, dry_run } => {
            let mut store = Store::open(&path)?;
            if !dry_run {
                crate::history::record(&store, "rm")?;
            }
            let mut media = MediaIndex::default();
            // One verb for everything removable, so a diagnostic can name a
            // command that exists instead of inventing one.
            if let Some((li, ci)) = store.project.clip_layer_index(&id) {
                let layer = &mut store.project.tracks.layers[li];
                layer.clips.remove(ci);
                if layer.clips.is_empty() && layer.z != 0 {
                    store.project.tracks.layers.remove(li);
                }
                finish(&mut store, &mut media, dry_run, json!({"changed": [id], "invalidated": 1}))
            } else if let Some(pos) = store
                .project
                .tracks
                .overlays
                .iter()
                .position(|o| o.id() == id)
            {
                store.project.tracks.overlays.remove(pos);
                finish(&mut store, &mut media, dry_run, json!({"changed": [id], "invalidated": 1}))
            } else if id == "captions" && store.project.tracks.captions.is_some() {
                store.project.tracks.captions = None;
                finish(&mut store, &mut media, dry_run, json!({"changed": ["captions"], "invalidated": 1}))
            } else if id == "audio" && store.project.tracks.audio.is_some() {
                store.project.tracks.audio = None;
                finish(&mut store, &mut media, dry_run, json!({"changed": ["audio"], "invalidated": 1}))
            } else {
                bail!(
                    "usage: nothing called {id}; try a clip id, a text id, `captions` or `audio` (splice list)"
                )
            }
        }
        Cmd::Text { content, from, to, pos, size, color, dry_run } => {
            let mut store = Store::open(&path)?;
            if !dry_run {
                crate::history::record(&store, "text")?;
            }
            let mut media = MediaIndex::default();
            let at = TimeRange::new(Time::parse(&from)?, Time::parse(&to)?)?;
            let anchor = match pos.as_str() {
                "top" => Anchor::Top,
                "top-left" => Anchor::TopLeft,
                "top-right" => Anchor::TopRight,
                "center" => Anchor::Center,
                "bottom" => Anchor::Bottom,
                "bottom-left" => Anchor::BottomLeft,
                "bottom-right" => Anchor::BottomRight,
                other => bail!(
                    "usage: --pos must be top|top-left|top-right|center|bottom|bottom-left|bottom-right, got {other}"
                ),
            };
            let id = store.project.next_id("text");
            let mut t = TextOverlay {
                id: id.clone(),
                content,
                at,
                anchor,
                size: 48,
                color: "white".into(),
            };
            if let Some(s) = size {
                t.size = s;
            }
            if let Some(c) = color {
                t.color = c;
            }
            store.project.tracks.overlays.push(Overlay::Text(t));
            finish(&mut store, &mut media, dry_run, json!({"changed": [id], "invalidated": 1}))
        }
        Cmd::Image { source, from, to, pos, width, opacity, margin, dry_run } => {
            let mut store = Store::open(&path)?;
            if !dry_run {
                crate::history::record(&store, "image")?;
            }
            let mut media = MediaIndex::default();
            let source = std::fs::canonicalize(&source)
                .with_context(|| format!("no such file: {}", source.display()))?;
            let at = TimeRange::new(Time::parse(&from)?, Time::parse(&to)?)?;
            let anchor = match pos.as_str() {
                "top" => Anchor::Top,
                "top-left" => Anchor::TopLeft,
                "top-right" => Anchor::TopRight,
                "center" => Anchor::Center,
                "bottom" => Anchor::Bottom,
                "bottom-left" => Anchor::BottomLeft,
                "bottom-right" => Anchor::BottomRight,
                other => bail!(
                    "usage: --pos must be top|top-left|top-right|center|bottom|bottom-left|bottom-right, got {other}"
                ),
            };
            if !(0.0..=1.0).contains(&opacity) {
                bail!("usage: --opacity is 0.0 to 1.0");
            }
            let canvas_w = store.project.canvas.width;
            let px = width.unwrap_or((canvas_w as f32 * 0.25) as u32).max(2);
            let m = margin.unwrap_or((canvas_w as f32 * 0.03) as u32);
            let id = store.project.next_id("image");
            store.project.tracks.overlays.push(Overlay::Image(
                crate::project::ImageOverlay {
                    id: id.clone(),
                    source: store.stored_path(&source),
                    at,
                    anchor,
                    width: px as f32 / canvas_w as f32,
                    opacity,
                    margin: m as f32 / canvas_w as f32,
                },
            ));
            finish(
                &mut store,
                &mut media,
                dry_run,
                json!({"changed": [id], "invalidated": 1, "width_px": px}),
            )
        }
        Cmd::Duck { db, ramp, gap, pad, none, dry_run } => {
            let mut store = Store::open(&path)?;
            if !dry_run {
                crate::history::record(&store, "duck")?;
            }
            let mut media = MediaIndex::default();
            let bed = store
                .project
                .tracks
                .audio
                .as_ref()
                .context("usage: no music bed; run `splice audio <file>` first")?
                .clone();
            if db >= 0.0 {
                bail!("usage: --db must be negative (it lowers the bed)");
            }

            let spans = if none {
                Vec::new()
            } else {
                let idx = project_index(&store)?;
                let idx_src = idx_source(&store);
                // Only clips cut from the indexed file can be ducked; speech
                // times from a different file mean nothing on this timeline.
                let mut clips = Vec::new();
                let mut durations = Vec::new();
                for c in store.project.clips() {
                    let src = store.resolve(&c.source);
                    if !same_file(&idx_src, &src) {
                        bail!(
                            "usage: the index describes {}, but clip {} is cut from {}; index that file, or duck only the clips from this one",
                            idx_src.display(),
                            c.id,
                            src.display()
                        );
                    }
                    let m = media.get(&src)?;
                    clips.push(c.clone());
                    durations.push(c.out_duration(m.duration));
                }
                if clips.is_empty() {
                    bail!("usage: no clips to duck under");
                }
                let words: Vec<(Time, Time)> =
                    idx.words.iter().map(|w| (w.start, w.end)).collect();
                crate::graph::speech_spans(
                    &clips,
                    &durations,
                    &words,
                    Time::parse(&gap)?,
                    Time::parse(&pad)?,
                )
            };

            let ramp = Time::parse(&ramp)?;
            let duck = if none {
                None
            } else {
                Some(crate::project::Duck {
                    db,
                    ramp,
                    spans: spans
                        .iter()
                        .map(|(s, e)| TimeRange { start: *s, end: *e })
                        .collect(),
                    layout: store.project.layout_token(),
                })
            };
            let span_count = spans.len();
            if let Some(b) = store.project.tracks.audio.as_mut() {
                b.duck = duck;
            } else {
                bail!("usage: no music bed; run `splice audio <file>` first");
            }
            finish(
                &mut store,
                &mut media,
                dry_run,
                json!({
                    "changed": ["audio"],
                    "invalidated": 1,
                    "duck_db": if none { 0.0 } else { db },
                    "phrases": span_count,
                    "bed": bed.source,
                }),
            )
        }
        Cmd::Transition { id, kind, duration, none, dry_run } => {
            let mut store = Store::open(&path)?;
            if !dry_run {
                crate::history::record(&store, "transition")?;
            }
            let mut media = MediaIndex::default();
            let (li, pos) = store
                .project
                .clip_layer_index(&id)
                .with_context(|| format!("usage: no clip {id}"))?;
            if pos == 0 {
                bail!("usage: {id} is the first clip in its layer; nothing to transition from");
            }
            let kind = if none {
                None
            } else {
                Some(
                    crate::project::TransitionKind::parse(&kind)
                        .with_context(|| format!("usage: --kind crossfade|dip_to_black, got {kind}"))?,
                )
            };
            let dur = Time::parse(&duration)?;
            // Longer than its neighbour and the graph can't be built honestly.
            let prev = store.project.tracks.layers[li].clips[pos - 1].id.clone();
            let shorter = store.project.tracks.layers[li]
                .clips
                .iter()
                .filter(|c| c.id == id || c.id == prev)
                .filter_map(|c| {
                    media
                        .get(&store.resolve(&c.source))
                        .ok()
                        .map(|m| c.out_duration(m.duration))
                })
                .min();
            if let (Some(shorter), false) = (shorter, none) {
                if dur >= shorter {
                    bail!(
                        "usage: a {} transition between {prev} and {id} is too long (shorter neighbour is {})",
                        dur, shorter
                    );
                }
            }
            store.project.tracks.layers[li].clips[pos].transition = kind.map(|k| crate::project::Transition {
                kind: k,
                duration: dur,
            });
            finish(
                &mut store,
                &mut media,
                dry_run,
                json!({"changed": [id], "invalidated": 1, "kind": if none { "cut".into() } else { kind_label(&kind) }}),
            )
        }
        Cmd::Audio { source, gain, dry_run } => {
            let mut store = Store::open(&path)?;
            if !dry_run {
                crate::history::record(&store, "audio")?;
            }
            let mut media = MediaIndex::default();
            let source = std::fs::canonicalize(&source)
                .with_context(|| format!("no such file: {}", source.display()))?;
            // Replacing the bed keeps any existing duck settings, but the spans
            // are stale until `splice duck` runs again.
            let duck = store.project.tracks.audio.as_ref().and_then(|b| b.duck.clone());
            store.project.tracks.audio = Some(AudioBed {
                source: store.stored_path(&source),
                gain_db: gain,
                duck,
            });
            finish(&mut store, &mut media, dry_run, json!({"changed": ["audio"], "invalidated": 1}))
        }
        Cmd::Caption { source, dry_run } => {
            let mut store = Store::open(&path)?;
            if !dry_run {
                crate::history::record(&store, "caption")?;
            }
            let mut media = MediaIndex::default();
            let source = std::fs::canonicalize(&source)
                .with_context(|| format!("no such file: {}", source.display()))?;
            let cues = crate::caption::load(&source)
                .with_context(|| format!("reading {}", source.display()))?;
            store.project.tracks.captions = Some(Captions {
                source: store.stored_path(&source),
                burn: true,
            });
            finish(
                &mut store,
                &mut media,
                dry_run,
                json!({"changed": ["captions"], "cues": cues.len(), "invalidated": 1}),
            )
        }
        Cmd::Cut { id, fillers, silences, keep, pad, dry_run } => {
            let mut store = Store::open(&path)?;
            let mut media = MediaIndex::default();
            if !fillers && silences.is_none() {
                bail!("usage: pass --fillers, --silences <threshold>, or both");
            }
            let threshold = match &silences {
                Some(raw) => {
                    let cleaned = raw.trim().trim_start_matches('>').trim_start_matches('=');
                    Some(Time::parse(cleaned).with_context(|| {
                        format!("usage: --silences takes a duration like 0.6s, got `{raw}`")
                    })?)
                }
                None => None,
            };
            let keep = Time::parse(&keep)?;
            let pad = Time::parse(&pad)?;

            let idx = project_index(&store)?;
            let idx_src = idx_source(&store);
            let (li, pos) = store
                .project
                .clip_layer_index(&id)
                .with_context(|| format!("usage: no clip {id}"))?;
            let base = store.project.tracks.layers[li].clips[pos].clone();
            let clip_src = store.resolve(&base.source);
            if !same_file(&idx_src, &clip_src) {
                bail!(
                    "usage: the index describes {}, not {}; index that file first",
                    idx_src.display(),
                    clip_src.display()
                );
            }

            let range = TimeRange {
                start: base.src_in,
                end: base.src_out.unwrap_or_else(|| {
                    media
                        .get(&clip_src)
                        .map(|m| m.duration)
                        .unwrap_or(Time::ZERO)
                }),
            };
            let cut = index::plan_cut(&idx, range, fillers, threshold, keep, pad);
            if cut.remove.is_empty() {
                ok(json!({"changed": [], "removed": [], "saved": "00:00:00.000"}), &[]);
                return Ok(());
            }
            if cut.kept.is_empty() {
                bail!("usage: that would remove all of {id}; loosen --silences or --keep");
            }

            let removed: Vec<serde_json::Value> = cut
                .remove
                .iter()
                .map(|r| json!({"from": r.start.to_string(), "to": r.end.to_string()}))
                .collect();
            let saved_us: i64 = cut
                .remove
                .iter()
                .map(|r| r.end.micros() - r.start.micros())
                .sum();

            let mut extra = Vec::new();
            if store.project.tracks.captions.is_some() {
                extra.push(
                    crate::report::Diagnostic::new(
                        "cut_shifts_layers",
                        "tracks.captions",
                        "captions were timed against the old timeline and are now offset",
                    )
                    .with_fix(format!("splice undo   (or re-run: splice caption {})", 
                        store.project.tracks.captions.as_ref().map(|c| c.source.clone()).unwrap_or_default())),
                );
            }
            if !store.project.tracks.overlays.is_empty() {
                extra.push(
                    crate::report::Diagnostic::new(
                        "cut_shifts_layers",
                        "tracks.overlays",
                        "text overlays were timed against the old timeline and are now offset",
                    )
                    .with_fix("splice undo   (or re-add the overlays at their new times)"),
                );
            }
            if dry_run {
                return finish_ext(
                    &mut store,
                    &mut media,
                    true,
                    json!({
                        "changed": [],
                        "would_replace": id,
                        "removed": removed,
                        "saved": Time::from_micros(saved_us).to_string(),
                    }),
                    &extra,
                );
            }

            crate::history::record(&store, "cut")?;
            store.project.tracks.layers[li].clips.remove(pos);
            let mut new_ids = Vec::new();
            for (k, span) in cut.kept.iter().enumerate() {
                let new_id = store.project.next_id("clip");
                let mut clip = base.clone();
                clip.id = new_id.clone();
                clip.src_in = span.start;
                clip.src_out = Some(span.end);
                store.project.tracks.layers[li].clips.insert(pos + k, clip);
                new_ids.push(new_id);
            }

            finish_ext(
                &mut store,
                &mut media,
                false,
                json!({
                    "changed": [id],
                    "replaced_by": new_ids,
                    "removed": removed,
                    "saved": Time::from_micros(saved_us).to_string(),
                }),
                &extra,
            )
        }
        Cmd::Scenes { threshold, min, json: as_json } => {
            let store = Store::open(&path)?;
            let idx_ref = store
                .project
                .tracks
                .index
                .as_ref()
                .context("no_index: run `splice index --from <transcript>` first")?;
            let source = store.resolve(&idx_ref.source);
            let min_len = Time::parse(&min)?;
            let idx_path = project_index_path(&store)?;
            let mut idx = index::Index::load(&idx_path)?;

            let stat = crate::cache::stat_token(&source);
            let mut cached = false;
            let scenes = match &idx.scenes {
                Some(s) if s.source_stat == stat && (s.threshold - threshold as f32).abs() < 0.001
                    && s.min_len == min_len =>
                {
                    cached = true;
                    s.scenes.clone()
                }
                _ => {
                    let cuts = detect_cuts(&source, threshold)?;
                    let duration = crate::media::probe(&source)?.duration;
                    let merged = index::merge_scenes(cuts, duration, min_len);
                    idx.scenes = Some(index::Scenes {
                        source_stat: stat,
                        threshold: threshold as f32,
                        min_len,
                        scenes: merged.clone(),
                    });
                    idx.save(&idx_path)?;
                    merged
                }
            };

            if as_json {
                let body: Vec<serde_json::Value> = scenes
                    .iter()
                    .enumerate()
                    .map(|(i, s)| json!({
                        "n": i + 1,
                        "start": s.start.to_string(),
                        "end": s.end.to_string(),
                        "keyframe": s.keyframe.to_string(),
                    }))
                    .collect();
                ok(json!({"cached": cached, "threshold": threshold, "scenes": body}), &[]);
            } else {
                for (i, s) in scenes.iter().enumerate() {
                    if !crate::report::emit_line(&format!(
                        "{}-{}  key {}  scene {}",
                        index::clock(s.start),
                        index::clock(s.end),
                        index::clock(s.keyframe),
                        i + 1
                    )) {
                        break;
                    }
                }
            }
            Ok(())
        }
        Cmd::Transcribe { provider, model, language, audio: audio_arg, yes, max_duration, srt, attach, dry_run } => {
            let mut store = Store::open(&path)?;
            let cfg = crate::config::load()?;
            let env = |v: &str| std::env::var(v).ok();
            let flagged = match &provider {
                Some(p) => Some(
                    crate::config::Provider::parse(p)
                        .with_context(|| format!("usage: --provider gemini|deepgram, got `{p}`"))?,
                ),
                None => None,
            };
            let (chosen, resolved) = crate::asr::choose(flagged, &cfg, env)?;
            let model = model.unwrap_or(resolved.model.clone());

            let source = match audio_arg {
                Some(p) => std::fs::canonicalize(&p)
                    .with_context(|| format!("no such file: {}", p.display()))?,
                None => {
                    let first = store
                        .project
                        .main_layer()
                        .and_then(|l| l.clips.first())
                        .context("usage: no clips yet; pass --audio <file>")?;
                    store.resolve(&first.source)
                }
            };
            let mut media = MediaIndex::default();
            let duration = media.get(&source)?.duration;
            let limit = Time::parse(&max_duration)?;
            crate::asr::check_duration(duration, yes, limit)?;

            let audio = crate::audio::asr_audio(&store, &source)?;
            let bytes = std::fs::read(&audio.path)?;
            let key = crate::cache::hash(&[
                audio.sha256.clone(),
                chosen.id().into(),
                model.clone(),
                language.clone().unwrap_or_default(),
            ]);

            if dry_run {
                ok(
                    json!({
                        "dry_run": true,
                        "provider": chosen.id(),
                        "model": model,
                        "language": language,
                        "audio": audio.path.display().to_string(),
                        "bytes": bytes.len(),
                        "duration": duration.to_string(),
                        "endpoint": crate::asr::endpoint(chosen),
                        "key_source": resolved.key_source.map(origin_name),
                        "cache_key": key,
                    }),
                    &[],
                );
                return Ok(());
            }

            let transcript_path = store
                .dir()
                .join(".splice")
                .join("asr")
                .join(format!("{key}.json"));
            let mut cached = false;
            let parsed = match index::Index::load(&transcript_path) {
                Ok(existing) => {
                    cached = true;
                    crate::transcript::Parsed {
                        words: existing.words.clone(),
                        provider: existing.provider.clone(),
                        language: existing.language.clone(),
                        approximate: existing.approximate,
                    }
                }
                Err(_) => {
                    if chosen == crate::config::Provider::Whisper {
                        crate::asr::transcribe_whisper(&cfg, &model, language.as_deref(), &audio.path)?
                    } else {
                        let secret = resolved
                            .key
                            .as_ref()
                            .context("asr_no_key: no key resolved for this provider")?;
                        // No prose wrapper: report::fail_from classifies on the
                        // leading `asr_*` code, and a context layer hides it.
                        crate::asr::transcribe(chosen, &model, language.as_deref(), &bytes, secret)?
                    }
                }
            };

            if parsed.words.is_empty() {
                // Silence is a valid answer from every provider; say so instead
                // of writing an empty index and an empty subtitle file.
                ok(
                    json!({
                        "provider": chosen.id(),
                        "model": model,
                        "words": 0,
                        "cues": 0,
                        "cached": cached,
                        "captions": null,
                        "speech": false,
                    }),
                    &[crate::report::Diagnostic::new(
                        "asr_no_speech",
                        "audio",
                        "the provider found no speech in this audio",
                    )
                    .with_fix("check the source, or pass --audio with the right file")],
                );
                return Ok(());
            }

            let mut words = parsed.words.clone();
            index::assign_ids(&mut words);
            let idx = index::Index {
                version: index::VERSION,
                audio_hash: audio.sha256.clone(),
                audio_duration: duration,
                transcript_hash: key.clone(),
                provider: parsed.provider.clone(),
                language: language.clone().or(parsed.language.clone()),
                approximate: parsed.approximate,
                scenes: None,
                model: Some(model.clone()),
                words,
            };

            if !dry_run {
                crate::history::record(&store, "transcribe")?;
                idx.save(&transcript_path)?;
            }

            // Captions from the same words, so the SRT and the index cannot drift.
            let cues: Vec<(Time, Time, String)> = idx
                .utterances()
                .into_iter()
                .map(|u| (u.start, u.end, u.text))
                .collect();
            // Subtitles go beside the project, not into `.splice/`: the cache is
            // documented as deletable, and a caption file the project points at
            // must not be.
            let srt_path = store.resolve(&srt.to_string_lossy());
            let srt_body = crate::caption::to_srt(&cues);
            if !dry_run {
                std::fs::write(&srt_path, &srt_body)
                    .with_context(|| format!("writing {}", srt_path.display()))?;
            }

            let mut warnings: Vec<crate::report::Diagnostic> = Vec::new();
            if attach {
                store.project.tracks.captions = Some(crate::project::Captions {
                    source: store.stored_path(&srt_path),
                    burn: true,
                });
            }
            store.project.tracks.index = Some(crate::project::IndexRef {
                audio_hash: audio.sha256.clone(),
                source: store.stored_path(&source),
                transcript: store.stored_path(&transcript_path),
                provider: idx.provider.clone(),
                language: idx.language.clone(),
                words: idx.words.len(),
                approximate: idx.approximate,
                model: Some(model.clone()),
            });
            if let Some(bed) = &store.project.tracks.audio {
                if bed.duck.is_some() {
                    warnings.push(
                        crate::report::Diagnostic::new(
                            "duck_stale",
                            "tracks.audio.duck",
                            "the word times changed; duck spans were mapped against the old ones",
                        )
                        .with_fix("sona duck"),
                    );
                }
            }
            finish_ext(
                &mut store,
                &mut media,
                false,
                json!({
                    "provider": idx.provider,
                    "model": model,
                    "language": idx.language,
                    "words": idx.words.len(),
                    "cues": cues.len(),
                    "cached": cached,
                    "captions": srt_path.display().to_string(),
                    "attached": attach,
                    "next": if attach { json!(null) } else { json!(format!("splice caption {}", srt_path.display())) },
                    "index": transcript_path.display().to_string(),
                }),
                &warnings,
            )
        }
        Cmd::Index { from, audio: audio_arg, force, language, model, dry_run } => {
            let mut store = Store::open(&path)?;
            if !dry_run {
                crate::history::record(&store, "index")?;
            }
            let source = match audio_arg {
                Some(p) => std::fs::canonicalize(&p)
                    .with_context(|| format!("no such file: {}", p.display()))?,
                None => {
                    // Default to the project's source, so a re-index after
                    // changing the transcript does not need --audio again.
                    if let Some(first) = store.project.main_layer().and_then(|l| l.clips.first()) {
                        store.resolve(&first.source)
                    } else if let Some(existing) = &store.project.tracks.index {
                        store.resolve(&existing.source)
                    } else {
                        bail!("usage: no clips yet; pass --audio <file>")
                    }
                }
            };
            let fp = audio::fingerprint(&store, &source)?;
            let transcript_path = std::fs::canonicalize(&from)
                .with_context(|| format!("no such file: {}", from.display()))?;
            let transcript_hash = audio::sha256_file(&transcript_path)?;
            let cache_root = store.dir().join(".splice");
            let idx_path = index::Index::path(&cache_root, &fp.sha256);

            let model = model.or_else(|| {
                let cfg = crate::config::load().unwrap_or_default();
                let p = crate::config::Provider::parse(
                    transcript::parse_file(&transcript_path).ok().map(|t| t.provider).as_deref().unwrap_or(""),
                );
                p.map(|p| crate::config::Resolved::get(p, &cfg, None, |v| std::env::var(v).ok()).model)
            });
            let mut cached = false;
            let idx = match index::Index::load(&idx_path) {
                Ok(existing)
                    if !force
                        && existing.transcript_hash == transcript_hash
                        && existing.model == model =>
                {
                    cached = true;
                    existing
                }
                _ => {
                    let mut parsed = transcript::parse_file(&transcript_path)?;
                    index::assign_ids(&mut parsed.words);
                    let idx = index::Index {
                        version: index::VERSION,
                        audio_hash: fp.sha256.clone(),
                        audio_duration: fp.duration,
                        transcript_hash,
                        provider: parsed.provider,
                        language: language.or(parsed.language),
                        approximate: parsed.approximate,
                        words: parsed.words,
                        scenes: None,
                        model: model.clone(),
                    };
                    if !dry_run {
                        idx.save(&idx_path)?;
                    }
                    idx
                }
            };

            store.project.tracks.index = Some(crate::project::IndexRef {
                audio_hash: fp.sha256.clone(),
                source: store.stored_path(&source),
                transcript: store.stored_path(&transcript_path),
                provider: idx.provider.clone(),
                language: idx.language.clone(),
                words: idx.words.len(),
                approximate: idx.approximate,
                model: idx.model.clone(),
            });
            if !dry_run {
                store.save()?;
            }
            let mut warnings = Vec::new();
            if let (Some(last), dur) = (idx.words.last(), idx.audio_duration) {
                if last.end.micros() > dur.micros() + 2_000_000 {
                    warnings.push(
                        crate::report::Diagnostic::new(
                            "index_past_audio",
                            "words[-1].end",
                            format!(
                                "the transcript ends at {} but the audio is only {}",
                                last.end, dur
                            ),
                        )
                        .with_fix("splice index --from <the transcript for this file> --audio <this file>"),
                    );
                }
            }
            ok(
                json!({
                    "audio_hash": fp.sha256,
                    "audio_duration": idx.audio_duration.to_string(),
                    "provider": idx.provider,
                    "model": idx.model,
                    "language": idx.language,
                    "approximate": idx.approximate,
                    "words": idx.words.len(),
                    "cached": cached,
                    "index": idx_path.display().to_string(),
                }),
                &warnings,
            );
            Ok(())
        }
        Cmd::View { range, json: as_json } => {
            let store = Store::open(&path)?;
            let idx = project_index(&store)?;
            let (start, end) = parse_range(&range)?;
            let lines: Vec<index::Utterance> = idx
                .utterances()
                .into_iter()
                .filter(|u| u.end > start && u.start < end)
                .collect();
            if as_json {
                let body: Vec<serde_json::Value> = lines
                    .iter()
                    .map(|u| json!({
                        "start": u.start.to_string(),
                        "end": u.end.to_string(),
                        "first": u.first,
                        "last": u.last,
                        "text": u.text,
                    }))
                    .collect();
                ok(
                    json!({"range": {"start": start.to_string(), "end": end.to_string()}, "lines": body}),
                    &[],
                );
            } else if lines.is_empty() {
                eprintln!("view: no words between {start} and {end}");
            } else {
                for u in &lines {
                    if !crate::report::emit_line(&index::format_line(u)) {
                        break;
                    }
                }
            }
            Ok(())
        }
        Cmd::Find { query, limit, json: as_json } => {
            let store = Store::open(&path)?;
            let idx = project_index(&store)?;
            let hits: Vec<index::Hit> = idx.find(&query).into_iter().take(limit).collect();
            if as_json {
                let body: Vec<serde_json::Value> = hits
                    .iter()
                    .map(|h| json!({
                        "start": h.utterance.start.to_string(),
                        "end": h.utterance.end.to_string(),
                        "first": h.utterance.first,
                        "last": h.utterance.last,
                        "match": idx.words[h.utterance.first_i + h.match_start..h.utterance.first_i + h.match_start + h.match_len]
                            .iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "),
                        "text": h.utterance.text,
                        "line": index::format_hit(&idx, h),
                    }))
                    .collect();
                ok(json!({"query": query, "hits": body}), &[]);
            } else {
                for h in &hits {
                    if !crate::report::emit_line(&index::format_hit(&idx, h)) {
                        break;
                    }
                }
            }
            Ok(())
        }
        Cmd::Inspect => {
            let store = Store::open(&path)?;
            let mut media = MediaIndex::default();
            let duration = store.project.duration(&mut media);
            let diags = store.project.diagnose(&mut media);
            let clips = store
                .project
                .clips()
                .map(|c| {
                    json!({
                        "id": c.id,
                        "source": c.source,
                        "in": c.src_in.to_string(),
                        "out": c.src_out.map(|t| t.to_string()),
                        "speed": c.speed,
                    })
                })
                .collect::<Vec<_>>();
            ok(
                json!({
                    "canvas": {"width": store.project.canvas.width, "height": store.project.canvas.height, "fps": store.project.canvas.fps.to_string()},
                    "duration": duration.to_string(),
                    "clips": clips,
                    "overlays": store.project.tracks.overlays.len(),
                    "captions": store.project.tracks.captions.is_some(),
                    "audio_bed": store.project.tracks.audio.is_some(),
                    "index": store.project.tracks.index.as_ref().map(|i| json!({
                        "audio_hash": i.audio_hash, "words": i.words,
                        "provider": i.provider, "approximate": i.approximate,
                    })),
                }),
                &diags,
            );
            Ok(())
        }
        Cmd::Frames { at, sheet, out, preview } => {
            let store = Store::open(&path)?;
            let mut media = MediaIndex::default();
            let profile = if preview { graph::Profile::Preview } else { graph::Profile::Final };
            let plan = graph::plan(&store, &mut media, &PathBuf::from("out.mp4"), profile, None)?;
            let source = plan.last_output().to_path_buf();
            if !source.exists() {
                let cmd = if preview { "splice render --preview" } else { "splice render" };
                fail(
                    "nothing_rendered",
                    format!("nothing rendered yet (run `{cmd}`)"),
                    Some(cmd.to_string()),
                );
            }
            // Separate directories, because a proxy still and a final still of
            // the same timestamp would otherwise overwrite each other.
            let out_dir = store.resolve(
                &out
                    .unwrap_or_else(|| {
                        PathBuf::from(if preview {
                            ".splice/preview-proxy"
                        } else {
                            ".splice/preview"
                        })
                    })
                    .to_string_lossy(),
            );
            let duration = plan.duration;
            let mut produced = Vec::new();
            if let Some(at) = at {
                let times = frames::parse_frame_times(&at, duration)?;
                for p in frames::stills(&source, &times, &out_dir)? {
                    produced.push(json!({"path": p.display().to_string()}));
                }
            }
            if let Some(n) = sheet {
                let p = frames::contact_sheet(&source, n, duration, &out_dir.join("sheet.png"))?;
                produced.push(json!({"path": p.display().to_string(), "kind": "sheet"}));
            }
            if produced.is_empty() {
                bail!("usage: pass --at <times> or --sheet <n>");
            }
            ok(json!({"frames": produced}), &plan.warnings);
            Ok(())
        }
        Cmd::Plan { preview, loudnorm, lufs, true_peak, lra } => {
            let store = Store::open(&path)?;
            let mut media = MediaIndex::default();
            let profile = if preview { graph::Profile::Preview } else { graph::Profile::Final };
            let loudness = loudness_arg(loudnorm, lufs, true_peak, lra);
            let plan = graph::plan(&store, &mut media, &PathBuf::from("out.mp4"), profile, loudness)?;
            ok(graph::plan_json(&plan, profile), &plan.warnings);
            Ok(())
        }
        Cmd::Render {
            out,
            timeout,
            preview,
            loudnorm,
            lufs,
            true_peak,
            lra,
            progress,
            no_progress,
        } => {
            let store = Store::open(&path)?;
            let mut media = MediaIndex::default();
            let profile = if preview { graph::Profile::Preview } else { graph::Profile::Final };
            let target = out.unwrap_or_else(|| {
                PathBuf::from(if preview { "preview.mp4" } else { "out.mp4" })
            });
            let out_path = store.resolve(&target.to_string_lossy());
            let loudness = loudness_arg(loudnorm, lufs, true_peak, lra);
            let plan = graph::plan(&store, &mut media, &out_path, profile, loudness)?;
            let env = std::env::var("SPLICE_PROGRESS").ok();
            let progress = progress_for(
                progress,
                no_progress,
                env.as_deref(),
                std::io::stderr().is_terminal(),
            );
            let summary = render::execute(&plan, Duration::from_secs(timeout), &progress)?;
            let canvas = profile.canvas(&store.project.canvas);
            ok(
                json!({
                    "out": out_path.display().to_string(),
                    "profile": profile.tag(),
                    "size": format!("{}x{}", canvas.0, canvas.1),
                    "fps": profile.fps(store.project.canvas.fps).to_string(),
                    "duration": plan.duration.to_string(),
                    "loudness": loudness.map(|l| json!({"i": l.i, "tp": l.tp, "lra": l.lra})),
                    "steps": {"total": summary.total, "ran": summary.ran, "reused": summary.reused},
                }),
                &plan.warnings,
            );
            Ok(())
        }
        Cmd::Layer { action } => {
            let mut store = Store::open(&path)?;
            let mut media = MediaIndex::default();
            match action {
                LayerCmd::Add { id, z, place, start, dry_run } => {
                    if store.project.layer(&id).is_some() {
                        bail!("usage: a layer called {id} already exists");
                    }
                    let place = match place {
                        Some(p) => parse_place(&p)?,
                        None => crate::project::Place::CANVAS,
                    };
                    let start = start.map(|s| Time::parse(&s)).transpose()?.unwrap_or(Time::ZERO);
                    store.project.tracks.layers.push(crate::project::Layer {
                        id: id.clone(),
                        z,
                        start,
                        place,
                        motion: None,
                        place_keys: None,
                        mask: None,
                        clips: Vec::new(),
                    });
                    // Never save a document the next command would refuse: a
                    // wrong `--z` or a non-canvas `place` on `z = 0` is a usage
                    // error (exit 5), not `project_invalid`, and no snapshot is
                    // taken when it fails.
                    store
                        .project
                        .validate()
                        .map_err(|e| anyhow::anyhow!("usage: {e}"))?;
                    if !dry_run {
                        crate::history::record(&store, "layer add")?;
                    }
                    finish(&mut store, &mut media, dry_run, json!({"added": id, "z": z}))?;
                }
                LayerCmd::Set { id, z, place, start, dry_run } => {
                    let start_t = start.map(|s| Time::parse(&s)).transpose()?;
                    let p = place.map(|p| parse_place(&p)).transpose()?;
                    // Read the layer through the immutable accessor first: the
                    // refusal must come before history::record, so a refused
                    // edit takes no snapshot and writes nothing. The condition
                    // is bottom-of-by_z() (lowest z), never z == 0.
                    store
                        .project
                        .layer(&id)
                        .with_context(|| format!("usage: no layer {id}"))?;
                    let is_bottom = store
                        .project
                        .by_z()
                        .first()
                        .map(|l| l.id == id)
                        .unwrap_or(false);
                    if is_bottom {
                        if let Some(s) = start_t {
                            if s.micros() != 0 {
                                bail!(
                                    "usage: {id} is the bottom layer (normally the one at z = 0); its start is not applied and its clips play from 0. Start a layer above it (`splice layer add pip --z 10`), or reset it with `splice layer set {id} --start 0`"
                                );
                            }
                        }
                        if let Some(p) = p {
                            if !p.is_canvas() {
                                bail!(
                                    "usage: {id} is the bottom layer (normally the one at z = 0); its place is ignored because the program frame is the full canvas. `--place` applies to layers above it"
                                );
                            }
                        }
                    }
                    let layer = store
                        .project
                        .layer_mut(&id)
                        .with_context(|| format!("usage: no layer {id}"))?;
                    if let Some(z) = z {
                        layer.z = z;
                    }
                    if let Some(p) = p {
                        layer.place = p;
                    }
                    if let Some(s) = start_t {
                        layer.start = s;
                    }
                    // Never save a document the next command would refuse: a
                    // wrong `--z` or a non-canvas `place` on `z = 0` is a usage
                    // error (exit 5), not `project_invalid`, and no snapshot is
                    // taken when it fails.
                    store
                        .project
                        .validate()
                        .map_err(|e| anyhow::anyhow!("usage: {e}"))?;
                    if !dry_run {
                        crate::history::record(&store, "layer set")?;
                    }
                    finish(&mut store, &mut media, dry_run, json!({"changed": [id]}))?;
                }
                LayerCmd::Rm { id, dry_run } => {
                    if id == "main" {
                        bail!("usage: `main` is the layer at z = 0 and carries the program audio; set another layer to z = 0 first");
                    }
                    if !dry_run {
                        crate::history::record(&store, "layer rm")?;
                    }
                    let before = store.project.tracks.layers.len();
                    store.project.tracks.layers.retain(|l| l.id != id);
                    if store.project.tracks.layers.len() == before {
                        bail!("usage: no layer {id}");
                    }
                    finish(&mut store, &mut media, dry_run, json!({"removed": id}))?;
                }
                LayerCmd::List => {
                    let layers: Vec<serde_json::Value> = store
                        .project
                        .by_z()
                        .into_iter()
                        .map(|l| {
                            json!({
                                "id": l.id, "z": l.z,
                                "start": l.start.to_string(),
                                "place": {"x": l.place.x, "y": l.place.y, "size": l.place.size},
                                "clips": l.clips.len(),
                                "keys": l.motion.as_ref().map(|m| m.keys.len()).unwrap_or(0),
                            })
                        })
                        .collect();
                    let duration = store.project.duration(&mut media);
                    ok(json!({"layers": layers, "duration": duration.to_string()}), &[])
                }
            }
            Ok(())
        }
        Cmd::Ramp { id, from, to, slices, none, dry_run } => {
            let mut store = Store::open(&path)?;
            let mut media = MediaIndex::default();
            if !dry_run {
                crate::history::record(&store, "ramp")?;
            }
            if !none && (from <= 0.0 || to <= 0.0) {
                bail!("usage: speeds must be positive");
            }
            if slices == 0 || slices > 64 {
                bail!("usage: --slices takes 1 to 64");
            }
            let (li, ci) = store
                .project
                .clip_layer_index(&id)
                .with_context(|| format!("usage: no clip {id}"))?;
            if none {
                store.project.tracks.layers[li].clips[ci].speed_keys = None;
                return finish(&mut store, &mut media, dry_run, json!({"changed": [id], "ramp": null}));
            }
            // read what the ramp needs before taking a mutable borrow
            let (src, start, src_out) = {
                let c = &store.project.tracks.layers[li].clips[ci];
                (store.resolve(&c.source), c.src_in, c.src_out)
            };
            let m = media.get(&src)?.clone();
            let end = src_out.unwrap_or(m.duration).min(m.duration).max(start);
            if end <= start {
                bail!("usage: {id} has no source span to ramp over");
            }
            let span = end.saturating_sub(start).secs_f64();
            // keys at the slice boundaries, speeds linearly interpolated
            let mut keys = Vec::new();
            for i in 0..=slices {
                let f = i as f64 / slices as f64;
                keys.push(crate::project::SpeedKey {
                    t: Time::from_secs_f64(start.secs_f64() + span * f),
                    speed: from + (to - from) * f,
                });
            }
            store.project.tracks.layers[li].clips[ci].speed_keys = Some(keys);
            store.project.validate()?;
            let dur = store.project.tracks.layers[li].clips[ci].out_duration(m.duration);
            finish(
                &mut store,
                &mut media,
                dry_run,
                json!({"changed": [id], "from": from, "to": to, "slices": slices, "clip_duration": dur.to_string()}),
            )
        }
        Cmd::Mask { layer, chroma, similarity, blend, vignette, angle, none, dry_run } => {
            let mut store = Store::open(&path)?;
            let mut media = MediaIndex::default();
            if !dry_run {
                crate::history::record(&store, "mask")?;
            }
            let mask = if none {
                None
            } else {
                match (chroma, vignette) {
                    (Some(color), false) => Some(crate::project::Mask::Chroma {
                        color,
                        similarity,
                        blend,
                    }),
                    (None, true) => Some(crate::project::Mask::Vignette { angle }),
                    (Some(_), true) => bail!("usage: pass --chroma or --vignette, not both"),
                    (None, false) => bail!("usage: pass --chroma <color>, --vignette, or --none"),
                }
            };
            let z = store
                .project
                .layer(&layer)
                .with_context(|| format!("usage: no layer {layer}"))?
                .z;
            if mask.is_some() && z == 0 {
                bail!(
                    "usage: {layer} is the bottom layer; a mask there reveals nothing. Mask a layer above z = 0, or add one with `splice layer add`"
                );
            }
            let l = store
                .project
                .layer_mut(&layer)
                .with_context(|| format!("usage: no layer {layer}"))?;
            l.mask = mask.clone();
            store.project.validate()?;
            finish(
                &mut store,
                &mut media,
                dry_run,
                json!({"changed": [layer], "mask": mask.map(|m| m.chain())}),
            )
        }
        Cmd::Grade { brightness, contrast, saturation, gamma, lut, clear, dry_run } => {
            let mut store = Store::open(&path)?;
            let mut media = MediaIndex::default();
            if !dry_run {
                crate::history::record(&store, "grade")?;
            }
            if clear {
                store.project.grade = None;
                return finish(&mut store, &mut media, dry_run, json!({"changed": ["grade"], "cleared": true}));
            }
            let lut_path = match lut {
                Some(p) => {
                    let abs = std::fs::canonicalize(&p)
                        .with_context(|| format!("no such LUT: {}", p.display()))?;
                    Some(store.stored_path(&abs))
                }
                None => None,
            };
            let existing = store.project.grade.clone().unwrap_or_default();
            let grade = crate::project::Grade {
                brightness: brightness.or(existing.brightness),
                contrast: contrast.or(existing.contrast),
                saturation: saturation.or(existing.saturation),
                gamma: gamma.or(existing.gamma),
                lut: lut_path.or(existing.lut),
            };
            if grade.is_noop() {
                bail!("usage: give at least one of --brightness, --contrast, --saturation, --gamma, --lut");
            }
            store.project.grade = Some(grade.clone());
            // validate the ranges now rather than at render time
            store.project.validate()?;
            finish(
                &mut store,
                &mut media,
                dry_run,
                json!({
                    "changed": ["grade"],
                    "brightness": grade.brightness,
                    "contrast": grade.contrast,
                    "saturation": grade.saturation,
                    "gamma": grade.gamma,
                    "lut": grade.lut,
                }),
            )
        }
        Cmd::Key { layer, t, zoom, at, place, list, rm, clear, dry_run } => {
            let mut store = Store::open(&path)?;
            let mut media = MediaIndex::default();
            let at_point = at.as_deref().map(parse_point).transpose()?;
            let place_point = place.as_deref().map(parse_point).transpose()?;
            // Read the layer through the immutable accessor first: the refusal
            // must come before history::record, so a refused edit takes no
            // snapshot and writes nothing. The condition is bottom-of-by_z()
            // (lowest z, normally z = 0), never z == 0 or "single layer".
            store
                .project
                .layer(&layer)
                .with_context(|| format!("usage: no layer {layer}"))?;
            if place_point.is_some()
                && store
                    .project
                    .by_z()
                    .first()
                    .map(|l| l.id == *layer)
                    .unwrap_or(false)
            {
                bail!(
                    "usage: {layer} is the bottom layer (normally the one at z = 0), so placement keys on it are never applied; add a layer above it (`splice layer add pip --z 10`) and key that, or use `--zoom`/`--at` for a camera move"
                );
            }
            if !dry_run && !list {
                crate::history::record(&store, "key")?;
            }
            let layer_ref = store
                .project
                .layer_mut(&layer)
                .with_context(|| format!("usage: no layer {layer}"))?;
            if clear {
                layer_ref.motion = None;
                layer_ref.place_keys = None;
                return finish(&mut store, &mut media, dry_run, json!({"changed": [layer], "keys": 0}));
            }
            if let Some(raw) = rm {
                let t = Time::parse(&raw)?;
                if let Some(m) = layer_ref.motion.as_mut() {
                    m.keys.retain(|k| k.t != t);
                    if m.keys.is_empty() {
                        layer_ref.motion = None;
                    }
                }
                if let Some(keys) = layer_ref.place_keys.as_mut() {
                    keys.retain(|k| k.t != t);
                    if keys.is_empty() {
                        layer_ref.place_keys = None;
                    }
                }
                let keys = layer_ref.motion.as_ref().map(|m| m.keys.len()).unwrap_or(0);
                return finish(&mut store, &mut media, dry_run, json!({"changed": [layer], "keys": keys}));
            }
            if list {
                let mut keys: Vec<serde_json::Value> = layer_ref
                    .motion
                    .as_ref()
                    .map(|m| {
                        m.keys
                            .iter()
                            .map(|k| json!({"kind": "camera", "t": k.t.to_string(), "zoom": k.zoom, "at": {"x": k.at.x, "y": k.at.y}}))
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(pk) = &layer_ref.place_keys {
                    keys.extend(pk.iter().map(|k| {
                        json!({"kind": "place", "t": k.t.to_string(), "place": {"x": k.x, "y": k.y}})
                    }));
                }
                ok(json!({"layer": layer, "keys": keys}), &[]);
                return Ok(());
            }
            let t = t
                .as_deref()
                .map(Time::parse)
                .transpose()?
                .context("usage: --t <time> is required to set a key")?;
            if zoom.is_none() && at.is_none() && place_point.is_none() {
                bail!("usage: give --zoom/--at for a camera key, or --place x,y to slide the layer");
            }
            if let Some(p) = place_point {
                let keys = layer_ref.place_keys.get_or_insert_with(Vec::new);
                keys.retain(|k| k.t != t);
                keys.push(crate::project::PlaceKey { t, x: p.x, y: p.y });
                keys.sort_by_key(|k| k.t.micros());
            }
            let camera_wanted = zoom.is_some() || at.is_some();
            let zoom = zoom.unwrap_or(1.0);
            if zoom < 1.0 {
                bail!("usage: --zoom must be at least 1.0 (1.0 shows the whole layer)");
            }
            if camera_wanted {
                let point = at_point.unwrap_or(crate::project::Point { x: 0.5, y: 0.5 });
                let m = layer_ref
                    .motion
                    .get_or_insert_with(|| crate::project::Motion { keys: Vec::new() });
                m.keys.retain(|k| k.t != t);
                m.keys.push(crate::project::Key { t, zoom, at: point });
                m.keys.sort_by_key(|k| k.t.micros());
            }
            let keys = layer_ref.motion.as_ref().map(|m| m.keys.len()).unwrap_or(0)
                + layer_ref.place_keys.as_ref().map(|k| k.len()).unwrap_or(0);
            finish(&mut store, &mut media, dry_run, json!({"changed": [layer], "keys": keys, "t": t.to_string()}))
        }
        Cmd::Config { action } => {
            use crate::config::{self, Provider, Secret};
            let mut cfg = config::load()?;
            let path = config::config_path()?;
            match action {
                ConfigCmd::Set { key, value, stdin, from_env } => {
                    let (provider, field) = split_config_key(&key)?;
                    let secret = field == "api_key";
                    if secret {
                        if value.is_some() {
                            bail!(
                                "usage: a key is never passed as an argument (it lands in ps and shell history); use --stdin or --from-env"
                            );
                        }
                        let raw = match (stdin, from_env.as_deref()) {
                            (true, None) => {
                                use std::io::Read;
                                let mut s = String::new();
                                std::io::stdin().read_to_string(&mut s)?;
                                s.trim().to_string()
                            }
                            (false, Some(var)) => std::env::var(var)
                                .with_context(|| format!("{var} is not set"))?,
                            (true, Some(_)) => bail!("usage: pass --stdin or --from-env, not both"),
                            (false, None) => bail!(
                                "usage: pass --stdin (pipe the key) or --from-env <VAR>"
                            ),
                        };
                        if raw.trim().is_empty() {
                            bail!("usage: the key is empty");
                        }
                        let fp = Secret::new(raw.clone()).fingerprint();
                        cfg.provider_mut(provider).api_key = Some(Secret::new(raw));
                        let written = config::save(&cfg)?;
                        ok(
                            json!({
                                "provider": provider.id(), "field": "api_key", "action": "set",
                                "fingerprint": fp, "path": written.display().to_string(),
                                "source": from_env.map(|v| format!("env:{v}")),
                            }),
                            &[],
                        );
                    } else {
                        let v = value.with_context(|| {
                            format!("usage: splice config set {key} <value>")
                        })?;
                        if field == "program" {
                            cfg.provider_mut(provider).program = Some(v.clone());
                        } else {
                            cfg.provider_mut(provider).model = Some(v.clone());
                        }
                        let written = config::save(&cfg)?;
                        ok(
                            json!({
                                "provider": provider.id(), "field": field, "action": "set",
                                "value": v, "path": written.display().to_string(),
                            }),
                            &[],
                        );
                    }
                }
                ConfigCmd::Get { key } => {
                    let (provider, field) = split_config_key(&key)?;
                    let r = config::Resolved::get(provider, &cfg, None, |v| std::env::var(v).ok());
                    if field == "api_key" {
                        ok(
                            json!({
                                "provider": provider.id(), "field": "api_key", "secret": true,
                                "set": r.key.is_some(),
                                "source": r.key_source.map(origin_name),
                                "fingerprint": r.key.as_ref().map(|k| k.fingerprint()),
                            }),
                            &[],
                        );
                    } else if field == "program" {
                        let pc = cfg.provider(provider);
                        ok(
                            json!({
                                "provider": provider.id(), "field": "program",
                                "value": pc.program,
                            }),
                            &[],
                        );
                    } else {
                        ok(
                            json!({
                                "provider": provider.id(), "field": "model",
                                "value": r.model, "source": origin_name(r.model_source),
                            }),
                            &[],
                        );
                    }
                }
                ConfigCmd::List => {
                    let mut providers = serde_json::Map::new();
                    for p in Provider::all() {
                        let r = config::Resolved::get(p, &cfg, None, |v| std::env::var(v).ok());
                        providers.insert(
                            p.id().into(),
                            json!({
                                "api_key": {
                                    "set": r.key.is_some(),
                                    "source": r.key_source.map(origin_name),
                                    "fingerprint": r.key.as_ref().map(|k| k.fingerprint()),
                                },
                                "model": {
                                    "value": r.model,
                                    "source": origin_name(r.model_source),
                                },
                            }),
                        );
                    }
                    let mut warnings = Vec::new();
                    if config::loose_perms(&path) {
                        warnings.push(
                            crate::report::Diagnostic::new(
                                "config_loose_perms",
                                "config",
                                format!("{} is readable by other users", path.display()),
                            )
                            .with_fix(format!("chmod 600 {}", path.display())),
                        );
                    }
                    ok(
                        json!({
                            "path": path.display().to_string(),
                            "exists": path.exists(),
                            "writable": path.parent().map(|d| is_writable(d)).unwrap_or(false),
                            "providers": providers,
                        }),
                        &warnings,
                    );
                }
                ConfigCmd::Path => {
                    ok(
                        json!({
                            "path": path.display().to_string(),
                            "exists": path.exists(),
                            "writable": path.parent().map(|d| is_writable(d)).unwrap_or(false),
                            "from": if std::env::var("SPLICE_CONFIG").is_ok() { "SPLICE_CONFIG" } else { "default" },
                        }),
                        &[],
                    );
                }
                ConfigCmd::Unset { key } => {
                    let (provider, field) = split_config_key(&key)?;
                    let pc = cfg.provider_mut(provider);
                    match field {
                        "api_key" => pc.api_key = None,
                        "model" => pc.model = None,
                        _ => pc.program = None,
                    }
                    config::save(&cfg)?;
                    ok(json!({"provider": provider.id(), "field": field, "action": "unset"}), &[]);
                }
            }
            Ok(())
        }
        Cmd::History { limit } => {
            let project = if path.is_absolute() {
                path.clone()
            } else {
                std::env::current_dir().unwrap_or_default().join(&path)
            };
            let entries: Vec<serde_json::Value> = crate::history::list(&project)?
                .into_iter()
                .take(limit)
                .map(|e| {
                    json!({
                        "seq": e.seq,
                        "at": e.at,
                        "age": e.age(),
                        "command": e.command,
                        "clips": e.clips(),
                        "overlays": e.overlays(),
                        "captions": e.has_captions(),
                        "index": e.has_index(),
                    })
                })
                .collect();
            ok(json!({ "entries": entries, "undoable": entries.len() }), &[]);
            Ok(())
        }
        Cmd::Undo { steps } => {
            let project = if path.is_absolute() {
                path.clone()
            } else {
                std::env::current_dir().unwrap_or_default().join(&path)
            };
            let mut restored = Vec::new();
            for _ in 0..steps.max(1) {
                match crate::history::undo(&project)? {
                    Some(e) => restored.push(json!({
                        "seq": e.seq,
                        "command": e.command,
                        "age": e.age(),
                        "clips": e.clips(),
                    })),
                    None => break,
                }
            }
            if restored.is_empty() {
                bail!("no_history: nothing to undo");
            }
            ok(json!({ "restored": restored }), &[]);
            Ok(())
        }
        Cmd::Doctor => {
            let caps = crate::ffmpeg::capabilities();
            let (ffmpeg_path, ffmpeg_source) = crate::ffmpeg::program("ffmpeg");
            let (ffprobe_path, _) = crate::ffmpeg::program("ffprobe");
            let (font, font_source) = match crate::ffmpeg::font_pick() {
                Some((origin, path)) => (json!(path), json!(origin)),
                None => (json!(null), json!(null)),
            };
            let ffprobe = std::process::Command::new(crate::ffmpeg::ffprobe())
                .arg("-version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            // Which key and model are actually in play, and where each came from.
            // A bare boolean could not answer "why is it using that key".
            let cfg = crate::config::load().unwrap_or_default();
            let cfg_path = crate::config::config_path().ok();
            let mut providers = serde_json::Map::new();
            for p in crate::config::Provider::all() {
                let r = crate::config::Resolved::get(p, &cfg, None, |v| std::env::var(v).ok());
                let mut entry = json!({
                    "key_set": r.key.is_some(),
                    "key_source": r.key_source.map(origin_name),
                    "fingerprint": r.key.as_ref().map(|k| k.fingerprint()),
                    "model": r.model,
                    "model_source": origin_name(r.model_source),
                });
                if p == crate::config::Provider::Whisper {
                    // whisper is a program plus a model file, so "set" is not
                    // "ready": report both, resolved.
                    let program = crate::asr::whisper_program(&cfg).ok();
                    entry["program"] = json!(program.as_ref().map(|p| p.display().to_string()));
                    entry["model_file"] = json!(std::path::Path::new(&r.model).is_file());
                    entry["ready"] = json!(program.is_some() && std::path::Path::new(&r.model).is_file());
                }
                providers.insert(p.id().into(), entry);
            }
            let default_provider = crate::asr::choose(None, &cfg, |v| std::env::var(v).ok())
                .map(|(p, _)| p.id().to_string())
                .ok();
            let config_block = json!({
                "path": cfg_path.as_ref().map(|p| p.display().to_string()),
                "exists": cfg_path.as_ref().map(|p| p.exists()).unwrap_or(false),
                "writable": cfg_path.as_ref().and_then(|p| p.parent()).map(|d| is_writable(d)),
            });
            // The cache is never authoritative: report what it holds, or null
            // when no project opens (a missing or broken document must not
            // fail doctor).
            let cache = Store::open(&path)
                .ok()
                .map(|store| {
                    let dir = store.cache_dir();
                    let (bytes, files) = crate::cache::usage(&dir);
                    json!({
                        "path": dir.display().to_string(),
                        "bytes": bytes,
                        "files": files,
                    })
                });

            let report = json!({
                "cache": cache,
                "ffmpeg": if caps.version.is_empty() { json!(null) } else { json!(caps.version) },
                "ffmpeg_path": ffmpeg_path.display().to_string(),
                "ffmpeg_source": match ffmpeg_source {
                    crate::ffmpeg::ProgramSource::Env => "SPLICE_FFMPEG",
                    crate::ffmpeg::ProgramSource::BesideBinary => "beside the binary",
                    crate::ffmpeg::ProgramSource::Cache => "splice cache",
                    crate::ffmpeg::ProgramSource::Path => "PATH",
                },
                "ffprobe": ffprobe,
                "ffprobe_path": ffprobe_path.display().to_string(),
                "encoders": {"libx264": caps.libx264},
                "filters": {"libass": caps.libass, "drawtext": caps.drawtext},
                "font": font,
                "font_source": font_source,
                "config": config_block,
                "default_provider": default_provider,
                "providers": providers,
            });

            // A doctor that says "ok" while ffmpeg is absent is worse than no
            // doctor: the failure surfaces later, at render time, as a mystery.
            if caps.version.is_empty() || !ffprobe {
                let what = if caps.version.is_empty() { "ffmpeg" } else { "ffprobe" };
                crate::report::fail_with(
                    "ffmpeg_missing",
                    format!("{what} is not on PATH"),
                    Some(install_ffmpeg_hint()),
                    json!({ "report": report }),
                );
            }
            if !caps.missing().is_empty() {
                crate::report::fail_with(
                    "ffmpeg_incapable",
                    format!("this ffmpeg lacks {}", caps.missing().join(", ")),
                    Some(install_ffmpeg_hint()),
                    json!({ "report": report }),
                );
            }

            let mut warnings = Vec::new();
            if crate::ffmpeg::resolve_font().is_none() {
                warnings.push(
                    crate::report::Diagnostic::new(
                        "no_font",
                        "font",
                        "no usable font found; text overlays and captions will fail",
                    )
                    .with_fix("set SPLICE_FONT=/path/to/font.ttf"),
                );
            }
            // A full cache only ever costs a re-render: warn, never block.
            if let Some(c) = &cache {
                if c.get("bytes").and_then(|b| b.as_u64()).unwrap_or(0) > crate::cache::WARN_BYTES {
                    warnings.push(
                        crate::report::Diagnostic::new(
                            "cache_large",
                            "cache",
                            format!(
                                "the cache at {} holds {} bytes; `.splice/` is never authoritative, so deleting it is safe and the next render rebuilds it",
                                c.get("path").and_then(|p| p.as_str()).unwrap_or(""),
                                c.get("bytes").and_then(|b| b.as_u64()).unwrap_or(0),
                            ),
                        )
                        .with_fix("splice cache prune"),
                    );
                }
            }
            ok(report, &warnings);
            Ok(())
        }
        Cmd::Update { check } => {
            let current = env!("CARGO_PKG_VERSION");
            let url = crate::update::registry_url();
            let document = crate::ffmpeg_install::fetch_bytes(&url).map_err(|e| {
                anyhow::anyhow!("update_network: {e}")
            })?;
            let latest = crate::update::latest_from_registry(&document).map_err(|e| {
                anyhow::anyhow!("update_network: {e}")
            })?;

            // Only move forward. A dev build newer than the registry must not be
            // silently replaced with an older published one.
            let newer = match (
                crate::update::Version::parse(&latest),
                crate::update::Version::parse(current),
            ) {
                (Some(l), Some(c)) => l > c,
                _ => latest != current,
            };

            if !newer {
                ok(
                    json!({
                        "current": current,
                        "latest": latest,
                        "updated": false,
                    }),
                    &[],
                );
                return Ok(());
            }
            if check {
                ok(
                    json!({
                        "check": true,
                        "current": current,
                        "latest": latest,
                        "updated": false,
                    }),
                    &[],
                );
                return Ok(());
            }
            crate::update::run_npm_install().map_err(|e| anyhow::anyhow!("{e}"))?;
            ok(
                json!({
                    "current": current,
                    "latest": latest,
                    "restart": "the new version runs from the next command",
                    "updated": true,
                }),
                &[],
            );
            Ok(())
        }
        Cmd::Cache { cmd } => match cmd {
            CacheCmd::Info => {
                let dir = Store::cache_dir_for(&path);
                let (bytes, files) = crate::cache::usage(&dir);
                ok(
                    json!({
                        "path": dir.display().to_string(),
                        "bytes": bytes,
                        "files": files,
                    }),
                    &[],
                );
                Ok(())
            }
            CacheCmd::Prune { all, dry_run } => {
                if all {
                    // `.splice/` is never authoritative: deleting every entry
                    // is always safe, and it needs no readable project.
                    // `read_dir` enumerates every directory entry including
                    // symlinks and dangling links; `remove_file` unlinks a
                    // link itself without following it, so nothing under the
                    // cache can escape.
                    let dir = Store::cache_dir_for(&path);
                    let mut removed: u64 = 0;
                    let mut freed_bytes: u64 = 0;
                    if let Ok(entries) = std::fs::read_dir(&dir) {
                        for entry in entries.flatten() {
                            if let Ok(m) = std::fs::metadata(&entry.path()) {
                                freed_bytes += m.len();
                            }
                            removed += 1;
                            if !dry_run {
                                let _ = std::fs::remove_file(entry.path());
                            }
                        }
                    }
                    let mut payload = json!({
                        "removed": removed,
                        "freed_bytes": freed_bytes,
                        "kept_files": 0,
                        "kept_bytes": 0,
                    });
                    if dry_run {
                        payload.as_object_mut().unwrap().insert("dry_run".into(), json!(true));
                    }
                    ok(payload, &[]);
                    Ok(())
                } else {
                    // Planning must succeed before anything is deleted: a
                    // broken document or missing media deletes nothing and
                    // fails with a project-class error pointing at --all.
                    //
                    // The keep set covers the plain Final and Preview plans
                    // and their default-loudnorm variants, since the cache key
                    // embeds the loudness clause. A `render --loudnorm` entry
                    // must survive pruning just like the plain one.
                    let store = match Store::open(&path) {
                        Ok(store) => store,
                        Err(e) => {
                            fail(
                                "project_invalid",
                                format!("{} is not a readable project: {e}", path.display()),
                                Some("splice cache prune --all".into()),
                            )
                        }
                    };
                    let mut media = MediaIndex::default();
                    let out = PathBuf::from("out.mp4");
                    let default_loudness = graph::Loudness::DEFAULT;
                    let plans = [
                        ("final", graph::plan(&store, &mut media, &out, graph::Profile::Final, None)),
                        ("preview", graph::plan(&store, &mut media, &out, graph::Profile::Preview, None)),
                        ("final+loudnorm", graph::plan(&store, &mut media, &out, graph::Profile::Final, Some(default_loudness))),
                        ("preview+loudnorm", graph::plan(&store, &mut media, &out, graph::Profile::Preview, Some(default_loudness))),
                    ];
                    let mut keep = HashSet::new();
                    for (name, p) in plans {
                        match p {
                            Ok(plan) => keep.extend(cache_keep_set(&[&plan])),
                            Err(e) => {
                                fail(
                                    "project_invalid",
                                    format!("planning {name} failed, nothing deleted: {e}"),
                                    Some("splice cache prune --all".into()),
                                )
                            }
                        }
                    }
                    let dir = store.cache_dir();
                    let mut removed: u64 = 0;
                    let mut freed_bytes: u64 = 0;
                    let mut kept_files: u64 = 0;
                    let mut kept_bytes: u64 = 0;
                    for entry in crate::cache::files(&dir) {
                        let stem = entry.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                        if keep.contains(stem) {
                            if let Ok(m) = std::fs::metadata(&entry) {
                                kept_files += 1;
                                kept_bytes += m.len();
                            }
                            continue;
                        }
                        if let Ok(m) = std::fs::metadata(&entry) {
                            freed_bytes += m.len();
                        }
                        removed += 1;
                        if !dry_run {
                            let _ = std::fs::remove_file(&entry);
                        }
                    }
                    let mut payload = json!({
                        "removed": removed,
                        "freed_bytes": freed_bytes,
                        "kept_files": kept_files,
                        "kept_bytes": kept_bytes,
                    });
                    if dry_run {
                        payload.as_object_mut().unwrap().insert("dry_run".into(), json!(true));
                    }
                    ok(payload, &[]);
                    Ok(())
                }
            }
        }
        Cmd::Ffmpeg { cmd } => match cmd {
            FfmpegCmd::Install => {
                let platform = crate::ffmpeg_install::platform().ok_or_else(|| {
                    anyhow::anyhow!(
                        "ffmpeg_install_unsupported: no pinned build for {} {}",
                        std::env::consts::OS,
                        std::env::consts::ARCH
                    )
                })?;
                let dir = crate::ffmpeg_install::cache_dir().ok_or_else(|| {
                    anyhow::anyhow!("ffmpeg_install_unsupported: no cache directory for this OS")
                })?;
                let (_, ffmpeg_pin, ffprobe_pin, ffmpeg_asset, ffprobe_asset) =
                    crate::ffmpeg_install::PINS
                        .iter()
                        .find(|(id, _, _, _, _)| *id == platform.id)
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "ffmpeg_install_unsupported: {} is not pinned",
                                platform.id
                            )
                        })?;
                let base = crate::ffmpeg_install::BASE_URL;
                let mut progress = |line: &str| eprintln!("{line}");
                let out = crate::ffmpeg_install::install(
                    &platform,
                    &dir,
                    &format!("{base}/{ffmpeg_asset}"),
                    &format!("{base}/{ffprobe_asset}"),
                    ffmpeg_pin,
                    ffprobe_pin,
                    &mut progress,
                )
                .map_err(|e| {
                    let text = format!("{e}");
                    let code = if text.contains("sha256 mismatch") {
                        "ffmpeg_install_hash"
                    } else if e.downcast_ref::<std::io::Error>().is_some() {
                        // ENOSPC, EACCES, a path that is a file: not the network.
                        "ffmpeg_install_io"
                    } else {
                        "ffmpeg_install_network"
                    };
                    anyhow::anyhow!("{code}: {text}")
                })?;
                ok(
                    json!({
                        "already_installed": out.already_installed,
                        "dir": out.dir.display().to_string(),
                        "ffmpeg_sha256": out.ffmpeg_sha256,
                        "ffprobe_sha256": out.ffprobe_sha256,
                        "installed": out.installed,
                        "platform": out.platform,
                    }),
                    &[],
                );
                Ok(())
            }
        }
    }
}

/// The cache entries the current plans still reference: every step key and
/// every sidecar stem, across every plan passed in. Delete by reference, never
/// by age. Pure: it does not touch the disk, so it can be unit-tested without
/// ffmpeg; planning (which does) is the caller's job, and a failed plan must
/// delete nothing.
fn cache_keep_set(plans: &[&graph::RenderPlan]) -> HashSet<String> {
    let mut keep = HashSet::new();
    for plan in plans {
        for step in &plan.steps {
            keep.insert(step.key.clone());
            if let Some(side) = &step.sidecar {
                if let Some(stem) = side.path.file_stem().and_then(|s| s.to_str()) {
                    keep.insert(stem.to_string());
                }
            }
        }
    }
    keep
}

/// Ends a mutation: duration, diagnostics, and a save unless this was a dry run.
///
/// A dry run must leave no trace — no document write, no history snapshot — so
/// the caller is responsible for skipping `history::record`, and this skips the
/// save. Everything else it reports is the real thing: the duration and
/// warnings the save would have produced.
fn finish(
    store: &mut Store,
    media: &mut MediaIndex,
    dry_run: bool,
    payload: serde_json::Value,
) -> Result<()> {
    finish_ext(store, media, dry_run, payload, &[])
}

/// `finish` plus warnings the command knows about that `diagnose` cannot see.
fn finish_ext(
    store: &mut Store,
    media: &mut MediaIndex,
    dry_run: bool,
    mut payload: serde_json::Value,
    extra: &[crate::report::Diagnostic],
) -> Result<()> {
    let duration = store.project.duration(media);
    let mut warnings = store.project.diagnose(media);
    warnings.extend(extra.iter().cloned());
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("duration".into(), json!(duration.to_string()));
        if dry_run {
            obj.insert("dry_run".into(), json!(true));
        }
    }
    if !dry_run {
        store.save()?;
    }
    ok(payload, &warnings);
    Ok(())
}

fn idx_source(store: &Store) -> PathBuf {
    let s = store
        .project
        .tracks
        .index
        .as_ref()
        .map(|i| i.source.clone())
        .unwrap_or_default();
    // The recorded source may be relative to the project; resolve it so the
    // comparison against a canonicalized clip path still works.
    store.resolve(&s)
}

fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    let ca = std::fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let cb = std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    ca == cb
}

/// Cuts, in source time, from `select=gt(scene,T)` + `showinfo`.
fn detect_cuts(source: &std::path::Path, threshold: f64) -> Result<Vec<Time>> {
    use std::process::Command;
    let filter = format!("select='gt(scene,{threshold})',showinfo");
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(source)
        .args(["-filter:v", &filter, "-f", "null", "-"])
        .output()
        .context("ffmpeg not found on PATH")?;
    let log = String::from_utf8_lossy(&out.stderr);
    let mut cuts = Vec::new();
    for line in log.lines() {
        if !line.contains("showinfo") {
            continue;
        }
        if let Some(pos) = line.find("pts_time:") {
            let rest = &line[pos + 9..];
            let value: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
                .collect();
            if let Ok(secs) = value.parse::<f64>() {
                if secs >= 0.0 {
                    cuts.push(Time::from_secs_f64(secs));
                }
            }
        }
    }
    Ok(cuts)
}

/// `x,y,size` in canvas fractions.
fn parse_place(raw: &str) -> Result<crate::project::Place> {
    let parts: Vec<&str> = raw.split(',').map(str::trim).collect();
    if parts.len() != 3 {
        bail!("usage: --place x,y,size, e.g. --place 0.72,0.72,0.25");
    }
    let x: f32 = parts[0].parse().context("--place x")?;
    let y: f32 = parts[1].parse().context("--place y")?;
    let size: f32 = parts[2].parse().context("--place size")?;
    if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) || size <= 0.0 || size > 1.0 {
        bail!("usage: --place takes fractions in 0..1, with a size above 0");
    }
    Ok(crate::project::Place { x, y, size })
}

/// `x,y` in canvas fractions.
fn parse_point(raw: &str) -> Result<crate::project::Point> {
    let parts: Vec<&str> = raw.split(',').map(str::trim).collect();
    if parts.len() != 2 {
        bail!("usage: --at x,y, e.g. --at 0.5,0.5");
    }
    let x: f32 = parts[0].parse().context("--at x")?;
    let y: f32 = parts[1].parse().context("--at y")?;
    if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
        bail!("usage: --at takes fractions in 0..1");
    }
    Ok(crate::project::Point { x, y })
}

fn split_config_key(raw: &str) -> Result<(crate::config::Provider, &'static str)> {
    let (p, field) = raw
        .split_once('.')
        .with_context(|| format!("usage: <provider>.<field>, e.g. gemini.api_key (got `{raw}`)"))?;
    let provider = crate::config::Provider::parse(p)
        .with_context(|| format!("usage: provider must be gemini, deepgram or whisper (got `{p}`)"))?;
    let field = match field {
        "api_key" => "api_key",
        "model" => "model",
        "program" => "program",
        other => bail!("usage: field must be api_key, model or program (got `{other}`)"),
    };
    Ok((provider, field))
}

fn origin_name(o: crate::config::Origin) -> &'static str {
    match o {
        crate::config::Origin::Flag => "flag",
        crate::config::Origin::Env => "env",
        crate::config::Origin::File => "file",
        crate::config::Origin::Default => "default",
    }
}

/// Answers "could a config be written here", by trying the one thing that
/// proves it. The previous version short-circuited on `!dir.exists()`, so a home
/// that does not exist and cannot be created reported itself writable.
fn is_writable(dir: &std::path::Path) -> bool {
    match std::fs::create_dir_all(dir) {
        Ok(()) => std::fs::metadata(dir)
            .map(|m| !m.permissions().readonly())
            .unwrap_or(false),
        Err(_) => false,
    }
}

fn install_ffmpeg_hint() -> String {
    let fallback = if cfg!(windows) {
        "winget install Gyan.FFmpeg"
    } else if cfg!(target_os = "macos") {
        "brew install ffmpeg"
    } else {
        "sudo apt install ffmpeg"
    };
    format!(
        "splice ffmpeg install   (fetches a verified static ffmpeg; or: {})",
        fallback
    )
}

fn loudness_arg(on: bool, lufs: f32, tp: f32, lra: f32) -> Option<graph::Loudness> {
    if !on {
        return None;
    }
    Some(graph::Loudness { i: lufs, tp, lra })
}

fn kind_label(k: &Option<crate::project::TransitionKind>) -> String {
    match k {
        Some(crate::project::TransitionKind::Crossfade) => "crossfade".into(),
        Some(crate::project::TransitionKind::DipToBlack) => "dip_to_black".into(),
        None => "cut".into(),
    }
}

/// The file the project's index actually lives in.
///
/// `index --from` writes `.splice/index/<audio hash>.json` and `transcribe`
/// writes `.splice/asr/<key>.json`; recomputing the path from the audio hash meant
/// `view` could not find an index `transcribe` had just written. The ref's own path
/// is the single answer, with the computed one as a fallback for older projects.
fn project_index_path(store: &Store) -> Result<std::path::PathBuf> {
    let r = store
        .project
        .tracks
        .index
        .as_ref()
        .context("no_index: run `splice transcribe` or `splice index --from <file>` first")?;
    let recorded = store.resolve(&r.transcript);
    if recorded.exists() {
        return Ok(recorded);
    }
    Ok(index::Index::path(
        &store.dir().join(".splice"),
        &r.audio_hash,
    ))
}

fn project_index(store: &Store) -> Result<index::Index> {
    index::Index::load(&project_index_path(store)?)
}

/// `4:12-4:58`, `00:00:04.500-00:00:10.000`.
fn parse_range(raw: &str) -> Result<(Time, Time)> {
    let (a, b) = raw
        .split_once('-')
        .context("usage: --range START-END, e.g. --range 4:12-4:58")?;
    let start = Time::parse(a)?;
    let end = Time::parse(b)?;
    if end <= start {
        bail!("usage: --range must end after it starts");
    }
    Ok((start, end))
}

fn parse_size(s: &str) -> Result<(u32, u32)> {
    let (w, h) = s
        .split_once(['x', 'X'])
        .context("usage: --size WxH, e.g. 1920x1080")?;
    Ok((w.trim().parse()?, h.trim().parse()?))
}

#[cfg(test)]
mod rm_tests {
    // The `rm` arm is exercised through the binary in the verification skill;
    // this pins the id kinds it must accept so a refactor cannot silently drop
    // one (it did once, and left a diagnostic pointing at a broken command).
    #[test]
    fn diagnostics_only_name_commands_that_work() {
        let src = include_str!("cli.rs");
        for needle in [
            "nothing called {id}",
            "id == \"captions\"",
            "id == \"audio\"",
            ".position(|o| o.id() == id)",
        ] {
            assert!(src.contains(needle), "rm lost a case: {needle}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_for_env_zero_is_silent() {
        let p = progress_for(false, false, Some("0"), false);
        assert!(!p.steps());
        assert!(!p.ticks());
    }

    #[test]
    fn progress_for_no_progress_wins_over_force() {
        let p = progress_for(true, true, Some("verbose"), true);
        assert!(!p.steps());
        assert!(!p.ticks());
    }

    #[test]
    fn progress_for_force_is_ticks() {
        let p = progress_for(true, false, None, false);
        assert!(p.steps());
        assert!(p.ticks());
    }

    #[test]
    fn progress_for_env_verbose_is_ticks() {
        let p = progress_for(false, false, Some("verbose"), false);
        assert!(p.steps());
        assert!(p.ticks());
    }

    #[test]
    fn progress_for_default_is_steps_without_ticks() {
        let p = progress_for(false, false, None, false);
        assert!(p.steps());
        assert!(!p.ticks());
    }

    #[test]
    fn progress_for_default_ticks_on_a_tty() {
        assert!(progress_for(false, false, None, true).ticks());
    }

    #[test]
    fn progress_for_no_progress_silences_even_on_a_tty() {
        assert!(!progress_for(false, true, None, true).steps());
        assert!(!progress_for(false, true, None, true).ticks());
    }

    #[test]
    fn cache_keep_set_is_every_step_key_and_sidecar_stem() {
        let sidecar = |key: &str| graph::Sidecar {
            path: PathBuf::from(format!("/cache/{key}.ass")),
            body: "x".into(),
        };
        let step = |key: &str, side: Option<graph::Sidecar>| graph::Step {
            label: "s".into(),
            key: key.into(),
            invocation: crate::ffmpeg::Invocation::new(),
            sidecar: side,
            output: PathBuf::new(),
        };
        let plan = |steps: Vec<graph::Step>| graph::RenderPlan {
            steps,
            out: PathBuf::new(),
            warnings: Vec::new(),
            duration: Time::ZERO,
        };

        let final_plan = plan(vec![step("k1", None), step("k2", Some(sidecar("k2")))]);
        let preview_plan = plan(vec![step("k3", None)]);
        let keep = cache_keep_set(&[&final_plan, &preview_plan]);
        assert!(keep.contains("k1"));
        assert!(keep.contains("k2"));
        assert!(keep.contains("k3"));
        assert!(!keep.contains("stale"));
        assert_eq!(keep.len(), 3);

        let empty = plan(Vec::new());
        assert!(cache_keep_set(&[&empty, &empty]).is_empty());
    }

    /// Pin: clap's `--loudnorm` defaults must match `Loudness::DEFAULT`, or
    /// `cache prune` stops keeping the loudnorm entry and the round-1 defect
    /// returns with every test green.
    #[test]
    fn loudnorm_defaults_match_loudness_default() {
        use clap::Parser;
        let cli = Cli::try_parse_from([
            "splice",
            "render",
            "--loudnorm",
        ])
        .expect("render --loudnorm must parse");
        if let Cmd::Render { loudnorm, lufs, true_peak, lra, .. } = cli.cmd {
            let l = loudness_arg(loudnorm, lufs, true_peak, lra)
                .expect("--loudnorm must yield a loudness");
            assert_eq!(l, graph::Loudness::DEFAULT);
        } else {
            panic!("expected Cmd::Render");
        }
    }

    /// Finding-5 trap: `layer set main --place ...` on a non-bottom layer at
    /// `z = 0` must not save a document the next command refuses. With a
    /// `z = -5` layer present, `main` is not the bottom so the bottom refusal
    /// does not fire, but `validate` still rejects a non-canvas place on `main`
    /// (exit 2, `project_invalid`) and splice.json is left byte-identical.
    #[test]
    fn layer_set_on_z_zero_refused_when_it_breaks_the_canvas_rule() {
        let dir = std::env::temp_dir().join(format!("splice-layer-trap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("splice.json");

        // main at z = 0 (canvas); under at z = -5 (the real bottom, non-canvas).
        let mut store = Store::create(&path, 640, 360, Rational::new(30, 1)).unwrap();
        store.project.tracks.layers.push(crate::project::Layer {
            id: "under".into(),
            z: -5,
            start: Time::ZERO,
            place: crate::project::Place { x: 0.2, y: 0.5, size: 0.5 },
            motion: None,
            place_keys: None,
            mask: None,
            clips: Vec::new(),
        });
        store.save().unwrap();

        let before = std::fs::read_to_string(&path).unwrap();

        let cli = Cli::try_parse_from([
            "splice",
            "--project",
            path.to_str().unwrap(),
            "layer",
            "set",
            "main",
            "--place",
            "0.2,0.5,0.5",
        ])
        .unwrap();
        let result = dispatch(cli);
        assert!(result.is_err(), "expected a refusal, got {:?}", result);
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(before, after, "splice.json must be unchanged by a refused edit");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A legitimate `layer set` on a layer above the bottom still applies.
    #[test]
    fn layer_set_on_upper_layer_applies_its_place() {
        let dir = std::env::temp_dir().join(format!("splice-layer-upper-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("splice.json");

        let mut store = Store::create(&path, 640, 360, Rational::new(30, 1)).unwrap();
        store.project.tracks.layers.push(crate::project::Layer {
            id: "pip".into(),
            z: 10,
            start: Time::ZERO,
            place: crate::project::Place::CANVAS,
            motion: None,
            place_keys: None,
            mask: None,
            clips: Vec::new(),
        });
        store.save().unwrap();

        let cli = Cli::try_parse_from([
            "splice",
            "--project",
            path.to_str().unwrap(),
            "layer",
            "set",
            "pip",
            "--place",
            "0.6,0.6,0.3",
        ])
        .unwrap();
        let result = dispatch(cli);
        assert!(result.is_ok(), "expected ok, got {:?}", result.err());

        let store = Store::open(&path).unwrap();
        let pip = store.project.layer("pip").expect("pip exists");
        assert!((pip.place.x - 0.6).abs() < f32::EPSILON);
        assert!((pip.place.y - 0.6).abs() < f32::EPSILON);
        assert!((pip.place.size - 0.3).abs() < f32::EPSILON);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
