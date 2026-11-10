# Render and step cache

## Sub-features

- `render --preview` — a proxy render (480x854, 15 fps, crf 30) with its own cache
  keys; `plan --preview` shows that argv, `frames --preview` reads its output.

- `plan` — the exact ffmpeg argv per step, with `cached`.
- `render -o out.mp4` — executes the plan into `.splice/cache/`, then copies the
  final step to `out.mp4`.
- Incremental reuse: a content hash per step, so an untouched step is skipped.

## How to get to it (user POV)

```bash
$SPLICE plan | jq '.steps[] | {label, cached}'
$SPLICE render -o out.mp4        # cold:  {"steps":{"total":4,"ran":4,"reused":0}}
$SPLICE render -o out.mp4        # warm:  {"steps":{"total":4,"ran":0,"reused":4}}
```

## Driving it with bash + JSON

```bash
cold=$($SPLICE render -o out.mp4 | jq -r '.steps.ran');   echo "cold ran=$cold"
warm=$($SPLICE render -o out.mp4 | jq -r '.steps.reused'); echo "warm reused=$warm"

# one overlay edit must move exactly the decorate step
$SPLICE text "Third line" --from 0 --to 2
$SPLICE render -o out.mp4 | jq '.steps'      # {"ran":1,"reused":3,...}

# one clip edit must move normalize + concat + decorate
$SPLICE trim clip_2 --to 4s
$SPLICE render -o out.mp4 | jq '.steps'      # {"ran":3,"reused":1,...}
```

## Loudness

```bash
$SPLICE render --loudnorm -o loud.mp4 | jq '.loudness, .steps'
ffmpeg -hide_banner -i loud.mp4 -af ebur128 -f null - 2>&1 | grep -A1 'Integrated'
```

Measured: a −47.8 LUFS source becomes −14.0 LUFS. The pass copies video and only
re-encodes audio, so it adds one cheap step: `{"total":2,"ran":1,"reused":1}` on a
project that was already assembled.

## Gotchas

- Preview and final share no cache entries. `reused: 0` after a final render
  following a preview is correct, not a bug.
- `frames` reads final by default; pass `--preview` to read the proxy.
- `--loudnorm` is one-pass, so the target is approached dynamically: check with
  `ebur128` rather than assuming −14.0. Different `--lufs` values are different
  cache entries, so masters coexist.
- The loudness step copies video. If `ran` equals `total` on a loudness-only
  change, an upstream step moved too.

- **`ran: 0` after an edit is a cache-key bug**, not a fast render. The step key
  must include its own argv; the decorate step once keyed only on its inputs and
  served a stale render after a new overlay was added.
- A failed ffmpeg step creates its output file before failing. Historically that
  zero-byte file became a cache hit. If a render "succeeds" in 0 steps with a
  broken output, check `ls -la .splice/cache` first.
- Deleting `.splice/` is always safe: it is cache, never project state.
- `plan` runs no ffmpeg and needs no cache; it is the fastest way to see what a
  change compiles to.
