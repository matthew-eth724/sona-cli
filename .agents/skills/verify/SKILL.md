---
name: verify
description: Drive the `splice` video-editing CLI to prove behavior on this repo — build the binary, create an isolated scratch project, and capture JSON, ffprobe facts, frame stills and pixel-diff evidence. Use when verifying a change to splice, reproducing a render or cache bug, or proving a timeline/caption/overlay feature works end to end.
---

# Verify `splice`

`splice` is a Rust CLI (binary at `target/debug/splice`) that compiles a `splice.json`
project into ffmpeg calls. Everything it prints on stdout is one JSON object.
There is no UI and no server: *driving* it means running commands in a scratch
directory and reading files, exit codes and pixels.

## Launch

There is nothing to keep alive. Launch = build once, then create a fresh
scratch project per drive.

```bash
cd <repo-root>
cargo build                       # ~10s cold, <1s warm
SPLICE="$PWD/target/debug/splice"

SCRATCH=$(mktemp -d /tmp/splice-verify.XXXXXX)
.agents/skills/verify/scripts/make-media.sh "$SCRATCH"
cd "$SCRATCH"
$SPLICE init --size 1280x720 --fps 30
```

Ready when this prints `"ok":true`:

```bash
$SPLICE doctor
```

Ready means more than "the binary exists": `doctor` must report
`"libx264":true`, `"libass":true`, `"drawtext":true` and a non-empty
`ffmpeg` version string. If any is false, stop — renders will fail on a filter,
not on your change.

Each drive gets its own `$SCRATCH`. Two drives never share a `splice.json` or a
`.splice/` cache, so they can run side by side. Never drive in the repo root: `splice`
writes `splice.json`, `.splice/cache/` and `out.mp4` into the current directory.

## Doctor

```bash
$SPLICE doctor                       # ffmpeg filters/encoders, resolved font, keys
ffprobe -v error -show_entries format=duration -of csv=p=0 "$SCRATCH/clip-a.mp4"
```

Run `doctor` first whenever anything looks off. It answers: is ffmpeg capable,
is a font resolvable, are ASR keys present. It never prints a secret value.

## Drive

Every command prints exactly one JSON object to stdout. Read it, do not grep
logs. `jq` is available.

```bash
cd "$SCRATCH"

# timeline
$SPLICE add ./clip-a.mp4 --from 1s --to 6s      # --from/--to are SOURCE times
$SPLICE add ./clip-b.mp4 --speed 1.5
$SPLICE list                                     # {clips:[...],duration:"00:00:08.333"}
$SPLICE trim clip_2 --to 4s
$SPLICE move clip_2 0
$SPLICE rm clip_2
$SPLICE inspect                                  # summary + "warnings"

# layers
$SPLICE text "Ship faster" --from 0.5 --to 3 --pos bottom --size 48
$SPLICE audio ./music.mp3 --gain -14             # negative needs no `--`
$SPLICE caption ./subs.srt                       # {changed:["captions"],cues:2}

# compile and render
$SPLICE plan | jq '.steps[] | {label, cached, argv: .argv[0:6]}'
$SPLICE render -o out.mp4
# {"steps":{"total":4,"ran":4,"reused":0}} on a cold render

# look at the result
$SPLICE frames --at 1,3 --sheet 8 -o .splice/preview
```

Stable handles: command names and the JSON keys (`steps.ran`,
`steps.reused`, `duration`, `warnings[].code`). Do not parse the `argv`
strings for content — use `cached` and `warnings`.

Exit codes are the contract: `0` ok, `2` project/precondition, `3` media, `4`
ffmpeg, `5` usage.

## Evidence

Write proofs to `<repo-root>/verification/<YYYY-MM-DD>-<what>/`. That directory
is outside `$SCRATCH` on purpose: cleanup deletes the scratch dir, never the
evidence.

Capture, per proof:

1. **Command + JSON** — `cmd.txt` with the exact commands, `out.json` with stdout.
2. **Timeline facts** — `scripts/facts.sh out.mp4` (duration, codecs, size, fps).
3. **Pixels** — the stills from `splice frames`, or a crop diff for a specific claim.
4. **Side effects** — `ls .splice/cache` counts, `steps.ran`/`steps.reused`, exit code.

