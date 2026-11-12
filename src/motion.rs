//! Turning camera keys into filter expressions.
//!
//! One rule, two lowerings. `window(zoom, at)` describes the visible rectangle of
//! the layer in canvas fractions: a crop of `1/zoom` centred at `at`. A constant
//! camera lowers to `crop`+`scale`, which is exact and cheap. A moving one lowers
//! to `scale` to the maximum zoom and then `zoompan`, because `scale` has no `t` —
//! it cannot animate, and `zoompan` can.
use crate::project::{Key, Motion};
use crate::time::{Rational, Time};

#[derive(Clone, Debug, PartialEq)]
pub enum Lowering {
    /// No motion: nothing to add.
    None,
    /// Every key agrees, so the crop rectangle is fixed.
    Static { zoom: f64, cx: f64, cy: f64 },
    /// Keys disagree: expressions in `t`.
    Animated { keys: Vec<Key>, max_zoom: f64 },
}

pub fn lowering(m: &Motion) -> Lowering {
    let first = match m.keys.first() {
        Some(k) => *k,
        None => return Lowering::None,
    };
    let constant = m
        .keys
        .iter()
        .all(|k| (k.zoom - first.zoom).abs() < 1e-9 && (k.at.x - first.at.x).abs() < 1e-9 && (k.at.y - first.at.y).abs() < 1e-9);
    // "No motion" is a property of *all* the keys, not the first one: a camera
    // that starts at 1.0 and ends at 2.0 begins at the identity and still moves.
    let identity = (first.zoom - 1.0).abs() < 1e-9
        && (first.at.x - 0.5).abs() < 1e-9
        && (first.at.y - 0.5).abs() < 1e-9;
    if identity && constant {
        return Lowering::None;
    }
    if constant {
        Lowering::Static {
            zoom: first.zoom,
            cx: first.at.x as f64,
            cy: first.at.y as f64,
        }
    } else {
        Lowering::Animated {
            keys: m.keys.clone(),
            max_zoom: m.max_zoom(),
        }
    }
}

fn even(v: f64) -> u32 {
    ((v.round().max(2.0) as u32) / 2) * 2
}

/// Safer piecewise builder: nested `if` walking forward, so each segment checks
/// its own end. `pick` reads the animated field.
pub fn lerp_expr(var: &str, keys: &[Key], pick: impl Fn(&Key) -> f64) -> String {
    fn build(var: &str, keys: &[Key], pick: &dyn Fn(&Key) -> f64, i: usize) -> String {
        if i + 1 >= keys.len() {
            return format!("{:.6}", pick(&keys[keys.len() - 1]));
        }
        let (a, b) = (&keys[i], &keys[i + 1]);
        let (va, vb) = (pick(a), pick(b));
        let t0 = a.t.secs_f64();
        let t1 = b.t.secs_f64();
        let span = (t1 - t0).max(1e-6);
        let slope = (vb - va) / span;
        format!(
            "if(lt({var},{t1:.6}),{va:.6}+({var}-{t0:.6})*{slope:.6},{})",
            build(var, keys, pick, i + 1)
        )
    }
    build(var, keys, &pick, 0)
}

/// `crop`+`scale` for a fixed camera. `canvas` is the layer render's size.
pub fn static_chain(l: &Lowering, canvas: (u32, u32)) -> Option<String> {
    let Lowering::Static { zoom, cx, cy } = l else {
        return None;
    };
    let w = even(canvas.0 as f64 / zoom);
    let h = even(canvas.1 as f64 / zoom);
    Some(format!(
        "crop={w}:{h}:(iw-ow)*{cx:.6}:(ih-oh)*{cy:.6},scale={}:{}:flags=lanczos",
        canvas.0, canvas.1
    ))
}

/// `scale` to the maximum zoom, then `zoompan`. `var` is what `zoompan` calls
/// time: `time` (seconds since the first frame).
pub fn animated_chain(l: &Lowering, canvas: (u32, u32), fps: Rational) -> Option<String> {
    let Lowering::Animated { keys, max_zoom } = l else {
        return None;
    };
    let k = max_zoom.max(1.0);
    let z = lerp_expr("time", keys, |k| k.zoom);
    let cx = lerp_expr("time", keys, |k| k.at.x as f64);
    let cy = lerp_expr("time", keys, |k| k.at.y as f64);
    // `zoompan`'s `z` *is* the displayed zoom: pre-scaling by `max_zoom` buys
    // resolution for the zoom, but multiplying `z` by it doubled the zoom at every
    // key. Measured with a still frame and a crop sweep.
    Some(format!(
        "scale=iw*{k:.6}:ih*{k:.6}:flags=lanczos,zoompan=z='({z})':x='({cx})*iw-(iw/zoom)/2':y='({cy})*ih-(ih/zoom)/2':d=1:s={w}x{h}:fps={fps}",
        w = canvas.0,
        h = canvas.1,
        fps = format_args!("{}/{}", fps.num, fps.den),
    ))
}

