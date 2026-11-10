//! Word index: normalized words with stable ids, cached by the hash of the
//! audio they came from.
//!
//! The index is what makes `view` and `find` cheap: a clipping task reads a few
//! dozen lines of text instead of loading a transcript or probing a timeline.
//! Ids (`w1`, `w2`, ...) are stable for a given index because the index is
//! written once and keyed by the audio it describes.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::paths::arg;
use crate::time::Time;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Word {
    pub id: String,
    pub text: String,
    pub start: Time,
    pub end: Time,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Index {
    pub version: u32,
    pub audio_hash: String,
    pub audio_duration: Time,
    pub transcript_hash: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default)]
    pub approximate: bool,
    pub words: Vec<Word>,
}

pub const VERSION: u32 = 1;

/// One readable span of speech: what `view` prints and what `find` anchors on.
#[derive(Clone, Debug)]
pub struct Utterance {
    pub start: Time,
    pub end: Time,
    pub first: String,
    pub last: String,
    pub first_i: usize,
    pub last_i: usize,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct Hit {
    pub utterance: Utterance,
    /// Word offsets inside `utterance.text`.
    pub match_start: usize,
    pub match_len: usize,
}

impl Index {
    pub fn path(cache_root: &Path, audio_hash: &str) -> PathBuf {
        cache_root.join("index").join(format!("{audio_hash}.json"))
    }

    pub fn load(path: &Path) -> Result<Index> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("no index at {} (run `sona index`)", path.display()))?;
        let index: Index = serde_json::from_str(&raw)
            .with_context(|| format!("{} is not a valid index", path.display()))?;
        if index.version != VERSION {
            bail!("no_index: index version {} is not supported", index.version);
        }
        Ok(index)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Group words into lines a human (or a model) can scan. A new line starts
    /// on a long silence, after sentence-ending punctuation, or when the line
    /// would get too long to read.
    pub fn utterances(&self) -> Vec<Utterance> {
        const GAP_US: i64 = 700_000;
        const MAX_WORDS: usize = 12;
        const MAX_CHARS: usize = 84;
        let mut out = Vec::new();
        let mut cur: Vec<usize> = Vec::new();
        let mut chars = 0usize;

        for (i, w) in self.words.iter().enumerate() {
            let mut start_new = false;
            if let Some(&p) = cur.last() {
                let prev = &self.words[p];
                let gap = w.start.micros() - prev.end.micros();
                start_new = gap > GAP_US
                    || prev.text.ends_with(['.', '?', '!'])
                    || cur.len() >= MAX_WORDS
                    || chars + 1 + w.text.len() > MAX_CHARS;
            }
            if start_new {
                out.push(self.finish(&cur));
                cur.clear();
                chars = 0;
            }
            chars += w.text.len() + 1;
            cur.push(i);
        }
        if !cur.is_empty() {
            out.push(self.finish(&cur));
        }
        out
    }

