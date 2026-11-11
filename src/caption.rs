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

/// Renders cues as SRT, wrapping each to at most two lines of `MAX_CHARS`.
/// Anything that still does not fit is split into a second cue at the word
/// boundary closest to halfway, with the time apportioned by characters.
pub fn to_srt(cues: &[(Time, Time, String)]) -> String {
    const MAX_CHARS: usize = 42;
    let mut out = Vec::new();
    for (i, (start, end, text)) in split_for_two_lines(cues, MAX_CHARS).iter().enumerate() {
        out.push(format!(
            "{}\n{} --> {}\n{}\n",
            i + 1,
            srt_stamp(*start),
            srt_stamp(*end),
            text
        ));
    }
    out.join("\n")
}

fn split_for_two_lines(
    cues: &[(Time, Time, String)],
    max: usize,
) -> Vec<(Time, Time, String)> {
    let mut out = Vec::new();
    for (start, end, text) in cues {
        let words: Vec<&str> = text.split_whitespace().collect();
        if wrap_line(text, max).len() <= 2 {
            out.push((*start, *end, wrap_line(text, max).join("\n")));
            continue;
        }
        let total: usize = words.iter().map(|w| w.len() + 1).sum();
        let mut acc = 0;
        let mut cut = words.len() / 2;
        for (i, w) in words.iter().enumerate() {
            acc += w.len() + 1;
            if acc * 2 >= total {
                cut = (i + 1).clamp(1, words.len() - 1);
                break;
            }
        }
        let frac = acc as f64 / total as f64;
        let span = end.micros() - start.micros();
        let mid = Time::from_micros(start.micros() + (span as f64 * frac) as i64);
        let head = wrap_line(&words[..cut].join(" "), max).join("\n");
        let tail = wrap_line(&words[cut..].join(" "), max).join("\n");
        out.push((*start, mid, head));
        out.push((mid, *end, tail));
    }
    out
}

fn wrap_line(text: &str, max: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for w in text.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + w.len() > max {
            lines.push(line.clone());
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(w);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// SRT wants `HH:MM:SS,mmm`.
fn srt_stamp(t: Time) -> String {
    let us = t.micros().max(0);
    format!(
        "{:02}:{:02}:{:02},{:03}",
        us / 3_600_000_000,
        (us / 60_000_000) % 60,
        (us / 1_000_000) % 60,
        (us / 1_000) % 1_000
    )
}

pub fn load(path: &Path) -> Result<Vec<Cue>> {
    let body = std::fs::read_to_string(path)?;
    parse_srt(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_wraps_to_two_lines() {
        let cues = vec![(
            Time::from_micros(0),
            Time::from_micros(3_500_000),
            "I wanted to let you know I taught fifth grade for four years in North Carolina.".into(),
        )];
        let srt = to_srt(&cues);
        let body: Vec<&str> = srt.lines().skip(1).take(4).collect();
        assert!(body[0].contains("-->"), "{srt}");
        assert_eq!(body[0], "00:00:00,000 --> 00:00:03,500");
        let lines = srt.lines().filter(|l| !l.contains("-->") && !l.trim().is_empty() && !l.trim().chars().all(|c| c.is_ascii_digit())).count();
        assert_eq!(lines, 2, "one cue, two lines: {srt}");
    }

    #[test]
    fn an_overlong_cue_becomes_two_cues() {
        let long = "This sentence is deliberately long enough that it cannot fit into two lines of forty two characters each at all.";
        let cues = vec![(Time::from_micros(0), Time::from_micros(4_000_000), long.to_string())];
        let srt = to_srt(&cues);
        assert_eq!(srt.matches("-->").count(), 2, "{srt}");
    }

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
