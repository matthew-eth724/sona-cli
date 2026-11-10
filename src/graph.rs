//! Project -> RenderPlan. Pure: builds argv, never runs it.
//!
//! Two stages on purpose. Every clip normalizes to one uniform segment; then a
//! single assemble pass concatenates and decorates. One giant filter graph over
//! heterogeneous sources is where this class of tool breaks.
use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::cache;
use crate::project::Canvas;
use crate::ffmpeg::Invocation;
use crate::media::MediaIndex;
use crate::paths::{arg, filter_escape, text_escape};
use crate::project::{Anchor, Clip, Overlay, Project, Store, Transform};
use crate::report::Diagnostic;
use crate::time::Time;

/// What the render is for. A preview is a different plan, not a different
/// quality setting: its canvas and frame rate differ, so every step's key
/// differs and the two renders share no cache entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Final,
    Preview,
}

impl Profile {
    pub fn tag(self) -> &'static str {
        match self {
            Profile::Final => "final",
            Profile::Preview => "preview",
        }
    }

    /// Preview fits the canvas into 480x854 and drops to 15 fps. Dimensions are
    /// forced even because h264 with yuv420p cannot encode odd ones.
    pub fn canvas(self, canvas: &Canvas) -> (u32, u32) {
        match self {
            Profile::Final => (even(canvas.width), even(canvas.height)),
            Profile::Preview => {
                let (w, h) = (canvas.width as f64, canvas.height as f64);
                let scale = (480.0 / w).min(854.0 / h).min(1.0);
                (even((w * scale).round() as u32), even((h * scale).round() as u32))
            }
        }
    }

    pub fn fps(self, fps: crate::time::Rational) -> crate::time::Rational {
        match self {
            Profile::Final => fps,
            Profile::Preview => {
                if fps.as_f64() > 15.0 {
                    crate::time::Rational::new(15, 1)
                } else {
                    fps
                }
            }
        }
    }

    fn video_args(self, inv: &mut Invocation) {
        let (crf, preset) = match self {
            Profile::Final => ("18", "veryfast"),
            Profile::Preview => ("30", "ultrafast"),
        };
        inv.push("-c:v").push("libx264").push("-preset").push(preset);
        inv.push("-crf").push(crf).push("-pix_fmt").push("yuv420p");
    }

    fn audio_args(self, inv: &mut Invocation) {
        let bitrate = match self {
            Profile::Final => "192k",
            Profile::Preview => "64k",
        };
        inv.push("-c:a").push("aac").push("-b:a").push(bitrate).push("-ar").push("48000");
    }
}

fn even(v: u32) -> u32 {
    (v.max(2) / 2) * 2
}

#[derive(Clone, Debug)]
pub struct Sidecar {
    pub path: PathBuf,
    pub body: String,
}

#[derive(Clone, Debug)]
pub struct Step {
    pub label: String,
    pub key: String,
    pub invocation: Invocation,
    pub sidecar: Option<Sidecar>,
    pub output: PathBuf,
}

/// Where each clip sits on the timeline, so source time can be mapped to
/// timeline time. Transitions overlap, so the starts are cumulative minus the
/// transitions already consumed.
pub fn clip_offsets(clips: &[Clip], durations: &[Time]) -> Vec<Time> {
    let mut out = Vec::with_capacity(clips.len());
    let mut cursor = Time::ZERO;
    for (i, c) in clips.iter().enumerate() {
        out.push(cursor);
        let next = Time::from_micros(
            cursor.micros() + durations[i].micros()
                - clips
                    .get(i + 1)
                    .and_then(|n| n.transition.as_ref())
                    .map(|t| t.duration.micros())
                    .unwrap_or(0),
        );
        cursor = next.max(cursor);
    }
    out
}

