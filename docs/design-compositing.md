# Compositing and keyframes — design

Status: **designed, not built.** Deliberately last: it changes the document
shape, so it should land once nothing else is moving.

## Problem

`sona` has one video layer. `tracks.video` is an ordered list, and its order *is*
the timeline. Everything downstream leans on that: duration is a sum, cache keys
are per clip, index word times map through one list, `duck` freezes spans against
a layout fingerprint. Picture-in-picture needs a second simultaneous picture;
zoom and pan need the picture to change *within* a clip.

Two things make this the hardest change left. First, the flat list is load-bearing
in six places, so "add a tracks field" is not a refactor, it is a new shape with
six existing callers. Second, ffmpeg's `overlay` takes exactly two inputs and
cannot be chained inside one filter string, so every simultaneous layer costs a
node, and every animation costs an expression string that has to be stable enough
to hash.

## Usage (caller's view)

Written first; the types below are derived from it.

### Picture-in-picture

```console
$ sona init --size 1920x1080 --fps 30
$ sona add background.mp4                       # the main layer, z = 0
$ sona layer add pip --z 10 --place 0.72,0.72,0.25 --start 4s
$ sona add demo.mp4 --layer pip --from 10s --to 24s
$ sona render -o out.mp4
```

`--place x,y,size` is a fraction of the canvas: top-left at 72%, 25% wide. Layer
`pip` starts at 4 s on the program clock and runs for its own 14 s.

### Background plate under the cut

```console
$ sona layer add plate --z -10                  # negative z is simply below
$ sona add loop.mov --layer plate
$ sona render -o out.mp4
```

No new concept: a plate is a layer that happens to be under everything.

### Zoom and pan

```console
$ sona key main --t 0    --zoom 1.0 --at 0.50,0.50
$ sona key main --t 2.5s --zoom 1.6 --at 0.55,0.48
$ sona key main --t 6s   --zoom 2.2 --at 0.72,0.40
{"ok":true,"layer":"main","keys":3,"span":"0.000000..6.000000",
 "invalidated":{"composite":1},"duration":"00:00:12.000"}

$ sona key main --list
t=0.000000  zoom=1.000  at=0.500,0.500
t=2.500000  zoom=1.600  at=0.550,0.480
t=6.000000  zoom=2.200  at=0.720,0.400
```

A slow push-in on the cut is three lines. The answer says exactly which single
step moved, because that is the whole incremental story: **no clip re-encodes
because a keyframe moved.**

### What an agent does when it is wrong

```console
$ sona key main --t 99s --zoom 0.5
{"ok":false,"error":{"code":"bad_key","message":"zoom must be >= 1.0",
 "fix":"sona key main --t 99s --zoom 1.0"}}

$ sona rm pip
{"ok":true,"changed":["pip"],"duration":"00:00:18.906"}
```

## Shape

### Module map

| module | owns |
| --- | --- |
| `project.rs` | `Layer`, `Place`, `Motion`, `Key`, `upgrade()`; invariants; no ffmpeg strings |
| `motion.rs` (new, ~120) | interpolation, expression generation, lowering choice |
| `composite.rs` (new, ~160) | z-ordered overlay chain, visibility windows, placement |
| `graph.rs` | the five stages and every step key (unchanged entry point) |

`graph::plan` stays the only entry point. `motion` and `composite` are private
modules; nothing outside `graph` can import them.

### Core types

```rust
pub struct Tracks {
    #[serde(default)] pub layers: Vec<Layer>,   // unordered; z is the order
    pub audio: Option<AudioBed>,
    pub overlays: Vec<Overlay>,                 // text and images, above every layer
    pub captions: Option<Captions>,
    pub index: Option<IndexRef>,
}

pub struct Layer {
    pub id: String,            // stable, like clip ids
    pub z: i32,                // total order; unique; higher is on top
    #[serde(default)] pub start: Time,   // where this layer's t=0 sits on the clock
    #[serde(default)] pub place: Place,  // where its frame lands on the canvas
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<Motion>,          // the camera
    pub clips: Vec<Clip>,                // sequential inside the layer: today's model
}

/// Canvas-normalised. `Place::CANVAS` = 0,0 / 1.0,1.0 is the default.
pub struct Place { pub x: f32, pub y: f32, pub size: f32 }

/// The camera. Layer-local time, linear between keys, held flat outside them.
pub struct Motion { pub keys: Vec<Key> }
pub struct Key { pub t: Time, pub zoom: f64, pub at: Point }   // zoom >= 1.0
```

Three fields do three different jobs and none of them overlaps another: `z` is
stacking, `start` is when, `place` is where on the canvas, `motion` is where the
camera looks inside the layer. **Placement is static in v1.** Animating where a
PiP slides is a follow-up, and it is visibly absent rather than half-present.

