# Render and step cache

## Sub-features

- `plan` — the exact ffmpeg argv per step, with `cached`.
- `render -o out.mp4` — executes the plan into `.sona/cache/`, then copies the
  final step to `out.mp4`.
- Incremental reuse: a content hash per step, so an untouched step is skipped.

## How to get to it (user POV)

```bash
$SONA plan | jq '.steps[] | {label, cached}'
$SONA render -o out.mp4        # cold:  {"steps":{"total":4,"ran":4,"reused":0}}
$SONA render -o out.mp4        # warm:  {"steps":{"total":4,"ran":0,"reused":4}}
```

## Driving it with bash + JSON

```bash
cold=$($SONA render -o out.mp4 | jq -r '.steps.ran');   echo "cold ran=$cold"
warm=$($SONA render -o out.mp4 | jq -r '.steps.reused'); echo "warm reused=$warm"

# one overlay edit must move exactly the decorate step
$SONA text "Third line" --from 0 --to 2
$SONA render -o out.mp4 | jq '.steps'      # {"ran":1,"reused":3,...}

# one clip edit must move normalize + concat + decorate
$SONA trim clip_2 --to 4s
$SONA render -o out.mp4 | jq '.steps'      # {"ran":3,"reused":1,...}
```

## Gotchas

- **`ran: 0` after an edit is a cache-key bug**, not a fast render. The step key
  must include its own argv; the decorate step once keyed only on its inputs and
  served a stale render after a new overlay was added.
- A failed ffmpeg step creates its output file before failing. Historically that
  zero-byte file became a cache hit. If a render "succeeds" in 0 steps with a
  broken output, check `ls -la .sona/cache` first.
- Deleting `.sona/` is always safe: it is cache, never project state.
- `plan` runs no ffmpeg and needs no cache; it is the fastest way to see what a
  change compiles to.