/// Speech spans in *timeline* time, for ducking.
///
/// Words are merged into phrases with `gap`, padded by `pad`, and mapped through
/// the clips: a span that starts in clip 2 and ends in clip 3 is split, because
/// each clip can move the source time differently.
pub fn speech_spans(
    clips: &[Clip],
    durations: &[Time],
    words: &[(Time, Time)],
    gap: Time,
    pad: Time,
) -> Vec<(Time, Time)> {
    if clips.is_empty() || words.is_empty() {
        return Vec::new();
    }
    // 1. merge words into phrases in source time
    let mut phrases: Vec<(i64, i64)> = Vec::new();
    for (start, end) in words {
        let (s, e) = (start.micros(), end.micros());
        match phrases.last_mut() {
            Some(last) if s - last.1 <= gap.micros() => last.1 = last.1.max(e),
            _ => phrases.push((s, e)),
        }
    }

    // 2. map each phrase through whichever clip covers it
    let offsets = clip_offsets(clips, durations);
    let mut out: Vec<(Time, Time)> = Vec::new();
    for (ps, pe) in phrases {
        let ps = ps - pad.micros();
        let pe = pe + pad.micros();
        for (i, clip) in clips.iter().enumerate() {
            let src_in = clip.src_in.micros();
            let src_end = src_in + durations[i].micros().saturating_mul(1).max(0);
            // Clip i covers source time [src_in, src_in + span), where span is
            // the pre-speed length. Reconstruct it from the post-speed duration.
            let span = (durations[i].secs_f64() * clip.speed * 1e6) as i64;
            let (cs, ce) = (src_in, src_in + span);
            let start = ps.max(cs);
            let end = pe.min(ce);
            if end <= start {
                continue;
            }
            let tl = |t: i64| {
                Time::from_micros(offsets[i].micros() + ((t - cs) as f64 / clip.speed) as i64)
            };
            out.push((tl(start), tl(end)));
        }
    }

    // 3. merge overlapping spans in timeline time
    out.sort_by_key(|(s, _)| s.micros());
    let mut merged: Vec<(Time, Time)> = Vec::new();
    for (s, e) in out {
        match merged.last_mut() {
            Some(last) if s.micros() <= last.1.micros() => {
                if e.micros() > last.1.micros() {
                    last.1 = e;
                }
            }
            _ => merged.push((s, e)),
        }
    }
    merged
}

/// `volume='…':eval=frame` expression that drops by `db` inside each span and
/// ramps over `ramp`. Trapezoids are summed and clamped, so overlapping spans
/// never duck harder than `db`.
pub fn duck_expression(spans: &[(Time, Time)], db: f32, ramp: Time) -> Option<String> {
    if spans.is_empty() {
        return None;
    }
    let depth = 1.0 - 10f64.powf(db as f64 / 20.0);
    let r = (ramp.secs_f64()).max(0.001);
    let terms: Vec<String> = spans
        .iter()
        .map(|(s, e)| {
            format!(
                "max(0,min(1,min((t-{:.3})/{r:.3},({:.3}-t)/{r:.3})))",
                s.secs_f64(),
                e.secs_f64()
            )
        })
        .collect();
    Some(format!(
        "1-{depth:.4}*min(1,{})",
        terms.join("+")
    ))
}

/// EBU R128 loudness targeting, applied as a final pass over the assembled
/// program.
///
/// One-pass `loudnorm` is deliberately chosen: it keeps every step's argv
/// knowable at plan time, which is what `sona plan` promises. A two-pass
/// measurement would make the apply step's argv depend on a run, and the plan
/// would stop being a plan.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Loudness {
    pub i: f32,
    pub tp: f32,
    pub lra: f32,
}

impl Loudness {
    pub const DEFAULT: Loudness = Loudness {
        i: -14.0,
        tp: -1.0,
        lra: 11.0,
    };

    fn filter(&self) -> String {
        format!("loudnorm=I={}:TP={}:LRA={}", self.i, self.tp, self.lra)
    }

    fn key_parts(&self) -> String {
        format!("loudnorm|{}|{}|{}", self.i, self.tp, self.lra)
    }
}

/// One normalized clip on its way into the assemble stage.
#[derive(Clone, Debug)]
pub struct Segment {
    pub key: String,
    pub path: PathBuf,
    pub duration: Time,
    pub transition: Option<crate::project::Transition>,
}

#[derive(Clone, Debug)]
pub struct RenderPlan {
    pub steps: Vec<Step>,
    pub out: PathBuf,
    pub warnings: Vec<Diagnostic>,
    pub duration: Time,
}

impl RenderPlan {
    pub fn last_output(&self) -> &Path {
        &self.steps.last().expect("plan has steps").output
    }
}

