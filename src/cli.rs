//! Thin: parse, call the domain, print one JSON object.
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use serde_json::json;
use std::path::PathBuf;
use std::time::Duration;

use crate::frames;
use crate::graph;
use crate::media::MediaIndex;
use crate::project::{Anchor, AudioBed, Captions, Clip, Overlay, Store, TextOverlay, Transform};
use crate::render;
use crate::report::{fail_from, ok};
use crate::time::{Rational, Time, TimeRange};

#[derive(Parser)]
#[command(name = "sona", version, about = "video editing CLI for agents")]
pub struct Cli {
    #[arg(long, global = true, default_value = "sona.json")]
    project: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
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
    },
    /// Change a clip's playback speed (pitch preserved).
    Speed { id: String, speed: f64 },
    /// Move a clip to a position (0-based).
    Move { id: String, to: usize },
    /// Remove a clip.
    Rm { id: String },
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
    },
    /// Set a music bed played under the clips.
    Audio {
        source: PathBuf,
        #[arg(long, default_value = "0", allow_hyphen_values = true)]
        gain: f32,
    },
    /// Attach an SRT to burn in.
    Caption { source: PathBuf },
    /// Summary plus diagnostics for the current project.
    Inspect,
    /// Render review stills or a contact sheet.
    Frames {
        #[arg(long)]
        at: Option<String>,
        #[arg(long)]
        sheet: Option<u32>,
        #[arg(short = 'o', long, default_value = ".sona/preview")]
        out: PathBuf,
    },
    /// Print the ffmpeg commands a render would run.
    Plan,
    /// Render the project.
    Render {
        #[arg(short = 'o', long, default_value = "out.mp4")]
        out: PathBuf,
        #[arg(long, default_value = "3600")]
        timeout: u64,
    },
    /// Check ffmpeg, fonts and credentials.
    Doctor,
}

