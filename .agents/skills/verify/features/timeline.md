# Timeline

## Sub-features

- `init` — create `splice.json` with canvas size and fps.
- `add` — append or insert a clip, optional `--from`/`--to`/`--speed`/`--at`/`--fill`.
- `list` / `inspect` — the timeline as JSON, plus `warnings[]`.
- `trim` — change a clip's source in/out points.
- `speed` — playback rate, pitch preserved.
- `move` / `rm` — reorder or remove.

## How to get to it (user POV)

Any terminal in a scratch directory. `splice` reads `splice.json` from the current
directory unless `--project` is given.

## Driving it with bash + JSON

```bash
$SPLICE init --size 1280x720 --fps 30
$SPLICE add ./clip-a.mp4 --from 1s --to 6s        # 5s of a 6s source
$SPLICE add ./clip-b.mp4 --speed 2.0              # 5s source becomes 2.5s
$SPLICE list | jq '.duration, [.clips[].id]'
$SPLICE move clip_2 0                             # clip_2 now first
$SPLICE trim clip_1 --from 2s
$SPLICE inspect | jq '.warnings'
```

## Speed ramps

```bash
$SONA ramp clip_1 --from 1 --to 2 --slices 4
$SONA plan | jq -r '.steps[].label'    # normalize 1/4 … 4/4, concat 4 segments
$SONA list | jq -r '.duration'         # 00:00:05.529 for 8 s at 1x->2x
```

The durations are the cheap proof: with more slices they converge on the exact
integral of the speed curve. For 8 s at 1×→2× that is 8·ln2 = 5.545 s; measured
5.333 (1 slice), 5.485 (2), 5.529 (4), 5.541 (8), 5.544 (16).

## Gotchas

- A ramp is a staircase, not a curve. More `--slices` is smoother and costs one
  render step each, so a 4-slice ramp turns one clip into four cache entries.
- `speed` clears a ramp, because a constant speed and a ramp are two ways to say
  the same thing.
- Ramp keys are absolute source offsets, so a `trim` that moves `src_in` keeps the
  ramp meaningful; keys outside the trim are ignored.

- Documents are **version 2**: `tracks.layers[].clips`, with `z`, `start`,
  `place` and `motion` per layer. A version-1 document upgrades in memory on open
  and is written back as v2 on the next save, so an old project still renders —
  proven by byte-identical plans captured before the change
  (`tests/fixtures/plans/`).
- A project with more than one layer, or a layer with camera keys, refuses to
  render with `composite_not_built`. It validates and saves; it does not render
  by ignoring the extras.
- `splice list` shows the `main` layer. Clip ids are unique across the document, so
  `trim`/`speed`/`move`/`rm` need no layer flag.

- Every mutating command returns the `warnings[]` it just created, so a trim
  past the end of a source is visible immediately, not on the next `inspect`.
- `--to` past the media duration is **clamped** to the media, and reported as
  `clip_past_media`. The claimed `duration` still matches the rendered length.
- `--fill` crops to the canvas (`scale=…:increase,crop=…`); the default `--fit`
  pads. On portrait footage (576x1024) into 1080x1920, `--fit` is a pure upscale
  with no bars.
- `--from`/`--to` are **source** times. `add clip-a.mp4 --from 1s --to 6s`
  contributes 5s, not 6s.
- `add` canonicalizes the source path at call time; a missing file fails with
  `usage`-class exit 2, not a later render error.
- `--at N` inserts at index N; N past the end is an error, not a clamp.
- `trim --to` must be after `--from`, in source time.
- `inspect` reports `warnings` for missing media, clips past the end of a
  source, and text past the end of the timeline. Warnings never block a render.