pub fn plan(
    store: &Store,
    media: &mut MediaIndex,
    out: &Path,
    profile: Profile,
    loudness: Option<Loudness>,
) -> Result<RenderPlan> {
    let project = &store.project;
    if project.tracks.video.is_empty() {
        bail!("project has no clips (run `sona add <file>`)");
    }
    let mut warnings = Vec::new();
    let mut steps = Vec::new();
    let cache_dir = store.cache_dir();
    std::fs::create_dir_all(&cache_dir)?;
    let version = crate::ffmpeg::capabilities().version;

    // Stage 1: one normalized segment per clip.
    let mut segments: Vec<Segment> = Vec::new();
    for clip in &project.tracks.video {
        let src = store.resolve(&clip.source);
        let m = media.get(&src)?.clone();
        let span = clip.source_span(m.duration);
        if span.micros() <= 0 {
            warnings.push(
                Diagnostic::new(
                    "clip_empty",
                    format!("clip {}", clip.id),
                    format!("{} contributes no duration", clip.id),
                )
                .with_fix(format!("sona rm {}", clip.id)),
            );
            continue;
        }
        let mut inv = Invocation::new();
        if clip.src_in > Time::ZERO {
            inv.push("-ss").push(fmt_secs(clip.src_in));
        }
        inv.push("-t").push(fmt_secs(span));
        inv.input(&src);
        if !m.has_audio {
            inv.push("-f").push("lavfi").push("-i").push("anullsrc=r=48000:cl=stereo");
        }
        let vf = video_chain(clip, project, profile);
        inv.push("-vf").push(vf);
        inv.push("-af").push(audio_chain(clip, m.has_audio));
        inv.push("-map").push("0:v:0");
        if m.has_audio {
            inv.push("-map").push("0:a:0");
        } else {
            inv.push("-map").push("1:a:0");
        }
        inv.push("-c:v").push("libx264").push("-preset").push("veryfast");
        inv.push("-crf").push("16").push("-pix_fmt").push("yuv420p");
        inv.push("-c:a").push("aac").push("-b:a").push("192k").push("-ar").push("48000");
        inv.push("-shortest");
        inv.push("-movflags").push("+faststart");
        inv.push("-f").push("mp4");
        let key = cache::hash(&[
            inv.render(),
            cache::stat_token(&src),
            version.clone(),
            profile.tag().into(),
        ]);
        let output = cache::path(&cache_dir, &key, "mp4");
        inv.push(arg(&output));
        steps.push(Step {
            label: format!("normalize {}", clip.id),
            key: key.clone(),
            invocation: inv,
            sidecar: None,
            output: output.clone(),
        });
        segments.push(Segment {
            key: key.clone(),
            path: output,
            duration: clip.out_duration(m.duration),
            // The first segment has no predecessor, whatever the document says.
            transition: if segments.is_empty() { None } else { clip.transition.clone() },
        });
    }
    if steps.is_empty() {
        bail!("every clip is empty after trimming");
    }

    // Stage 2a: concatenate.
    let wants_transition = segments.iter().skip(1).any(|s| s.transition.is_some());
    let (mut base_key, mut base_path) = match segments.len() {
        1 => {
            let s = &steps[0];
            (s.key.clone(), s.output.clone())
        }
        _ if wants_transition => {
            // xfade consumes both inputs and runs them together, so this cannot
            // be the concat demuxer's `-c copy`: it is a real re-encode.
            let clause: Vec<String> = segments
                .iter()
                .map(|s| format!("{}|{}|{}", s.key, s.transition.as_ref().map(|t| t.kind.xfade()).unwrap_or("cut"), s.transition.as_ref().map(|t| t.duration.micros()).unwrap_or(0)))
                .collect();
            let key = cache::hash(&[
                clause.join(","),
                "xfade".into(),
                version.clone(),
                profile.tag().into(),
            ]);
            let output = cache::path(&cache_dir, &key, "mp4");
            let mut inv = Invocation::new();
            for seg in &segments {
                inv.input(&seg.path);
            }
            let graph = xfade_graph(&segments);
            inv.push("-filter_complex").push(graph);
            inv.push("-map").push("[v]");
            inv.push("-map").push("[a]");
            profile.video_args(&mut inv);
            profile.audio_args(&mut inv);
            inv.push("-movflags").push("+faststart");
            inv.push(arg(&output));
            steps.push(Step {
                label: format!("crossfade {} segments", segments.len()),
                key: key.clone(),
                invocation: inv,
                sidecar: None,
                output: output.clone(),
            });
            (key, output)
        }
        _ => {
            let keys: Vec<String> = steps.iter().map(|s| s.key.clone()).collect();
            let key = cache::hash(&[
                keys.join(","),
                "concat".into(),
                version.clone(),
                profile.tag().into(),
            ]);
            let list_path = cache::path(&cache_dir, &key, "txt");
            let output = cache::path(&cache_dir, &key, "mp4");
            let mut body = String::new();
            for seg in &segments {
                body.push_str(&concat_line(&seg.path));
                body.push('\n');
            }
            let mut inv = Invocation::new();
            inv.push("-f").push("concat").push("-safe").push("0");
            inv.input(&list_path);
            inv.push("-c").push("copy").push("-movflags").push("+faststart");
            inv.push(arg(&output));
            steps.push(Step {
                label: format!("concat {} segments", segments.len()),
                key: key.clone(),
                invocation: inv,
                sidecar: Some(Sidecar {
                    path: list_path,
                    body,
                }),
                output: output.clone(),
            });
            (key, output)
        }
    };

    // Stage 2b: captions, text, music. Only when there is something to burn.
    let has_text = project
        .tracks
        .overlays
        .iter()
        .any(|o| matches!(o, Overlay::Text(_)));
    let has_images = project.tracks.overlays.iter().any(|o| o.is_image());
    let has_captions = project
        .tracks
        .captions
        .as_ref()
        .map(|c| c.burn)
        .unwrap_or(false);
    let has_bed = project.tracks.audio.is_some();
    let mut extra_inputs: Vec<String> = Vec::new();

    if has_text || has_images || has_captions || has_bed {
        let mut inv = Invocation::new();
        inv.input(&base_path);
        let mut next_input = 1usize;
        let bed_input = if has_bed { 1 } else { 0 };
        if has_captions {
            let caps = project.tracks.captions.as_ref().unwrap();
            let srt = store.resolve(&caps.source);
            if !srt.exists() {
                bail!("captions file {} does not exist", srt.display());
            }
            // `subtitles` reads the file itself; adding it as `-i` would shift
            // the music bed to input 1 and break the audio graph.
            extra_inputs.push(cache::stat_token(&srt));
        }
        if has_bed {
            let bed = project.tracks.audio.as_ref().unwrap();
            let music = store.resolve(&bed.source);
            if !music.exists() {
                bail!("music bed {} does not exist", music.display());
            }
            inv.input(&music);
            extra_inputs.push(cache::stat_token(&music));
            next_input += 1;
        }
        // Images come after the bed, so the audio graph's indices stay stable.
        let mut image_inputs: Vec<(String, usize)> = Vec::new();
        for o in &project.tracks.overlays {
            let Overlay::Image(img) = o else { continue };
            let src = store.resolve(&img.source);
            if !src.exists() {
                bail!("overlay image {} does not exist", src.display());
            }
            inv.input(&src);
            extra_inputs.push(cache::stat_token(&src));
            image_inputs.push((img.id.clone(), next_input));
            next_input += 1;
        }

        let (canvas_w, _) = profile.canvas(&project.canvas);
        let graph = build_filter_graph(
            project,
            store,
            has_captions,
            has_bed,
            &image_inputs,
            canvas_w,
            bed_input,
        )?;
        inv.push("-filter_complex").push(graph);
        inv.push("-map").push("[v]");
        inv.push("-map").push("[a]");
        profile.video_args(&mut inv);
        profile.audio_args(&mut inv);
        inv.push("-movflags").push("+faststart");
        // The argv *is* the step: any change to a filter, an overlay, a style or
        // the canvas must move the key. Building it from inputs alone let a new
        // text overlay reuse the previous render.
        let mut key_parts = vec![
            base_key.clone(),
            "decorate".into(),
            version.clone(),
            profile.tag().into(),
            inv.render(),
        ];
        key_parts.extend(extra_inputs.iter().cloned());
        let key = cache::hash(&key_parts);
        let output = cache::path(&cache_dir, &key, "mp4");
        inv.push(arg(&output));
        steps.push(Step {
            label: "burn captions/text, mix audio".into(),
            key: key.clone(),
            invocation: inv,
            sidecar: None,
            output: output.clone(),
        });
        base_key = key;
        base_path = output;
    }

    // Final pass: loudness targeting. Video is copied, only audio is touched.
    if let Some(l) = loudness {
        let key = cache::hash(&[
            base_key.clone(),
            "loudnorm".into(),
            version.clone(),
            profile.tag().into(),
            l.key_parts(),
        ]);
        let output = cache::path(&cache_dir, &key, "mp4");
        let mut inv = Invocation::new();
        inv.input(&base_path);
        inv.push("-map").push("0:v:0").push("-map").push("0:a:0");
        inv.push("-c:v").push("copy");
        inv.push("-af").push(l.filter());
        inv.push("-c:a").push("aac");
        match profile {
            Profile::Final => inv.push("-b:a").push("192k"),
            Profile::Preview => inv.push("-b:a").push("64k"),
        };
        inv.push("-ar").push("48000");
        inv.push("-movflags").push("+faststart");
        inv.push(arg(&output));
        steps.push(Step {
            label: format!("loudness {} LUFS", l.i),
            key: key.clone(),
            invocation: inv,
            sidecar: None,
            output: output.clone(),
        });
        base_key = key;
        base_path = output;
    }

    let duration = project.duration(media);
    let _ = (&base_key, &base_path);
    Ok(RenderPlan {
        steps,
        out: out.to_path_buf(),
        warnings,
        duration,
    })
}

