//! Text overlays, compiled to ASS.
//!
//! Captions already go through libass via the `subtitles` filter. Titles used to
//! go through `drawtext`, which is a *second* text engine with its own escaping,
//! its own font lookup and its own positioning rules — and, for the bundled
//! ffmpeg, its own dependency that the build could not satisfy. One engine means
//! one set of behaviours to test and one library to ship.
use anyhow::{bail, Result};

use crate::project::{Anchor, Overlay, Project, TextOverlay};
use crate::time::Time;

/// The whole overlay document for a canvas, or `None` when there are no titles.
pub fn document(project: &Project, width: u32, height: u32, family: &str) -> Option<String> {
    let texts: Vec<&TextOverlay> = project
        .tracks
        .overlays
        .iter()
        .filter_map(|o| match o {
            Overlay::Text(t) => Some(t),
            _ => None,
        })
        .collect();
    if texts.is_empty() {
        return None;
    }

    let mut out = String::new();
    out.push_str("[Script Info]\nScriptType: v4.00+\n");
    out.push_str(&format!("PlayResX: {width}\nPlayResY: {height}\n"));
    // Wrap like a subtitle line, not like a word processor.
    out.push_str("WrapStyle: 0\nScaledBorderAndShadow: yes\n\n");
    out.push_str("[V4+ Styles]\n");
    out.push_str("Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n");
    // BorderStyle 3 is an opaque box, which is what drawtext's `box=1` drew. The
    // box colour lives in OutlineColour and `Outline` is its padding.
    out.push_str(&format!(
        "Style: Overlay,{family},48,&H00FFFFFF,&H00FFFFFF,&H8C000000,&H00000000,0,0,0,0,100,100,0,0,3,16,0,2,0,0,0,1\n\n"
    ));
    out.push_str("[Events]\n");
    out.push_str("Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n");

    for t in texts {
        let (an, x, y) = placement(t.anchor, width, height);
        let colour = match ass_colour(&t.color) {
            Ok(c) => c,
            Err(_) => "&H00FFFFFF".to_string(),
        };
        out.push_str(&format!(
            "Dialogue: 0,{},{},Overlay,,0,0,0,,{{\\an{an}\\pos({x},{y})\\fs{}\\c{}\\bord16}}{}\n",
            stamp(t.at.start),
            stamp(t.at.end),
            t.size,
            colour,
            escape(&t.content),
        ));
    }
    Some(out)
}

/// Alignment number, and the anchor point `\pos` refers to.
fn placement(anchor: Anchor, width: u32, height: u32) -> (u8, u32, u32) {
    const M: u32 = 48;
    const BOTTOM: u32 = 64;
    match anchor {
        Anchor::Top => (8, width / 2, M),
        Anchor::TopLeft => (7, M, M),
        Anchor::TopRight => (9, width.saturating_sub(M), M),
        Anchor::Center => (5, width / 2, height / 2),
        Anchor::Bottom => (2, width / 2, height.saturating_sub(BOTTOM)),
        Anchor::BottomLeft => (1, M, height.saturating_sub(BOTTOM)),
        Anchor::BottomRight => (3, width.saturating_sub(M), height.saturating_sub(BOTTOM)),
    }
}

/// `h:mm:ss.cc`, which is what ASS counts in.
fn stamp(t: Time) -> String {
    let cs = (t.micros().max(0) as f64 / 10_000.0).round() as u64;
    format!("{}:{:02}:{:02}.{:02}", cs / 360_000, (cs / 6_000) % 60, (cs / 100) % 60, cs % 100)
}