pub fn run() -> ! {
    let cli = Cli::parse();
    if let Err(e) = dispatch(cli) {
        fail_from(e);
    }
    std::process::exit(0);
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
        Cmd::Add { files, from, to, speed, at, fill } => {
            if files.is_empty() {
                bail!("usage: sona add <file>...");
            }
            let mut store = Store::open(&path)?;
            let mut media = MediaIndex::default();
            let from = from.map(|s| Time::parse(&s)).transpose()?;
            let to = to.map(|s| Time::parse(&s)).transpose()?;
            if let (Some(f), Some(t)) = (from, to) {
                if t <= f {
                    bail!("--to must be after --from");
                }
            }
            let insert_at = at.unwrap_or(store.project.tracks.video.len());
            if insert_at > store.project.tracks.video.len() {
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
                let id = store.project.next_id("clip");
                let clip = Clip {
                    id: id.clone(),
                    source: abs.to_string_lossy().to_string(),
                    src_in: from.unwrap_or(Time::ZERO),
                    src_out: to,
                    speed,
                    transform: if fill { Transform::Fill } else { Transform::Fit },
                    gain_db: 0.0,
                };
                let dur = clip.out_duration(m.duration);
                store.project.tracks.video.insert(insert_at + offset, clip);
                added.push(json!({
                    "id": id,
                    "source": abs.display().to_string(),
                    "duration": dur.to_string(),
                }));
            }
            store.save()?;
            let (duration, warnings) = after_edit(&store, &mut media);
            ok(json!({"added": added, "duration": duration}), &warnings);
            Ok(())
        }
        Cmd::List => {
            let store = Store::open(&path)?;
            let mut media = MediaIndex::default();
            let duration = store.project.duration(&mut media);
            let clips: Vec<_> = store
                .project
                .tracks
                .video
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
        Cmd::Trim { id, from, to } => {
            let mut store = Store::open(&path)?;
            let mut media = MediaIndex::default();
            let clip = store
                .project
                .tracks
                .video
                .iter_mut()
                .find(|c| c.id == id)
                .with_context(|| format!("usage: no clip {id}"))?;
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
            store.save()?;
            let (duration, warnings) = after_edit(&store, &mut media);
            ok(json!({"changed": [id], "invalidated": 1, "duration": duration}), &warnings);
            Ok(())
        }
        Cmd::Speed { id, speed } => {
            if speed <= 0.0 {
                bail!("usage: speed must be positive");
            }
            let mut store = Store::open(&path)?;
            let mut media = MediaIndex::default();
            let clip = store
                .project
                .tracks
                .video
                .iter_mut()
                .find(|c| c.id == id)
                .with_context(|| format!("usage: no clip {id}"))?;
            clip.speed = speed;
            store.save()?;
            let (duration, warnings) = after_edit(&store, &mut media);
            ok(json!({"changed": [id], "invalidated": 1, "duration": duration}), &warnings);
            Ok(())
        }
        Cmd::Move { id, to } => {
            let mut store = Store::open(&path)?;
            let mut media = MediaIndex::default();
            let pos = store
                .project
                .tracks
                .video
                .iter()
                .position(|c| c.id == id)
                .with_context(|| format!("usage: no clip {id}"))?;
            let clip = store.project.tracks.video.remove(pos);
            let target = to.min(store.project.tracks.video.len());
            store.project.tracks.video.insert(target, clip);
            store.save()?;
            let (duration, warnings) = after_edit(&store, &mut media);
            ok(json!({"changed": [id], "invalidated": 2, "duration": duration}), &warnings);
            Ok(())
        }
        Cmd::Rm { id } => {
            let mut store = Store::open(&path)?;
            let mut media = MediaIndex::default();
            let pos = store
                .project
                .tracks
                .video
                .iter()
                .position(|c| c.id == id)
                .with_context(|| format!("usage: no clip {id}"))?;
            store.project.tracks.video.remove(pos);
            store.save()?;
            let (duration, warnings) = after_edit(&store, &mut media);
            ok(json!({"changed": [id], "invalidated": 1, "duration": duration}), &warnings);
            Ok(())
        }
        Cmd::Text { content, from, to, pos, size, color } => {
            let mut store = Store::open(&path)?;
            let mut media = MediaIndex::default();
            let at = TimeRange::new(Time::parse(&from)?, Time::parse(&to)?)?;
            let anchor = match pos.as_str() {
                "top" => Anchor::Top,
                "center" => Anchor::Center,
                "bottom" => Anchor::Bottom,
                other => bail!("usage: --pos must be top|center|bottom, got {other}"),
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
            store.save()?;
            let (duration, warnings) = after_edit(&store, &mut media);
            ok(json!({"changed": [id], "invalidated": 1, "duration": duration}), &warnings);
            Ok(())
        }
        Cmd::Audio { source, gain } => {
            let mut store = Store::open(&path)?;
            let mut media = MediaIndex::default();
            let source = std::fs::canonicalize(&source)
                .with_context(|| format!("no such file: {}", source.display()))?;
            store.project.tracks.audio = Some(AudioBed {
                source: source.to_string_lossy().to_string(),
                gain_db: gain,
            });
            store.save()?;
            let (duration, warnings) = after_edit(&store, &mut media);
            ok(json!({"changed": ["audio"], "invalidated": 1, "duration": duration}), &warnings);
            Ok(())
        }
        Cmd::Caption { source } => {
            let mut store = Store::open(&path)?;
            let mut media = MediaIndex::default();
            let source = std::fs::canonicalize(&source)
                .with_context(|| format!("no such file: {}", source.display()))?;
            let cues = crate::caption::load(&source)
                .with_context(|| format!("reading {}", source.display()))?;
            store.project.tracks.captions = Some(Captions {
                source: source.to_string_lossy().to_string(),
                burn: true,
            });
            store.save()?;
            let (duration, warnings) = after_edit(&store, &mut media);
            ok(
                json!({"changed": ["captions"], "cues": cues.len(), "invalidated": 1, "duration": duration}),
                &warnings,
            );
            Ok(())
        }
        Cmd::Inspect => {
            let store = Store::open(&path)?;
            let mut media = MediaIndex::default();
            let duration = store.project.duration(&mut media);
            let diags = store.project.diagnose(&mut media);
            let clips = store
                .project
                .tracks
                .video
                .iter()
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
                }),
                &diags,
            );
            Ok(())
        }
        Cmd::Frames { at, sheet, out } => {
            let store = Store::open(&path)?;
            let mut media = MediaIndex::default();
            let plan = graph::plan(&store, &mut media, &PathBuf::from("out.mp4"))?;
            let source = plan.last_output().to_path_buf();
            if !source.exists() {
                bail!("nothing rendered yet (run `sona render`)");
            }
            let out_dir = store.resolve(&out.to_string_lossy());
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
            ok(json!({"frames": produced}), &[]);
            Ok(())
        }
        Cmd::Plan => {
            let store = Store::open(&path)?;
            let mut media = MediaIndex::default();
            let plan = graph::plan(&store, &mut media, &PathBuf::from("out.mp4"))?;
            ok(graph::plan_json(&plan), &plan.warnings);
            Ok(())
        }
        Cmd::Render { out, timeout } => {
            let store = Store::open(&path)?;
            let mut media = MediaIndex::default();
            let out_path = store.resolve(&out.to_string_lossy());
            let plan = graph::plan(&store, &mut media, &out_path)?;
            let summary = render::execute(&plan, Duration::from_secs(timeout))?;
            ok(
                json!({
                    "out": out_path.display().to_string(),
                    "duration": plan.duration.to_string(),
                    "steps": {"total": summary.total, "ran": summary.ran, "reused": summary.reused},
                }),
                &plan.warnings,
            );
            Ok(())
        }
        Cmd::Doctor => {
            let caps = crate::ffmpeg::capabilities();
            let font = crate::ffmpeg::resolve_font();
            let keys = json!({
                "gemini": std::env::var("GEMINI_API_KEY").is_ok(),
                "deepgram": std::env::var("DEEPGRAM_API_KEY").is_ok(),
            });
            let missing = caps.missing();
            ok(
                json!({
                    "ffmpeg": caps.version,
                    "encoders": {"libx264": caps.libx264},
                    "filters": {"libass": caps.libass, "drawtext": caps.drawtext},
                    "font": font,
                    "keys_in_env": keys,
                }),
                &[],
            );
            if !missing.is_empty() {
                eprintln!("doctor: ffmpeg is missing {}", missing.join(", "));
            }
            Ok(())
        }
    }
}

/// Mutations report the timeline *and* the diagnostics they just created: an
/// agent that trims past the end of its source should see that immediately,
/// not on the next `inspect`.
fn after_edit(store: &Store, media: &mut MediaIndex) -> (serde_json::Value, Vec<crate::report::Diagnostic>) {
    let duration = store.project.duration(media);
    let warnings = store.project.diagnose(media);
    (json!(duration.to_string()), warnings)
}

fn parse_size(s: &str) -> Result<(u32, u32)> {
    let (w, h) = s
        .split_once(['x', 'X'])
        .context("usage: --size WxH, e.g. 1920x1080")?;
    Ok((w.trim().parse()?, h.trim().parse()?))
}