fn fmt_secs(t: Time) -> String {
    format!("{:.6}", t.secs_f64())
}

fn video_chain(clip: &Clip, project: &Project, profile: Profile) -> String {
    let (w, h) = profile.canvas(&project.canvas);
    let mut parts = vec![match clip.transform {
        Transform::Fit => format!(
            "scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2"
        ),
        Transform::Fill => format!(
            "scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}"
        ),
    }];
    parts.push("setsar=1".into());
    let fps = profile.fps(project.canvas.fps);
    parts.push(format!("fps={}/{}", fps.num, fps.den));
    if (clip.speed - 1.0).abs() > f64::EPSILON {
        parts.push(format!("setpts=PTS/{:.6}", clip.speed));
    }
    parts.join(",")
}

fn audio_chain(clip: &Clip, has_audio: bool) -> String {
    let mut parts = vec!["aresample=48000".to_string()];
    if has_audio {
        parts.push("aformat=sample_fmts=fltp:channel_layouts=stereo".into());
    }
    if (clip.speed - 1.0).abs() > f64::EPSILON {
        parts.extend(atempo_chain(clip.speed));
    }
    if clip.gain_db.abs() > 0.01 {
        parts.push(format!("volume={:.2}dB", clip.gain_db));
    }
    parts.join(",")
}

