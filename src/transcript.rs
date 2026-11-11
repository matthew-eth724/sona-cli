//! Transcript files in, one word list out. Every provider has its own wire
//! shape; nothing downstream should have to know which one it came from.
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::path::Path;

use crate::caption;
use crate::index::Word;
use crate::time::Time;

#[derive(Debug, Clone)]
pub struct Parsed {
    pub words: Vec<Word>,
    pub provider: String,
    pub language: Option<String>,
    /// True when timings were interpolated (SRT) rather than reported by an ASR
    /// backend. Worth knowing before cutting on a word boundary.
    pub approximate: bool,
}

/// Parses an already-fetched provider response. `parse_file` is the same thing
/// plus the file handling, so a live API reply and a saved fixture take one path.
pub fn parse_value(v: &Value) -> Result<Parsed> {
    if v.get("steps").is_some() {
        from_gemini(v)
    } else if v.get("results").is_some() {
        from_deepgram(v)
    } else if v.get("status").and_then(|s| s.as_str()) == Some("completed") {
        // A completed Gemini interaction with no `steps` is audio with no
        // speech: measured on five seconds of a sine tone, the whole body is
        // 558 bytes with `total_output_tokens: 0`. That is an answer, not a
        // parse failure.
        Ok(Parsed {
            words: Vec::new(),
            provider: "gemini".into(),
            language: None,
            approximate: false,
        })
    } else {
        bail!(
            "unrecognised transcript: expected a Gemini `steps` or Deepgram `results` response"
        )
    }
}

pub fn parse_file(path: &Path) -> Result<Parsed> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "srt" | "vtt" => from_srt(path),
        "json" => {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            let v: Value = serde_json::from_str(&raw)
                .with_context(|| format!("{} is not valid JSON", path.display()))?;
            parse_value(&v).with_context(|| format!("reading {}", path.display()))
        }
        other => bail!("unsupported transcript type `{other}`: pass .json, .srt or .vtt"),
    }
}

fn word(text: String, start: Time, end: Time, speaker: Option<u32>, confidence: Option<f32>) -> Word {
    Word {
        id: String::new(),
        text,
        start,
        end,
        speaker,
        confidence,
    }
}

/// Gemini 3.5 Transcribe: words are annotations with string offsets like
/// `"3s"` or `"0.700s"` — the fractional part is optional.
fn from_gemini(v: &Value) -> Result<Parsed> {
    let mut words = Vec::new();
    let steps = v
        .get("steps")
        .and_then(|s| s.as_array())
        .context("gemini: `steps` is not an array")?;
    for step in steps {
        let Some(content) = step.get("content").and_then(|c| c.as_array()) else {
            continue;
        };
        for part in content {
            let Some(anns) = part.get("annotations").and_then(|a| a.as_array()) else {
                continue;
            };
            for a in anns {
                if a.get("type").and_then(|t| t.as_str()) != Some("word_info") {
                    continue;
                }
                let text = a.get("text").and_then(|t| t.as_str()).unwrap_or("").trim();
                if text.is_empty() {
                    continue;
                }
                let start = Time::parse(a.get("start_offset").and_then(|t| t.as_str()).unwrap_or("0s"))?;
                let end = Time::parse(a.get("end_offset").and_then(|t| t.as_str()).unwrap_or("0s"))?;
                let speaker = a
                    .get("speaker")
                    .and_then(|s| s.as_str())
                    .and_then(|s| s.strip_prefix("spk:"))
                    .and_then(|s| s.parse::<u32>().ok());
                words.push(word(text.to_string(), start, end, speaker, None));
            }
        }
    }
    // No annotations inside a present `steps` array is also "no speech".
    Ok(Parsed {
        words,
        provider: "gemini".into(),
        language: None,
        approximate: false,
    })
}

/// Deepgram nova-3: floats, integer speakers, `punctuated_word` when present.
fn from_deepgram(v: &Value) -> Result<Parsed> {
    let alts = v
        .get("results")
        .and_then(|r| r.get("channels"))
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("alternatives"))
        .and_then(|a| a.get(0))
        .context("deepgram: no results.channels[0].alternatives[0]")?;
    let language = v
        .get("results")
        .and_then(|r| r.get("channels"))
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("detected_language"))
        .and_then(|l| l.as_str())
        .map(str::to_string);
    let mut words = Vec::new();
    for w in alts.get("words").and_then(|w| w.as_array()).into_iter().flatten() {
        let text = w
            .get("punctuated_word")
            .or_else(|| w.get("word"))
            .and_then(|t| t.as_str())
            .unwrap_or("");
        if text.is_empty() {
            continue;
        }
        let start = Time::from_secs_f64(w.get("start").and_then(|s| s.as_f64()).unwrap_or(0.0));
        let end = Time::from_secs_f64(w.get("end").and_then(|s| s.as_f64()).unwrap_or(0.0));
        let speaker = w.get("speaker").and_then(|s| s.as_u64()).map(|s| s as u32);
        let confidence = w.get("confidence").and_then(|c| c.as_f64()).map(|c| c as f32);
        words.push(word(text.to_string(), start, end, speaker, confidence));
    }
    Ok(Parsed {
        words,
        provider: "deepgram".into(),
        language,
        approximate: false,
    })
}

