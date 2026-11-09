//! SRT in, cues out. Burn-in is a filter, not our problem to render.
use anyhow::{Result, bail};
use std::path::Path;

use crate::time::Time;

#[derive(Clone, Debug)]
pub struct Cue {
    pub start: Time,
    pub end: Time,
    pub text: String,
}

pub fn parse_srt(body: &str) -> Result<Vec<Cue>> {
    let mut cues = Vec::new();
    let normalized = body.replace("\r\n", "\n");
    for block in normalized.split("\n\n") {
        let lines: Vec<&str> = block.lines().collect();
        let Some(idx) = lines.iter().position(|l| l.contains("-->")) else {
            continue;
        };
        let (start_raw, end_raw) = lines[idx]
            .split_once("-->")
            .map(|(a, b)| (a.trim(), b.trim()))
            .unwrap();
        let start = srt_time(start_raw)?;
        let end = srt_time(end_raw)?;
        let text = lines[idx + 1..].join("\n").trim().to_string();
        if text.is_empty() {
            continue;
        }
        cues.push(Cue { start, end, text });
    }
    Ok(cues)
}

fn srt_time(raw: &str) -> Result<Time> {
    // 00:00:04,500  ->  00:00:04.500
    let cleaned = raw.trim().replace(',', ".");
    if !cleaned.contains(':') {
        bail!("bad srt timestamp `{raw}`");
    }
    Time::parse(&cleaned)
}

pub fn load(path: &Path) -> Result<Vec<Cue>> {
    let body = std::fs::read_to_string(path)?;
    parse_srt(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commas_and_multiline_text() {
        let srt = "1\n00:00:00,700 --> 00:00:01,200\nJane?\n\n2\n00:00:01,700 --> 00:00:02,500\nMark? Mark?\n";
        let cues = parse_srt(srt).unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start.micros(), 700_000);
        assert_eq!(cues[1].end.micros(), 2_500_000);
        assert_eq!(cues[1].text, "Mark? Mark?");
    }
}