/// atempo only accepts 0.5..=2.0, so long speed changes chain.
pub fn atempo_chain(speed: f64) -> Vec<String> {
    let mut left = speed.max(0.01);
    let mut out = Vec::new();
    while left > 2.0 {
        out.push("atempo=2.0".to_string());
        left /= 2.0;
    }
    while left < 0.5 {
        out.push("atempo=0.5".to_string());
        left /= 0.5;
    }
    out.push(format!("atempo={left:.6}"));
    out
}

/// `image_inputs` maps overlay id → ffmpeg input index, in the order the plan
/// appended them.
fn build_filter_graph(
    project: &Project,
    store: &Store,
    has_captions: bool,
    has_bed: bool,
    image_inputs: &[(String, usize)],
    canvas_width: u32,
    bed_input: usize,
) -> Result<String> {
    let mut vf: Vec<String> = Vec::new();
    if has_captions {
        let caps = project.tracks.captions.as_ref().unwrap();
        let srt = store.resolve(&caps.source);
        let esc = filter_escape(&srt, &store.dir());
        vf.push(format!(
            "subtitles=filename={esc}:force_style='Fontsize=20,Outline=1.4,MarginV=28'"
        ));
    }
    for o in &project.tracks.overlays {
        let Overlay::Text(t) = o else { continue };
        let font = crate::ffmpeg::resolve_font().context("no_font: fc-match found no sans font")?;
        let (x, y) = match t.anchor {
            Anchor::Top => ("(w-text_w)/2", "48".to_string()),
            Anchor::TopLeft => ("48", "48".to_string()),
            Anchor::TopRight => ("w-text_w-48", "48".to_string()),
            Anchor::Center => ("(w-text_w)/2", "(h-th)/2".to_string()),
            Anchor::Bottom => ("(w-text_w)/2", "h-th-64".to_string()),
            Anchor::BottomLeft => ("48", "h-th-64".to_string()),
            Anchor::BottomRight => ("w-text_w-48", "h-th-64".to_string()),
        };
        vf.push(format!(
            "drawtext=fontfile={}:text='{}':fontsize={}:fontcolor={}:box=1:boxcolor=black@0.45:boxborderw=16:x={}:y={}:enable='between(t,{:.3},{:.3})'",
            filter_escape(Path::new(&font), &store.dir()),
            text_escape(&t.content),
            t.size,
            t.color,
            x,
            y,
            t.at.start.secs_f64(),
            t.at.end.secs_f64()
        ));
    }

    let mut graph = String::new();
    let mut last = if vf.is_empty() {
        // `subtitles` is first in the chain so overlay text is not restyled by ASS.
        graph.push_str("[0:v]null[v0]");
        "v0".to_string()
    } else {
        graph.push_str(&format!("[0:v]{}[v0]", vf.join(",")));
        "v0".to_string()
    };

    // Images are composited after the text, one node each, because `overlay`
    // takes exactly two inputs and cannot be chained inside one filter string.
    let mut n = 0usize;
    for (id, input) in image_inputs {
        let Some(Overlay::Image(img)) = project.tracks.overlays.iter().find(|o| o.id() == id)
        else {
            continue;
        };
        n += 1;
        let w = img.px_width(canvas_width);
        let m = img.px_margin(canvas_width);
        let (x, y) = img.xy_expr();
        let x = x.replace('m', &m.to_string());
        let y = y.replace('m', &m.to_string());
        graph.push_str(&format!(
            ";[{input}:v]format=rgba,scale={w}:-1,colorchannelmixer=aa={:.3}[img{n}]",
            img.opacity
        ));
        graph.push_str(&format!(
            ";[{last}][img{n}]overlay=x={x}:y={y}:enable='between(t,{:.3},{:.3})'[v{n}]",
            img.at.start.secs_f64(),
            img.at.end.secs_f64()
        ));
        last = format!("v{n}");
    }
    graph.push_str(&format!(";[{last}]null[v]"));

    if has_bed {
        let bed = project.tracks.audio.as_ref().unwrap();
        let mut chain: Vec<String> = Vec::new();
        if bed.gain_db.abs() > 0.01 {
            chain.push(format!("volume={:.2}dB", bed.gain_db));
        }
        // Ducking is index-driven: the bed drops where the words are.
        if let Some(duck) = &bed.duck {
            let spans: Vec<(Time, Time)> =
                duck.spans.iter().map(|r| (r.start, r.end)).collect();
            if let Some(expr) = duck_expression(&spans, duck.db, duck.ramp) {
                chain.push(format!("volume='{expr}':eval=frame"));
            }
        }
        if chain.is_empty() {
            graph.push_str(&format!(
                ";[0:a][{bed_input}:a]amix=inputs=2:duration=first:normalize=0[a]"
            ));
        } else {
            graph.push_str(&format!(
                ";[{bed_input}:a]{}[m];[0:a][m]amix=inputs=2:duration=first:normalize=0[a]",
                chain.join(",")
            ));
        }
    } else {
        graph.push_str(";[0:a]anull[a]");
    }
    Ok(graph)
}