/// Where a layer's frame lands, as `(x, y, side)` in output pixels, or `None`
/// when it fills the canvas and needs no scaling.
pub fn place_px(x: f32, y: f32, size: f32, canvas: (u32, u32)) -> Option<(u32, u32, u32)> {
    if (size - 1.0).abs() < f32::EPSILON && x.abs() < f32::EPSILON && y.abs() < f32::EPSILON {
        return None;
    }
    let side = even((canvas.0 as f32 * size) as f64);
    Some((
        (canvas.0 as f32 * x).round().max(0.0) as u32,
        (canvas.1 as f32 * y).round().max(0.0) as u32,
        side,
    ))
}

/// Keeps a layer visible only between its start and its end.
pub fn enable_expr(start: Time, end: Time) -> String {
    format!(
        "between(t,{:.6},{:.6})",
        start.secs_f64(),
        end.secs_f64()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Point;

    fn key(t: f64, zoom: f64, x: f32, y: f32) -> Key {
        Key {
            t: Time::from_secs_f64(t),
            zoom,
            at: Point { x, y },
        }
    }

    #[test]
    fn a_centred_zoom_of_one_is_no_motion() {
        let m = Motion {
            keys: vec![key(0.0, 1.0, 0.5, 0.5)],
        };
        assert_eq!(lowering(&m), Lowering::None);
    }

    #[test]
    fn agreeing_keys_lower_to_a_fixed_crop() {
        let m = Motion {
            keys: vec![key(0.0, 1.6, 0.5, 0.5), key(4.0, 1.6, 0.5, 0.5)],
        };
        let l = lowering(&m);
        assert!(matches!(l, Lowering::Static { .. }));
        let chain = static_chain(&l, (1920, 1080)).unwrap();
        // 1080/1.6 = 675, rounded down to the nearest even height
        assert!(chain.starts_with("crop=1200:674:"), "{chain}");
        assert!(chain.ends_with("scale=1920:1080:flags=lanczos"));
    }

    #[test]
    fn disagreeing_keys_lower_to_zoompan() {
        let m = Motion {
            keys: vec![key(0.0, 1.0, 0.5, 0.5), key(4.0, 2.0, 0.5, 0.5)],
        };
        let l = lowering(&m);
        assert!(matches!(l, Lowering::Animated { max_zoom, .. } if (max_zoom - 2.0).abs() < 1e-9));
        let chain = animated_chain(&l, (640, 360), Rational::new(25, 1)).unwrap();
        assert!(chain.contains("scale=iw*2.000000"));
        assert!(chain.contains("zoompan=z='(if("), "{chain}");
        assert!(!chain.contains("zoompan=z='2.000000*"), "the pre-scale must not multiply z");
        assert!(chain.contains("s=640x360:fps=25/1"));
    }

    #[test]
    fn the_expression_interpolates_and_holds() {
        let keys = vec![key(0.0, 1.0, 0.5, 0.5), key(4.0, 2.0, 0.5, 0.5)];
        let e = lerp_expr("t", &keys, |k| k.zoom);
        // at t=0 the value is 1, at t=4 it is 2, with slope 0.25 per second
        assert!(e.starts_with("if(lt(t,4.000000),1.000000+(t-0.000000)*0.250000,"));
        assert!(e.ends_with("2.000000)"));
    }

    #[test]
    fn a_placement_that_fills_the_canvas_needs_no_scaling() {
        assert!(place_px(0.0, 0.0, 1.0, (1920, 1080)).is_none());
        let (x, y, side) = place_px(0.5, 0.25, 0.25, (1920, 1080)).unwrap();
        assert_eq!((x, y), (960, 270));
        assert_eq!(side, 480);
        assert_eq!(side % 2, 0);
    }

    #[test]
    fn visibility_is_a_time_window() {
        let e = enable_expr(Time::from_secs_f64(4.0), Time::from_secs_f64(18.5));
        assert_eq!(e, "between(t,4.000000,18.500000)");
    }
}
