# Timeline

## Sub-features

- `init` — create `va.json` with canvas size and fps.
- `add` — append or insert a clip, optional `--from`/`--to`/`--speed`/`--at`/`--fill`.
- `list` / `inspect` — the timeline as JSON, plus `warnings[]`.
- `trim` — change a clip's source in/out points.
- `speed` — playback rate, pitch preserved.
- `move` / `rm` — reorder or remove.

## How to get to it (user POV)

Any terminal in a scratch directory. `va` reads `va.json` from the current
directory unless `--project` is given.

## Driving it with bash + JSON

```bash
$VA init --size 1280x720 --fps 30
$VA add ./clip-a.mp4 --from 1s --to 6s        # 5s of a 6s source
$VA add ./clip-b.mp4 --speed 2.0              # 5s source becomes 2.5s
$VA list | jq '.duration, [.clips[].id]'
$VA move clip_2 0                             # clip_2 now first
$VA trim clip_1 --from 2s
$VA inspect | jq '.warnings'
```

## Gotchas

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