/// Subtitles: cues, not words. Word times inside a cue are interpolated by
/// character share, so they are marked approximate.
fn from_srt(path: &Path) -> Result<Parsed> {
    let cues = caption::load(path)?;
    if cues.is_empty() {
        bail!("{} has no cues", path.display());
    }
    let mut words = Vec::new();
    for cue in &cues {
        let parts: Vec<&str> = cue.text.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }
        let total_chars: usize = parts.iter().map(|p| p.len()).sum();
        let span = (cue.end.micros() - cue.start.micros()).max(1);
        let mut acc = 0usize;
        for p in &parts {
            let start = cue.start.micros() + (span * acc as i64) / total_chars as i64;
            acc += p.len();
            let end = cue.start.micros() + (span * acc as i64) / total_chars as i64;
            words.push(word(
                p.to_string(),
                Time::from_micros(start),
                Time::from_micros(end),
                None,
                None,
            ));
        }
    }
    Ok(Parsed {
        words,
        provider: "srt".into(),
        language: None,
        approximate: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/asr")
            .join(name)
    }

    #[test]
    fn gemini_offsets_parse_both_shapes() {
        let p = parse_file(&fixture("gemini-verbatim.json")).unwrap();
        assert_eq!(p.words.len(), 78);
        assert_eq!(p.words[0].text, "Jane?");
        assert_eq!(p.words[0].start.micros(), 700_000);
        // `"3s"` — the integer form that a strict parser drops.
        let hi = p.words.iter().find(|w| w.text == "Hi.").unwrap();
        assert_eq!(hi.start.micros(), 3_000_000);
        assert!(!p.approximate);
    }

    #[test]
    fn gemini_diarization_becomes_a_number() {
        let p = parse_file(&fixture("gemini-diarized.json")).unwrap();
        assert_eq!(p.words[0].speaker, Some(0));
        assert_eq!(p.words[1].speaker, Some(1));
    }

    #[test]
    fn a_completed_interaction_with_no_steps_means_no_speech() {
        let v: Value = serde_json::from_str(
            r#"{"status":"completed","usage":{"total_output_tokens":0}}"#,
        )
        .unwrap();
        let p = parse_value(&v).unwrap();
        assert!(p.words.is_empty());
        assert_eq!(p.provider, "gemini");
    }

    #[test]
    fn something_unrelated_is_still_an_error() {
        let v: Value = serde_json::from_str(r#"{"error":{"code":400}}"#).unwrap();
        assert!(parse_value(&v).is_err());
    }

    #[test]
    fn deepgram_floats_round() {
        let p = parse_file(&fixture("deepgram-nova3-diarized.json")).unwrap();
        assert_eq!(p.words.len(), 77);
        assert_eq!(p.words[0].start.micros(), 720_000);
        assert_eq!(p.words[0].text, "Jane?");
        assert!(p.words[0].confidence.is_some());
    }
}

#[cfg(test)]
mod scene_tests {
    use super::super::index::merge_scenes;
    use crate::time::Time;

    fn t(x: f64) -> Time {
        Time::from_secs_f64(x)
    }

    #[test]
    fn hard_cuts_become_scenes_with_keyframes() {
        let scenes = merge_scenes(vec![t(2.0), t(4.0)], t(6.0), t(0.5));
        assert_eq!(scenes.len(), 3);
        assert_eq!(scenes[0].start, t(0.0));
        assert_eq!(scenes[0].end, t(2.0));
        assert_eq!(scenes[2].end, t(6.0));
        // keyframe sits inside the shot, not on its first frame
        assert!(scenes[0].keyframe > scenes[0].start);
        assert!(scenes[0].keyframe < scenes[0].end);
    }

    #[test]
    fn shots_under_the_floor_are_merged_away() {
        // a 0.2s flash between two shots
        let scenes = merge_scenes(vec![t(2.0), t(2.2)], t(6.0), t(0.5));
        assert_eq!(scenes.len(), 2, "the flash disappears: {:?}", scenes.len());
        assert_eq!(scenes[1].start, t(2.0));
    }

    #[test]
    fn cuts_outside_the_file_are_ignored() {
        let scenes = merge_scenes(vec![t(0.0), t(99.0)], t(6.0), t(0.5));
        assert_eq!(scenes.len(), 1);
        assert_eq!(scenes[0].end, t(6.0));
    }

    #[test]
    fn no_cuts_is_one_scene() {
        let scenes = merge_scenes(vec![], t(6.0), t(0.5));
        assert_eq!(scenes.len(), 1);
        assert_eq!(scenes[0].keyframe, t(0.3));
    }
}