    fn finish(&self, idx: &[usize]) -> Utterance {
        let first = &self.words[idx[0]];
        let last = &self.words[*idx.last().unwrap()];
        Utterance {
            start: first.start,
            end: last.end,
            first: first.id.clone(),
            last: last.id.clone(),
            first_i: idx[0],
            last_i: *idx.last().unwrap(),
            text: idx
                .iter()
                .map(|&i| self.words[i].text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        }
    }

    /// Case-insensitive word-sequence match. Punctuation is ignored for
    /// matching and kept in the output.
    pub fn find(&self, query: &str) -> Vec<Hit> {
        let needles: Vec<String> = query.split_whitespace().map(normalize).collect();
        if needles.is_empty() || needles.iter().any(|n| n.is_empty()) {
            return Vec::new();
        }
        let utterances = self.utterances();
        let mut hits = Vec::new();
        let mut i = 0;
        while i + needles.len() <= self.words.len() {
            let matched = needles
                .iter()
                .enumerate()
                .all(|(k, n)| &normalize(&self.words[i + k].text) == n);
            if matched {
                if let Some(u) = utterances
                    .iter()
                    .find(|u| u.first_i <= i && i <= u.last_i)
                {
                    hits.push(Hit {
                        utterance: u.clone(),
                        match_start: i - u.first_i,
                        match_len: needles.len(),
                    });
                }
                i += needles.len();
            } else {
                i += 1;
            }
        }
        hits
    }
}

/// `w27` or `w27-w35`, 1-based and inclusive, as printed by `view`/`find`.
pub fn parse_word_range(raw: &str) -> Result<(usize, usize)> {
    let s = raw.trim();
    let rest = s
        .strip_prefix('w')
        .with_context(|| format!("usage: --words w12 or w12-w19, got `{raw}`"))?;
    let (a, b) = match rest.split_once('-') {
        Some((x, y)) => (x, y.strip_prefix('w').unwrap_or(y)),
        None => (rest, rest),
    };
    let start: usize = a
        .parse()
        .with_context(|| format!("usage: --words w12 or w12-w19, got `{raw}`"))?;
    let end: usize = b
        .parse()
        .with_context(|| format!("usage: --words w12 or w12-w19, got `{raw}`"))?;
    if start == 0 || end < start {
        bail!("usage: --words must be w1 or a range like w12-w19, got `{raw}`");
    }
    Ok((start - 1, end - 1))
}

/// Source times for a word range, with padding on both sides. ASR word
/// boundaries are approximate, so a cut exactly at `start` clips the first
/// phoneme; a small pad is the difference between "fine" and "audible".
pub fn span_for(idx: &Index, first: usize, last: usize, pad: Time) -> Result<(Time, Time)> {
    if last >= idx.words.len() {
        bail!(
            "usage: word w{} is past the end of the index ({} words)",
            last + 1,
            idx.words.len()
        );
    }
    let start = Time::from_micros((idx.words[first].start.micros() - pad.micros()).max(0));
    let end = Time::from_micros(idx.words[last].end.micros() + pad.micros());
    Ok((start, end))
}

/// Lowercase, alphanumeric only. `Ship?` and `ship` are the same token.
pub fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Exactly one line per utterance, cheap to read and grep. Format is stable:
/// `start-end  wFirst-wLast  text`.
pub fn format_line(u: &Utterance) -> String {
    format!(
        "{}-{}  {}-{}  {}",
        clock(u.start),
        clock(u.end),
        u.first,
        u.last,
        u.text
    )
}

/// A hit with the matched words bracketed, so a reader can see why it matched.
pub fn format_hit(idx: &Index, h: &Hit) -> String {
    let words = &idx.words[h.utterance.first_i..=h.utterance.last_i];
    // One bracket pair around the whole match, not per word: `[good memory.]`
    let end = h.match_start + h.match_len;
    let text = words
        .iter()
        .enumerate()
        .map(|(i, w)| match i {
            i if i == h.match_start && h.match_len == 1 => format!("[{}]", w.text),
            i if i == h.match_start => format!("[{}", w.text),
            i if i + 1 == end => format!("{}]", w.text),
            _ => w.text.clone(),
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "{}-{}  {}-{}  {}",
        clock(h.utterance.start),
        clock(h.utterance.end),
        h.utterance.first,
        h.utterance.last,
        text
    )
}

/// `04:12.0` for times under an hour, `1:04:12.0` beyond.
pub fn clock(t: Time) -> String {
    let total = t.secs_f64().max(0.0);
    let h = (total / 3600.0).floor() as u64;
    let m = ((total % 3600.0) / 60.0).floor() as u64;
    let s = total % 60.0;
    if h > 0 {
        format!("{h}:{m:02}:{s:04.1}")
    } else {
        format!("{m:02}:{s:04.1}")
    }
}

/// Words carry punctuation from the provider; keep it — it is what makes the
/// text readable, and `normalize` strips it for matching.
pub fn assign_ids(words: &mut [Word]) {
    for (i, w) in words.iter_mut().enumerate() {
        w.id = format!("w{}", i + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(i: i64, text: &str, start_ms: i64, end_ms: i64) -> Word {
        Word {
            id: format!("w{}", i + 1),
            text: text.into(),
            start: Time::from_micros(start_ms * 1000),
            end: Time::from_micros(end_ms * 1000),
            speaker: None,
            confidence: None,
        }
    }

    fn sample() -> Index {
        Index {
            version: VERSION,
            audio_hash: "test".into(),
            audio_duration: Time::from_secs_f64(10.0),
            transcript_hash: "t".into(),
            provider: "gemini".into(),
            language: None,
            approximate: false,
            words: vec![
                w(0, "Hi.", 0, 500),
                w(1, "We", 600, 700),
                w(2, "ship", 700, 900),
                w(3, "faster", 900, 1200),
                // long gap -> new line
                w(4, "Today.", 3000, 3500),
            ],
        }
    }

    #[test]
    fn utterances_break_on_punctuation_and_silence() {
        // "Hi." ends a sentence, then the 1.8s gap starts a new line too.
        let u = sample().utterances();
        assert_eq!(u.len(), 3);
        assert_eq!(u[0].text, "Hi.");
        assert_eq!(u[1].first, "w2");
        assert_eq!(u[1].last, "w4");
        assert_eq!(u[1].text, "We ship faster");
        assert_eq!(u[2].text, "Today.");
    }

    #[test]
    fn ids_are_assigned_in_order_and_stable() {
        let mut words = sample().words;
        assign_ids(&mut words);
        let ids: Vec<&str> = words.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, ["w1", "w2", "w3", "w4", "w5"]);
        let mut again = sample().words;
        assign_ids(&mut again);
        assert_eq!(ids, again.iter().map(|w| w.id.as_str()).collect::<Vec<_>>());
    }

    #[test]
    fn find_ignores_punctuation_and_case() {
        let idx = sample();
        let hits = idx.find("SHIP faster");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].match_start, 1);
        assert_eq!(hits[0].match_len, 2);
        assert_eq!(
            format_hit(&idx, &hits[0]),
            "00:00.6-00:01.2  w2-w4  We [ship faster]"
        );
        assert!(idx.find("xyzzy").is_empty());
    }

    #[test]
    fn find_single_word_brackets_one_word() {
        let idx = sample();
        let hits = idx.find("faster");
        assert_eq!(hits.len(), 1);
        assert!(format_hit(&idx, &hits[0]).contains("[faster]"));
    }

    #[test]
    fn clock_switches_to_hours_past_3600() {
        assert_eq!(clock(Time::from_secs_f64(252.0)), "04:12.0");
        assert_eq!(clock(Time::from_secs_f64(3852.0)), "1:04:12.0");
    }
}

#[cfg(test)]
mod word_range_tests {
    use super::*;