```rust
impl Motion {
    /// Piecewise-linear, held at the ends. Pure: same keys, same numbers.
    fn sample(&self, t: Time) -> (f64, Point);
    /// `if(lt(t,2.500000),1.000000+(t-0.000000)*0.240000, …)` — a deterministic
    /// string, so it is a legitimate cache input. `var` is `t` for crop/overlay
    /// and `time` for zoompan: one formula, two filter vocabularies.
    fn exprs(&self, var: &str) -> (String, String, String);   // zoom, cx, cy
    fn max_zoom(&self) -> f64;
}
impl Place { fn px(&self, canvas: (u32, u32)) -> (u32, u32, u32, u32); }  // even-rounded
impl Tracks { fn by_z(&self) -> Vec<&Layer>; }   // the one sorted accessor
fn composite_graph(layers: &[LayerRender], canvas: (u32, u32), fps: Rational) -> String;
```

### Invariants

Hard errors, checked in `validate` (media-free, runs on open):

- `z` unique; at least one layer; every layer has at least one clip.
- `z == 0` exists, has `place == Place::CANVAS`, and carries the program audio.
- `Motion::keys` strictly ascending in `t`; `zoom >= 1.0`; `at` inside `0..=1`.
- `start >= 0`.

Warnings, from `diagnose` (needs files):

- `layer_audio_dropped` — a layer above z=0 has clips with audio. Fix: move those
  clips to `main`. One audio source in v1, stated rather than silently mixed.
- `layer_past_program` — a layer starts after the program ends.
- `place_off_canvas` — the placement rectangle leaves the frame.

### How a plan is built

Five stages. Stages 1, 2, 4 and 5 are today's code, run per layer; stage 3 is new.

1. **normalize** — unchanged, one step per clip, always at full canvas size.
   Placement-free on purpose: `z`, `start`, `place` and `motion` never reach a
   normalize key, so sliding a PiP cannot re-encode its footage.
2. **assemble** — unchanged concat/xfade, run once per layer, in z order.
3. **composite** — skipped entirely when there is one layer and no motion, which
   makes the legacy plan **byte-identical**. Otherwise one step, one
   `-filter_complex`, inputs = layer renders in z order:

   ```
   [0:v]setpts=PTS-STARTPTS[L0];
   [1:v]scale=iw*K:ih*K:flags=lanczos,zoompan=z='<z*K>':x='iw*<cx>':y='ih*<cy>'
        :d=1:s=1920x1080:fps=30[L1];
   [L0][L1]overlay=eof_action=pass[o1];
   [2:v]setpts=PTS+4.000000/TB,scale=480:270,format=rgba[L2];
   [o1][L2]overlay=x='<px>':y='<py>':eof_action=pass:enable='between(t,4,18)'[v]
   ```

   One camera rule, two lowerings, chosen by the data:
   `window(zoom, at) = (cx - 1/(2z), cy - 1/(2z), 1/z, 1/z)`.

   | keys | lowering | why |
   | --- | --- | --- |
   | one key, or all keys equal | `crop` (static size) + `scale` back | exact, cheap, no resampling surprises |
   | varying zoom | `scale` to `K = max_zoom`, then `zoompan d=1` | `scale` has no `t`; only `zoompan` animates zoom |

   `eof_action=pass` lets a shorter layer stop contributing without `tpad` and
   without an alpha intermediate. `enable='between(...)'` is the only visibility
   mechanism: a PiP at 4 s is a time window, not a transparent pixel.
4. **decorate** — unchanged, above the composite. Captions, text and images stay
   above every video layer; `z` never reorders decorations.
5. **loudnorm** — unchanged.

Program duration becomes `max over layers (start + layer_duration)`, computed in
`Project::duration`. With one layer that reduces to today's
`sum(clips) - sum(transitions)` exactly.

### Cache keys

| step | key material | what moves it |
| --- | --- | --- |
| normalize | argv, source stat, ffmpeg version, profile | the clip only |
| assemble | that layer's segment keys, "concat"/"xfade", version, profile | that layer's clips |
| composite | layer keys in z order, "composite", version, profile, **the graph string** | z, start, place, motion, layer membership |
| decorate / loudnorm | upstream key, argv, version, profile | their own inputs |

A keyframe edit moves the composite key, and therefore decorate and loudnorm
downstream. Nothing else. That is the property the whole decomposition exists to
buy, and it is why placement lives in composite rather than in normalize.

## Rationale

### Synthesis decision

Two runner candidates survived, and they disagreed about the one thing that
matters most: **where placement and animation live.**

- **A** kept every clip render at full canvas and composited placement,
  visibility and camera in a separate composite stage. No alpha intermediates.
- **B** baked per-track size and per-clip keyframes into normalize, producing
  alpha video (prores 4444 / vp9) for animated clips, with both clip-local and
  track-level keyframe lists.

A is the base. B's alpha intermediates exist only because it animates *inside*
normalize; moving that work to a compositing stage removes the need for alpha
entirely, and with it a codec choice, a colour-space risk and a 3–5× intermediate
size for every animated clip. B's two keyframe time bases (clip-local and
timeline) were also a genuine "two ways to do one task": a user could express the
same zoom two ways and get different results at a transition.

