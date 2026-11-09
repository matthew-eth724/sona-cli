//! Project -> RenderPlan. Pure: builds argv, never runs it.
//!
//! Two stages on purpose. Every clip normalizes to one uniform segment; then a
//! single assemble pass concatenates and decorates. One giant filter graph over
//! heterogeneous sources is where this class of tool breaks.
use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::cache;
use crate::ffmpeg::Invocation;
use crate::media::MediaIndex;
use crate::paths::{arg, filter_escape, text_escape};
use crate::project::{Anchor, Clip, Overlay, Project, Store, Transform};
use crate::report::Diagnostic;
use crate::time::Time;

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

pub fn plan(store: &Store, media: &mut MediaIndex, out: &Path) -> Result<RenderPlan> {
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
    let mut segments: Vec<(String, PathBuf)> = Vec::new();
    for clip in &project.tracks.video {
        let src = store.resolve(&clip.source);
        let m = media.get(&src)?.clone();
        let span = clip.source_span(m.duration);
        if span.micros() <= 0 {
            warnings.push(Diagnostic::new(
                "clip_empty",
                format!("clip {}", clip.id),
                format!("{} contributes no duration", clip.id),
            ));
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
        let vf = video_chain(clip, project);
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
        let key = cache::hash(&[inv.render(), cache::stat_token(&src), version.clone()]);
        let output = cache::path(&cache_dir, &key, "mp4");
        inv.push(arg(&output));
        steps.push(Step {
            label: format!("normalize {}", clip.id),
            key,
            invocation: inv,
            sidecar: None,
            output: output.clone(),
        });
        segments.push((clip.id.clone(), output));
    }
    if steps.is_empty() {
        bail!("every clip is empty after trimming");
    }

    // Stage 2a: concatenate.
    let (mut base_key, mut base_path) = match segments.len() {
        1 => {
            let s = &steps[0];
            (s.key.clone(), s.output.clone())
        }
        _ => {
            let keys: Vec<String> = steps.iter().map(|s| s.key.clone()).collect();
            let key = cache::hash(&[keys.join(","), "concat".into(), version.clone()]);
            let list_path = cache::path(&cache_dir, &key, "txt");
            let output = cache::path(&cache_dir, &key, "mp4");
            let mut body = String::new();
            for (_, seg) in &segments {
                body.push_str(&concat_line(seg));
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
    let has_captions = project
        .tracks
        .captions
        .as_ref()
        .map(|c| c.burn)
        .unwrap_or(false);
    let has_bed = project.tracks.audio.is_some();
    let mut extra_inputs: Vec<String> = Vec::new();

    if has_text || has_captions || has_bed {
        let mut inv = Invocation::new();
        inv.input(&base_path);
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
            inv.input(&music);
            extra_inputs.push(cache::stat_token(&music));
        }

        let graph = build_filter_graph(project, store, has_captions, has_bed)?;
        inv.push("-filter_complex").push(graph);
        inv.push("-map").push("[v]");
        inv.push("-map").push("[a]");
        inv.push("-c:v").push("libx264").push("-preset").push("veryfast");
        inv.push("-crf").push("18").push("-pix_fmt").push("yuv420p");
        inv.push("-c:a").push("aac").push("-b:a").push("192k");
        inv.push("-movflags").push("+faststart");
        // The argv *is* the step: any change to a filter, an overlay, a style or
        // the canvas must move the key. Building it from inputs alone let a new
        // text overlay reuse the previous render.
        let mut key_parts = vec![base_key.clone(), "decorate".into(), version.clone(), inv.render()];
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

fn video_chain(clip: &Clip, project: &Project) -> String {
    let (w, h) = (project.canvas.width, project.canvas.height);
    let mut parts = vec![match clip.transform {
        Transform::Fit => format!(
            "scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2"
        ),
        Transform::Fill => format!(
            "scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}"
        ),
    }];
    parts.push("setsar=1".into());
    parts.push(format!(
        "fps={}/{}",
        project.canvas.fps.num, project.canvas.fps.den
    ));
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

fn build_filter_graph(
    project: &Project,
    store: &Store,
    has_captions: bool,
    has_bed: bool,
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
        let Overlay::Text(t) = o;
        let font = crate::ffmpeg::resolve_font().context("no_font: fc-match found no sans font")?;
        let y = match t.anchor {
            Anchor::Top => "48".to_string(),
            Anchor::Center => "(h-th)/2".to_string(),
            Anchor::Bottom => "h-th-64".to_string(),
        };
        vf.push(format!(
            "drawtext=fontfile={}:text='{}':fontsize={}:fontcolor={}:box=1:boxcolor=black@0.45:boxborderw=16:x=(w-text_w)/2:y={}:enable='between(t,{:.3},{:.3})'",
            filter_escape(Path::new(&font), &store.dir()),
            text_escape(&t.content),
            t.size,
            t.color,
            y,
            t.at.start.secs_f64(),
            t.at.end.secs_f64()
        ));
    }

    let mut graph = String::new();
    if vf.is_empty() {
        graph.push_str("[0:v]null[v]");
    } else {
        // `subtitles` is first in the chain so overlay text is not restyled by ASS.
        graph.push_str(&format!("[0:v]{}[v]", vf.join(",")));
    }

    if has_bed {
        let bed = project.tracks.audio.as_ref().unwrap();
        if bed.gain_db.abs() > 0.01 {
            graph.push_str(&format!(
                ";[1:a]volume={:.2}dB[m];[0:a][m]amix=inputs=2:duration=first:normalize=0[a]",
                bed.gain_db
            ));
        } else {
            graph.push_str(";[0:a][1:a]amix=inputs=2:duration=first:normalize=0[a]");
        }
    } else {
        graph.push_str(";[0:a]anull[a]");
    }
    Ok(graph)
}

fn concat_line(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/").replace('\'', "'\\''");
    format!("file '{s}'")
}

/// What `sona plan` prints. The agent-facing debugging lever.
pub fn plan_json(plan: &RenderPlan) -> serde_json::Value {
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
    serde_json::json!({ "steps": steps, "out": plan.out.display().to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atempo_never_leaves_the_documented_range() {
        assert_eq!(atempo_chain(1.5), vec!["atempo=1.500000"]);
        assert_eq!(atempo_chain(4.0), vec!["atempo=2.0", "atempo=2.000000"]);
        assert_eq!(atempo_chain(0.25), vec!["atempo=0.5", "atempo=0.500000"]);
    }

    #[test]
    fn concat_lines_are_absolute_and_escaped() {
        let line = concat_line(Path::new("/tmp/a b/it's.mp4"));
        assert_eq!(line, "file '/tmp/a b/it'\\''s.mp4'");
    }
}