    #[test]
    fn parses_single_and_range() {
        assert_eq!(parse_word_range("w27").unwrap(), (26, 26));
        assert_eq!(parse_word_range("w27-w35").unwrap(), (26, 34));
        assert_eq!(parse_word_range("w1-w1").unwrap(), (0, 0));
    }

    #[test]
    fn rejects_junk() {
        assert!(parse_word_range("27").is_err());
        assert!(parse_word_range("w0").is_err());
        assert!(parse_word_range("w9-w2").is_err());
        assert!(parse_word_range("w").is_err());
        assert!(parse_word_range("w1-").is_err());
    }

    fn idx() -> Index {
        Index {
            version: VERSION,
            audio_hash: "t".into(),
            audio_duration: Time::from_secs_f64(10.0),
            transcript_hash: "t".into(),
            provider: "gemini".into(),
            language: None,
            approximate: false,
            words: vec![
                Word { id: "w1".into(), text: "Ship".into(), start: Time::from_secs_f64(1.0), end: Time::from_secs_f64(1.4), speaker: None, confidence: None },
                Word { id: "w2".into(), text: "faster.".into(), start: Time::from_secs_f64(1.4), end: Time::from_secs_f64(2.0), speaker: None, confidence: None },
            ],
        }
    }

    #[test]
    fn span_adds_padding_on_both_sides() {
        let (a, b) = span_for(&idx(), 0, 1, Time::parse("120ms").unwrap()).unwrap();
        assert_eq!(a.micros(), 880_000);
        assert_eq!(b.micros(), 2_120_000);
    }

    #[test]
    fn span_never_goes_negative() {
        let (a, _) = span_for(&idx(), 0, 0, Time::parse("2s").unwrap()).unwrap();
        assert_eq!(a.micros(), 0);
    }

    #[test]
    fn span_rejects_past_the_end() {
        assert!(span_for(&idx(), 0, 5, Time::ZERO).is_err());
    }
}