/// ffmpeg colour names and `0xRRGGBB`/`#RRGGBB`, as ASS's `&HAABBGGRR`.
pub fn ass_colour(raw: &str) -> Result<String> {
    let (body, alpha) = match raw.split_once('@') {
        Some((c, a)) => {
            let a: f32 = a.trim().parse().context_err()?;
            (c, (255.0 - a.clamp(0.0, 1.0) * 255.0).round() as u32)
        }
        None => (raw, 0),
    };
    let body = body.trim();
    let rgb = match body.to_ascii_lowercase().as_str() {
        "white" => 0xFFFFFF,
        "black" => 0x000000,
        "red" => 0xFF0000,
        "green" => 0x00FF00,
        "blue" => 0x0000FF,
        "yellow" => 0xFFFF00,
        "cyan" | "aqua" => 0x00FFFF,
        "magenta" | "fuchsia" => 0xFF00FF,
        "orange" => 0xFFA500,
        "gray" | "grey" => 0x808080,
        other if other.starts_with("0x") => {
            u32::from_str_radix(&other[2..], 16).map_err(|e| anyhow::anyhow!("colour: {e}"))?
        }
        other if other.starts_with('#') => {
            u32::from_str_radix(&other[1..], 16).map_err(|e| anyhow::anyhow!("colour: {e}"))?
        }
        other => bail!("unsupported colour `{other}`; use a name, 0xRRGGBB or #RRGGBB"),
    };
    let (r, g, b) = ((rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF);
    Ok(format!("&H{:02X}{b:02X}{g:02X}{r:02X}", alpha))
}

/// ASS has no escape for braces — they open override blocks — and a newline is
/// `\N`. Everything else is literal.
fn escape(text: &str) -> String {
    text.replace('{', "(")
        .replace('}', ")")
        .replace('\n', "\\N")
}

trait ContextErr<T> {
    fn context_err(self) -> Result<T>;
}
impl<T, E: std::fmt::Display> ContextErr<T> for std::result::Result<T, E> {
    fn context_err(self) -> Result<T> {
        self.map_err(|e| anyhow::anyhow!("colour: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Project, TextOverlay};
    use crate::time::TimeRange;
    use crate::time::Rational;

    fn with_text(content: &str, anchor: Anchor) -> Project {
        let mut p = Project::new(640, 360, Rational::new(25, 1));
        p.tracks.overlays.push(Overlay::Text(TextOverlay {
            id: "text_1".into(),
            content: content.into(),
            at: TimeRange::new(Time::from_secs_f64(1.0), Time::from_secs_f64(3.0)).unwrap(),
            anchor,
            size: 48,
            color: "white".into(),
        }));
        p
    }

    #[test]
    fn no_titles_means_no_document() {
        let p = Project::new(640, 360, Rational::new(25, 1));
        assert!(document(&p, 640, 360, "sans").is_none());
    }

    #[test]
    fn a_title_becomes_one_dialogue_with_the_canvas_in_its_header() {
        let doc = document(&with_text("hello", Anchor::Bottom), 640, 360, "Noto Sans").unwrap();
        assert!(doc.contains("PlayResX: 640"));
        assert!(doc.contains("PlayResY: 360"));
        assert!(doc.contains("Style: Overlay,Noto Sans"));
        // an\an2 is bottom-centre; the y is the canvas height less the margin
        assert!(doc.contains("\\an2\\pos(320,296)"), "{doc}");
        assert!(doc.contains("Dialogue: 0,0:00:01.00,0:00:03.00,Overlay"));
        assert!(doc.trim_end().ends_with("hello"));
    }

    #[test]
    fn anchors_map_to_the_corner_they_name() {
        let cases = [
            (Anchor::TopLeft, "\\an7\\pos(48,48)"),
            (Anchor::TopRight, "\\an9\\pos(592,48)"),
            (Anchor::Center, "\\an5\\pos(320,180)"),
            (Anchor::BottomLeft, "\\an1\\pos(48,296)"),
            (Anchor::BottomRight, "\\an3\\pos(592,296)"),
            (Anchor::Top, "\\an8\\pos(320,48)"),
        ];
        for (anchor, expected) in cases {
            let doc = document(&with_text("x", anchor), 640, 360, "sans").unwrap();
            assert!(doc.contains(expected), "{anchor:?} -> {doc}");
        }
    }

    #[test]
    fn colours_become_ass_bgr_with_inverted_alpha() {
        assert_eq!(ass_colour("white").unwrap(), "&H00FFFFFF");
        assert_eq!(ass_colour("red").unwrap(), "&H000000FF");
        assert_eq!(ass_colour("0x00FF00").unwrap(), "&H0000FF00");
        assert_eq!(ass_colour("#0000FF").unwrap(), "&H00FF0000");
        // 80% opaque is 20% transparent = 0x33
        assert_eq!(ass_colour("white@0.8").unwrap(), "&H33FFFFFF");
        assert!(ass_colour("chartreuse").is_err());
    }

    #[test]
    fn braces_do_not_become_override_blocks() {
        let doc = document(&with_text("50% {off}", Anchor::Bottom), 640, 360, "sans").unwrap();
        assert!(!doc.contains("{off}"));
        assert!(doc.contains("50% (off)"));
    }

    #[test]
    fn timestamps_are_centiseconds() {
        assert_eq!(stamp(Time::from_secs_f64(0.0)), "0:00:00.00");
        assert_eq!(stamp(Time::from_secs_f64(1.005)), "0:00:01.01");
        assert_eq!(stamp(Time::from_secs_f64(62.5)), "0:01:02.50");
    }
}