Grafted from B: per-layer `target` sizing was rejected (it would move normalize
keys when a PiP is resized), but its `timeline_in` thinking is why `start` exists
as an explicit field rather than implicit clip order. Its migration section is
why the design rejects a legacy `video` mirror.

Rejected outright: `tracks.video` kept alongside `tracks.layers` as a derived
mirror. Two writers for timeline state is *split ownership*, and an agent that
edits one is invisible to the other.

A third shape was considered and rejected in synthesis: **compositing folded into
the existing decorate step** (layers become extra inputs to the same graph
string). It is fewer steps, but then the decorate key covers z-order, placement
and every camera expression, so changing one text overlay would re-run the
composite — and the composite step's "skipped when trivial" property, which keeps
legacy plans byte-identical, disappears.

### Tradeoffs accepted

- **Placement static in v1** in exchange for one unambiguous place for it. Moving
  a PiP over time is a real future feature, and half-supporting it now would put
  position in two representations.
- **One audio source** (z=0) in exchange for not modelling per-layer audio buses.
  A PiP with its own voiceover is a warning and a documented limit, not a silent
  mix.
- **No mid-layer gaps**: clips inside a layer are contiguous, so a PiP that
  appears twice is two layers at consecutive `z`. In exchange, `move` inside a
  layer stays the simple index shuffle it is today.
- **Unique `z`** in exchange for a total order with no tie-breaking rule.
  Non-overlapping layers can order arbitrarily; overlapping equal `z` is exactly
  the ambiguity worth refusing.
- **A document version bump to 2** in exchange for not carrying a compatibility
  field forever. `upgrade()` rewrites v1 in memory on load.
- **`zoompan` for animated zoom** in exchange for accepting a filter with known
  rough edges (integer rounding, its own scaler). The alternative is not
  implementing zoom.

### Migration

- **v1 → v2 on load.** `tracks.video` becomes one layer `main`, `z: 0`,
  `start: 0`, `place: CANVAS`, `motion: None`. Written back on the next save.
  `validate` rejects v1 on disk *only* if `upgrade` could not run, which it always
  can.
- **Commands.** `add` gains `--layer` (default `main`). `trim`, `speed`, `move`,
  `rm`, `transition` need no flag: clip ids are unique across the document, so a
  clip id is already an unambiguous handle. `rm` extends to `layer <id>`.
- **`history`/`undo`.** Snapshots store the whole project, so a v1 snapshot
  restores a v1 document and upgrades again on the next open. Nothing to migrate.
- **Index and `duck`.** Word times map through the clips of z=0, unchanged. The
  `layout` fingerprint gains the layer structure (z, start, place), because moving
  a layer moves what is audible. `duck_stale` then covers the new cases.
- **`cut`** operates on a clip, unchanged. Its `cut_shifts_layers` warning already
  covers captions and overlays; it gains layer motion as a third case.

### Open questions

1. **Does `start` belong on the layer or on its first clip?** On the layer, today.
   If mid-layer gaps ever land, `start` moves onto clips and the layer field
   disappears — one of the two must not exist.
2. **Animated placement.** Zoom and pan are camera moves inside a layer. Sliding a
   PiP across the canvas is `place` changing over time, which needs a second
   animatable field. Is that worth the second representation, or is
   `place`-per-layer-time-slice enough?
3. **Audio buses.** One source is honest but limiting. `Layer::audio: bool` plus a
   mix step is the obvious next increment; the mix belongs in decorate, not
   composite.
4. **`zoompan` quality.** Needs one comparison render at 2× zoom against a
   `crop`+`scale` equivalent before shipping. If it is visibly soft, the fallback
   is to animate `scale` with `eval=frame`, which ffmpeg does not currently
   support for `scale` — so the fallback is really "constant-zoom only".
5. **Expression length.** A hundred keys is a few kilobytes in one argv. Fine for
   Linux, worth checking on Windows.

### Next implementation step

Land the document change and its migration **before any compositor code**:
`Layer`/`Place`/`Motion`/`Key` in `project.rs`, `upgrade()` v1→v2 on open, `z` and
`motion` validation, `Project::duration` as a max over layers, and `by_z()`.
Then assert the migration is provable: a v1 project upgrades and produces a plan
whose argv is **byte-identical** to what it produced before the change. That test
is the gate; the compositor comes after it passes.

## Red-flag screening

| flag | resolution |
| --- | --- |
| shallow module | `motion` and `composite` are private; the public surface grows by four commands and one field per layer |
| information leakage | no ffmpeg vocabulary in `project.rs`; expressions are produced inside `motion` and consumed inside `graph` |
| temporal decomposition | each stage owns knowledge: normalize owns clips, assemble owns a layer's topology, composite owns stacking and camera |
| pass-through | none added; `plan` composes stages directly |
| split ownership | one `layers` array, one `by_z()` accessor, `Store` the only writer |
| two ways to do one task | placement is `place` only, camera is `motion` only, visibility is `enable` only; no alpha intermediates |
| importable internals | `motion`/`composite` are `mod`, not `pub mod` |
| hand-synced list | z order derived from `layers` at every read; no second ordering anywhere |