/// One xfade per pair, chained; audio acrossfades in step.
///
/// The offset for pair i is everything before it minus every transition so far,
/// which is why the running total is kept rather than each clip's length.
fn xfade_graph(segments: &[Segment]) -> String {
    // Chains joined with ';' — a leading separator is an empty filter, which
    // ffmpeg rejects as "No such filter: ''".
    let mut chains: Vec<String> = Vec::new();
    let mut total: i64 = 0;
    let mut last_v = "0:v".to_string();
    let mut last_a = "0:a".to_string();
    for (i, seg) in segments.iter().enumerate().skip(1) {
        let prev = &segments[i - 1];
        total += prev.duration.micros();
        let (kind, dur) = match &seg.transition {
            Some(t) => (t.kind.xfade(), t.duration.micros().max(1)),
            None => ("fade", 1), // a cut is a 1us fade: inaudible, keeps one graph
        };
        let offset = ((total - dur) as f64 / 1e6).max(0.0);
        let d = dur as f64 / 1e6;
        chains.push(format!(
            "[{last_v}][{i}:v]xfade=transition={kind}:duration={d:.6}:offset={offset:.6}[x{i}]"
        ));
        chains.push(format!(
            "[{last_a}][{i}:a]acrossfade=d={d:.6}:c1=tri:c2=tri[a{i}]"
        ));
        last_v = format!("x{i}");
        last_a = format!("a{i}");
        total -= dur;
    }
    chains.push(format!("[{last_v}]null[v]"));
    chains.push(format!("[{last_a}]anull[a]"));
    chains.join(";")
}