Prove the user path, not an internal one. `splice render` exiting 0 is not proof the
overlay appeared: prove it in pixels.

**Pixel proof for anything layered (text, captions):** render the same project
twice, once with the layer and once without, changing nothing else, then diff
the same timestamp and the same crop. Identical background, so the number is
the layer.

```bash
.agents/skills/verify/scripts/psnr-ab.sh out.mp4 out-bare.mp4 1.5 "1280:120:0:520"
# -> average:15.897  (a burned caption lives here; ~40+ dB means nothing changed)
```

**Audio proof for a music bed:** the residual, not the mean.

```bash
ffmpeg -hide_banner -i out-full.mp4 -i out-bare.mp4 -filter_complex \
 "[0:a][1:a]amix=inputs=2:weights=1 -1:normalize=0,astats=measure_overall=RMS_level:measure_perchannel=none" \
 -f null - 2>&1 | grep 'RMS level'
# -18.4 dB residual means the bed is in the mix; near -inf means it is not
```

**Cache proof:** `steps.reused` is the claim. A cold render reports
`ran == total`; an unchanged re-render reports `reused == total`; a one-field
edit reports `ran == 1` (decorate only) or `ran == 3` (one clip changed:
normalize + concat + decorate).

## Cleanup

```bash
rm -rf "$SCRATCH"        # only what this drive created
```

Never kill by process name and never `rm -rf` a path you did not create. `splice`
leaves no processes behind. Confirm the evidence survived:

```bash
ls <repo-root>/verification/<date>-<what>/
```

## Gotchas

- **`ran: 0` after you changed something is a bug, not a win.** The cache key
  must move when the argv moves. If you edited an overlay or a caption style and
  the render reported `reused: total`, the step's key is not covering its own
  filters — report it, do not accept the fast path.
- **A failed ffmpeg step used to leave a zero-byte cache file that the next run
  trusted.** If a render "succeeds" in 0 steps and the output is 0 bytes or
  silent, check `ls -la .splice/cache` before blaming the encoder.
- **`--from`/`--to` on `add` are positions inside the source file**, not on the
  timeline. `--to 6s` on a 6s clip plus `--from 1s` yields 5s of output.
- **`--to` past the end of the media is clamped**, with a `clip_past_media`
  warning. Mutating commands return their own warnings, so read them there.
- **Source paths are canonicalized when added**, so a project keeps working if
  the scratch dir moves. `--project /elsewhere/splice.json` works from any cwd.
- **Paths inside filters must be relative** (`src/paths.rs`). A `C:\` path
  unescaped becomes "No such filter", not a missing-file error. `cargo test`
  covers this; do not bypass `filter_escape`.
- **Text needs a font.** `doctor` prints the resolved `font`; a null there makes
  every `drawtext` fail with `no_font`.
- **The 10-minute ASR cap sits below the 30-minute hard refusal**, so a long
  file hits `asr_over_limit` (confirmable with `--yes`) before `asr_too_long`.
- **`splice frames` renders from the last plan step's cache**, so it needs a render
  first: `nothing rendered yet (run 'splice render')` is expected on a fresh project.

## Helpers

Both scripts are executable and take an explicit target directory.

```bash
.agents/skills/verify/scripts/make-media.sh "$SCRATCH"   # deterministic fixtures
.agents/skills/verify/scripts/facts.sh out.mp4           # ffprobe facts as JSON
.agents/skills/verify/scripts/psnr-ab.sh a.mp4 b.mp4 1.5 "1280:120:0:520"
```

`make-media.sh` writes `clip-a.mp4` (6s, 640x360@30, 440Hz), `clip-b.mp4` (5s,
1280x720@25, 660Hz), `subs.srt` (2 cues) and `music.mp3` (8s sine bed). All
generated by ffmpeg, so no binaries need to live in the repo.

See `features/README.md` for the feature map.