fn concat_line(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/").replace('\'', "'\\''");
    format!("file '{s}'")
}

/// What `sona plan` prints. The agent-facing debugging lever.
pub fn plan_json(plan: &RenderPlan, profile: Profile) -> serde_json::Value {
    let steps: Vec<serde_json::Value> = plan
        .steps
        .iter()
        .map(|s| {
            serde_json::json!({
                "label": s.label,
                "key": s.key,
                "cached": s.output.exists(),
                "argv": s.invocation.args.iter().map(|a: &OsString| a.to_string_lossy().to_string()).collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::json!({
        "profile": profile.tag(),
        "steps": steps,
        "out": plan.out.display().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Canvas;
    use crate::time::Rational;

    fn canvas(w: u32, h: u32, fps: Rational) -> Canvas {
        Canvas { width: w, height: h, fps }
    }

    #[test]
    fn final_profile_keeps_the_canvas_but_makes_it_even() {
        let p = Profile::Final;
        assert_eq!(p.canvas(&canvas(1080, 1920, Rational::new(30, 1))), (1080, 1920));
        assert_eq!(p.canvas(&canvas(1081, 1919, Rational::new(30, 1))), (1080, 1918));
    }

    #[test]
    fn preview_fits_inside_480x854_and_keeps_aspect() {
        let p = Profile::Preview;
        assert_eq!(p.canvas(&canvas(1080, 1920, Rational::new(30, 1))), (480, 852));
        assert_eq!(p.canvas(&canvas(1920, 1080, Rational::new(30, 1))), (480, 270));
        assert_eq!(p.canvas(&canvas(1280, 720, Rational::new(30, 1))), (480, 270));
        // small sources are not upscaled
        assert_eq!(p.canvas(&canvas(576, 1024, Rational::new(30, 1))), (480, 852));
        assert_eq!(p.canvas(&canvas(320, 240, Rational::new(30, 1))), (320, 240));
    }

    #[test]
    fn preview_caps_fps_at_15() {
        let p = Profile::Preview;
        assert_eq!(p.fps(Rational::new(30, 1)).to_string(), "15");
        assert_eq!(p.fps(Rational::new(24, 1)).to_string(), "15");
        assert_eq!(p.fps(Rational::new(12, 1)).to_string(), "12");
        assert_eq!(Profile::Final.fps(Rational::new(30, 1)).to_string(), "30");
    }

    #[test]
    fn profiles_have_distinct_tags_so_keys_never_collide() {
        assert_ne!(Profile::Final.tag(), Profile::Preview.tag());
    }

    #[test]
    fn atempo_never_leaves_the_documented_range() {
        assert_eq!(atempo_chain(1.5), vec!["atempo=1.500000"]);
        assert_eq!(atempo_chain(4.0), vec!["atempo=2.0", "atempo=2.000000"]);
        assert_eq!(atempo_chain(0.25), vec!["atempo=0.5", "atempo=0.500000"]);
    }

    fn seg(secs: f64, tx: Option<(crate::project::TransitionKind, f64)>) -> Segment {
        Segment {
            key: format!("k{secs}"),
            path: PathBuf::from(format!("/tmp/{secs}.mp4")),
            duration: Time::from_secs_f64(secs),
            transition: tx.map(|(kind, d)| crate::project::Transition {
                kind,
                duration: Time::from_secs_f64(d),
            }),
        }
    }

    #[test]
    fn xfade_offsets_run_behind_the_cumulative_timeline() {
        use crate::project::TransitionKind;
        let segs = vec![
            seg(3.0, None),
            seg(3.0, Some((TransitionKind::Crossfade, 0.6))),
            seg(3.0, Some((TransitionKind::DipToBlack, 0.6))),
        ];
        let g = xfade_graph(&segs);
        assert!(g.contains("xfade=transition=fade:duration=0.600000:offset=2.400000"));
        assert!(g.contains("xfade=transition=fadeblack:duration=0.600000:offset=4.800000"));
        assert!(g.contains("acrossfade=d=0.600000"));
        // a hard cut still needs a chain, so it becomes a 1us fade
        assert!(!g.starts_with(';'), "leading ';' is an empty filter");
    }

    #[test]
    fn concat_lines_are_absolute_and_escaped() {
        let line = concat_line(Path::new("/tmp/a b/it's.mp4"));
        assert_eq!(line, "file '/tmp/a b/it'\\''s.mp4'");
    }
}

#[cfg(test)]
mod duck_tests {
    use super::*;
    use crate::project::{Clip, TransitionKind, Transform};

    fn clip(id: &str, src_in: f64, out: f64, speed: f64, tx: Option<f64>) -> Clip {
        Clip {
            id: id.into(),
            source: "/x.mp4".into(),
            src_in: Time::from_secs_f64(src_in),
            src_out: Some(Time::from_secs_f64(src_in + out * speed)),
            speed,
            transform: Transform::Fit,
            gain_db: 0.0,
            transition: tx.map(|d| crate::project::Transition {
                kind: TransitionKind::Crossfade,
                duration: Time::from_secs_f64(d),
            }),
        }
    }

    #[test]
    fn offsets_account_for_transition_overlap() {
        let clips = vec![clip("a", 0.0, 3.0, 1.0, None), clip("b", 0.0, 3.0, 1.0, Some(0.6))];
        let d = vec![Time::from_secs_f64(3.0), Time::from_secs_f64(3.0)];
        let o = clip_offsets(&clips, &d);
        assert_eq!(o[0].micros(), 0);
        assert_eq!(o[1].micros(), 2_400_000, "second clip starts 0.6s early");
    }

    #[test]
    fn source_time_maps_through_a_trimmed_clip() {
        // clip starts 10s into the source, so source 12s is 2s into the timeline
        let clips = vec![clip("a", 10.0, 5.0, 1.0, None)];
        let d = vec![Time::from_secs_f64(5.0)];
        let words = vec![(Time::from_secs_f64(12.0), Time::from_secs_f64(12.5))];
        let spans = speech_spans(&clips, &d, &words, Time::ZERO, Time::ZERO);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].0.micros(), 2_000_000);
        assert_eq!(spans[0].1.micros(), 2_500_000);
    }

    #[test]
    fn speed_changes_the_mapping() {
        // 2x speed: source 3.0s in is 1.5s on the timeline
        let clips = vec![clip("a", 0.0, 4.0, 2.0, None)];
        let d = vec![Time::from_secs_f64(2.0)];
        let words = vec![(Time::from_secs_f64(3.0), Time::from_secs_f64(3.5))];
        let spans = speech_spans(&clips, &d, &words, Time::ZERO, Time::ZERO);
        assert_eq!(spans[0].0.micros(), 1_500_000);
        assert_eq!(spans[0].1.micros(), 1_750_000);
    }

    #[test]
    fn a_phrase_across_a_transition_covers_both_clips() {
        // clip b overlaps clip a by 0.6s, so source 3-4s lands at timeline
        // 2.4-3.4 while the tail of a still runs to 3.0. The union is 2.0-3.4.
        let clips = vec![clip("a", 0.0, 3.0, 1.0, None), clip("b", 3.0, 3.0, 1.0, Some(0.6))];
        let d = vec![Time::from_secs_f64(3.0), Time::from_secs_f64(3.0)];
        let words = vec![(Time::from_secs_f64(2.0), Time::from_secs_f64(4.0))];
        let spans = speech_spans(&clips, &d, &words, Time::from_secs_f64(5.0), Time::ZERO);
        assert_eq!(spans.len(), 1, "{spans:?}");
        assert_eq!(spans[0].0.micros(), 2_000_000);
        assert_eq!(spans[0].1.micros(), 3_400_000);
    }

    #[test]
    fn expression_is_silent_outside_the_spans() {
        let spans = vec![(Time::from_secs_f64(2.0), Time::from_secs_f64(4.0))];
        let expr = duck_expression(&spans, -12.0, Time::from_micros(200_000)).unwrap();
        assert!(expr.starts_with("1-0.7488"), "{expr}");
        assert!(expr.contains("(t-2.000)/0.200"));
        assert!(duck_expression(&[], -12.0, Time::ZERO).is_none());
    }
}
