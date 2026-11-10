# CLI reference

```
sona [--project <path>] <command> [options]
```

`--project` is global and defaults to `sona.json` in the current directory. Paths
inside the project resolve against the project file's directory, so
`sona --project /elsewhere/sona.json` works from any cwd.

## Output contract

Every command prints one JSON object to stdout with `"ok": true|false`. On
failure stdout still carries the object, and stderr repeats two lines:

```
media_missing: no such file: /nope.mp4
fix: sona add ./nope.mp4
```

Exit codes: `0` ok, `2` project/precondition, `3` media, `4` ffmpeg, `5` usage.

Mutating commands all return the same envelope:

```json
{"ok": true, "changed": ["clip_2"], "invalidated": 1,
 "duration": "00:00:07.666", "warnings": [{"code": "clip_past_media", …}]}
```

`invalidated` is the number of render steps the edit can affect. `warnings` is
omitted when empty, and every warning carries a `fix` naming a command that
clears it:

```json
{"code": "clip_past_media",
 "message": "/media/a.mp4 is 00:00:09.000 long but clip clip_1 ends at 00:00:20.000",
 "fix": "sona trim clip_1 --to 00:00:09.000"}
```

Errors carry the same `fix` field, so a caller never has to guess the next move.

### --dry-run

Every mutating command accepts `--dry-run`: it computes the change, reports the
resulting `duration` and `warnings`, and writes nothing — no document write, no
history snapshot, no index file.

```console
$ sona add clip-a.mp4 --dry-run
{"ok": true, "dry_run": true, "added": [{"id": "clip_1", "duration": "00:00:06.000"}],
 "duration": "00:00:06.000"}
$ ls .sona/history 2>/dev/null | wc -l
0
```

Use it to preview a trim, a cut, or a captions swap before committing. `cut`
already had `--dry-run`; it now returns the same envelope as everything else.

## Times

Accepted anywhere a time is expected: `4`, `4s`, `4.5s`, `250ms`,
`00:00:04.500`, `1:02.5`. The fractional part is optional everywhere.
`sona frames --at` additionally accepts percentages of the timeline: `25%`.

## Geometry

`--size WxH` and `--fps` are fixed per project at `init`. `--fps` accepts `30`,
`29.97`, `30000/1001`. Every clip is conformed to the canvas: `fit` pads
(letterbox), `--fill` crops.

---

## init

```
sona init [--size 1920x1080] [--fps 30]
```

Creates `sona.json` and `.sona/`. Fails if the file exists.

```json
{"ok": true, "project": "sona.json", "canvas": {"width": 1920, "height": 1080, "fps": "30"}}
```

## add

```
sona add <file>... [--from <t>] [--to <t>] [--words <range>] [--pad <t>]
                  [--speed <n>] [--at <index>] [--fill]
```

Appends clips, or inserts them at `--at`. `--from`/`--to` are offsets **inside
the source file**, not the timeline. `--speed 1.5` shortens the clip;
`--speed 0.5` lengthens it. Audio pitch is preserved.

```json
{"ok": true, "added": [{"id": "clip_2", "source": "/media/demo.webm", "duration": "00:00:10.666"}],
 "duration": "00:00:18.906"}
```

`--words w12-w19` cuts by index ids instead of times: it resolves the range to
source times with `--pad` (default `120ms`) on each side. `--words` replaces
`--from`/`--to`, and requires the index to describe the file being added.

```console
$ sona find "good memory"
00:12.6-00:14.4  w27-w31  You've got a [good memory.]
$ sona add podcast.mp4 --words w27-w31
{"ok": true, "added": [{"id": "clip_1", "duration": "00:02.040"}]}
```

Notes:

- Source paths are canonicalized at call time, so a project keeps working when
  the scratch directory moves.
- `--to` past the end of the medium is clamped to the medium and reported as a
  `clip_past_media` warning. The `duration` reported is what will render.
- A file with no video stream is rejected (`2`).

## list

```
sona list
```

```json
{"ok": true, "duration": "00:00:08.333", "clips": [
  {"index": 0, "id": "clip_1", "source": "/media/a.mp4", "in": "00:00:01.000",
   "duration": "00:00:05.000", "speed": 1.0, "missing": false}]}
```

`missing: true` means the source file is gone, not that the project is invalid.

## trim

```
sona trim <id> [--from <t>] [--to <t>] [--words <range>] [--pad <t>]
```

Moves the clip's source in/out points. Omitted side is left alone. `--to` must be
after `--from`.

`--words w12-w19` sets both points from the index, padding each side by `--pad`
(default `120ms`). The clip's source must be the file the index describes.

## speed

```
sona speed <id> <factor>
```

Pitch-preserving speed change. Factors outside 0.5–2.0 are chained into multiple
`atempo` filters internally, so `--speed 4` works.

## move

```
sona move <id> <index>
```

Moves a clip to a 0-based position. An index past the end appends.

## rm

```
sona rm <id>
```

One verb for everything removable, so a diagnostic can always name a command that
exists:

| id | removes |
| --- | --- |
| `clip_3` | that clip |
| `text_2` | that text overlay |
| `captions` | the attached SRT |
| `audio` | the music bed |

Removing the last clip leaves an empty project; `render` then fails with `2`.

## text

```
sona text "<content>" --from <t> --to <t> [--pos bottom|center|top] [--size 48] [--color white]
```

Burns text into the frame for the given window. `--color` takes any ffmpeg
colour (`white`, `0x00FF00`, `red@0.8`). Content may contain `:` `,` `'` `%`.

```json
{"ok": true, "changed": ["text_1"], "invalidated": 1, "duration": "00:00:08.333"}
```

A window past the end of the timeline is a `text_past_end` warning, not an error.

## image

```
sona image <file> --from <t> --to <t> [--pos bottom-left] [--width <px>]
                 [--opacity 1.0] [--margin <px>]
```

Places a PNG over the picture: a logo, a lower third, a sting. `--pos` takes
`top`, `top-left`, `top-right`, `center`, `bottom`, `bottom-left`,
`bottom-right`; the corners are what a logo or a lower third actually wants.

```console
$ sona image logo.png --from 1 --to 5 --pos top-right --width 120 --opacity 0.9
{"ok": true, "changed": ["image_1"], "width_px": 120, "invalidated": 1}
$ sona image lower-third.png --from 2 --to 4 --pos bottom --width 560
```

`--width` and `--margin` are pixels at the **project canvas** and are stored as a
fraction of it, so the same project renders correctly at preview size. At 640 px
wide with `--width 120`, the preview profile draws it 90 px wide without being
told. `--opacity` is 0–1 and multiplies the PNG's own alpha.

Images are composited after the text and captions, so a lower third covers burned
subtitles rather than the reverse.

## transition

```
sona transition <id> [--kind crossfade|dip_to_black] [--duration 500ms] [--none]
```

Sets the transition that brings clip `<id>` in from its predecessor. `--none`
restores a hard cut. The clip must not be first, and the duration must be shorter
than both neighbours — the command refuses otherwise, and `diagnose` warns
`transition_too_long` with a fix if a hand-edited project tries it.

```console
$ sona transition clip_2 --kind crossfade --duration 600ms
{"ok": true, "changed": ["clip_2"], "kind": "crossfade", "duration": "00:00:08.400"}
$ sona transition clip_3 --kind dip_to_black --duration 600ms
$ sona list | jq -r '.duration'      # 7.800 — 9s of clips, two 0.6s overlaps
```

**Transitions cost a re-encode.** A plain concat copies streams; an `xfade` chain
has to decode and blend, so the assemble step becomes a real encode. Audio
crossfades in step, and a program with any transition reports its duration minus
the overlaps.

## audio

```
sona audio <file> [--gain <db>]
```

Sets the music bed played under the clips, mixed at `duration=first` so it never
extends the program. Negative gain needs no `--` escape. Replaces any previous
bed; there is exactly one.

## duck

```
sona duck [--db -12] [--ramp 140ms] [--gap 400ms] [--pad 80ms] [--none]
```

Lowers the music bed wherever the word index says there is speech. This is the
payoff of having a transcript: a gain envelope built from words, not a signal
compressor guessing at loudness.

```console
$ sona audio bed.mp3 --gain -6
$ sona duck --db -18
{"ok": true, "changed": ["audio"], "duck_db": -18.0, "phrases": 2, "invalidated": 1}

$ sona plan | grep -o "volume='[^']*'"
volume='1-0.8741*min(1,max(0,min(1,min((t-1.920)/0.140,(4.080-t)/0.140)))+…':eval=frame
```

Measured on a silent-clip bed: `-24.5 dB` in both windows without ducking, and
`-42.5 dB during speech / -24.5 dB in the gaps` with `--db -18`.

How the envelope is built:

- Words closer than `--gap` become one phrase; each phrase grows by `--pad` so
  the duck leads the word slightly.
- Phrases are mapped from **source** time to **timeline** time through the clips,
  honouring `src_in`, speed and transitions, then merged where they overlap.
- Each phrase becomes a trapezoid of length `--ramp`; trapezoids sum and clamp, so
  overlapping phrases never duck harder than `--db`. A word at 3.0s of a clip
  trimmed to start at 10s ducks at 3.0s of the timeline only if the clip actually
  covers it.
- The spans are resolved when you run `duck` and stored in the project, so a
  render needs no index. `duck_stale` warns when the cut has moved since; the fix
  is `sona duck`.
- `--none` removes the ducking and leaves the bed alone.

Every clip must be cut from the file the index describes, or there is no honest
mapping and the command refuses.

## caption

```
sona caption <file.srt>
```

Attaches an SRT and burns it in during the assemble pass. The cue count comes
from parsing the file, so a wrong `cues` value is a parse problem, not a render
problem.

```json
{"ok": true, "changed": ["captions"], "cues": 42, "invalidated": 1, "duration": "00:00:42.500"}
```

## cut

```
sona cut <id> [--fillers] [--silences <threshold>] [--keep 200ms] [--pad 120ms] [--dry-run]
```

Removes filler words and long silences from one clip, replacing it with the spans
that survive. Needs an index for that clip's source.

```console
$ sona cut clip_1 --silences '>0.6s' --fillers
{"ok": true, "changed": ["clip_1"], "replaced_by": ["clip_1", "clip_2", "clip_3"],
 "removed": [{"from": "00:00:00.100", "to": "00:00:00.600"},
             {"from": "00:00:12.500", "to": "00:00:13.000"}],
 "saved": "00:00:02.487", "duration": "00:00:28.300"}
```

- `--fillers` drops `um`, `uh`, `hmm`, `er`, `ah`, `mm` and friends. The list is
  deliberately conservative; deleting "like" because a model guessed is worse
  than leaving an "um" in.
- `--silences 0.6s` shortens every gap longer than 0.6s. `>0.6s` also parses.
  `--keep` (default 200 ms) survives, split at both ends, so speech never starts
  on a hard edge. Leading and trailing silence are shortened too.
- `--pad` is air around a removed filler word, so neighbours do not butt together.
- `--dry-run` returns the `removed` spans and `saved` time without touching the
  project. Use it before a destructive cut.
- Kept spans shorter than 120 ms are dropped: they are clicks, not lines.

The clip is replaced by one clip per surviving span, in order, inheriting speed,
transform and gain. Nothing to remove reports `changed: []`.

Cutting removes time, so anything timed against the old timeline is now offset:
`cut_shifts_layers` warns once for captions and once for overlays.

## scenes

```
sona scenes [--threshold 0.4] [--min 500ms] [--json]
```

Detects shot boundaries with ffmpeg's `scene` score, merges anything shorter than
`--min` into its neighbour, and writes the result into the index:

```console
$ sona scenes
00:00.0-00:02.0  key 00:00.2  scene 1
00:02.0-00:02.2  key 00:02.0  scene 2
00:02.2-00:04.2  key 00:02.4  scene 3
```

`key` is a timestamp inside the shot (10% in, capped at 300 ms) to sample for a
thumbnail — the first frame of a cut is often a flash or a blur.

- `--min` is what makes this usable: at the default 500 ms a 0.2 s flash is
  absorbed; at `--min 100ms` it becomes its own scene.
- `--threshold` is ffmpeg's scene score (0–1). Lower finds more cuts.
- Results are cached in the index against the source's size and mtime, plus the
  threshold and `--min`, so changing any of them re-detects.
- `--json` returns `{n, start, end, keyframe}` per scene.

Scenes live in the index because that is the per-source analysis file, but they
describe the picture rather than the audio: the detection records the stat of the
file it ran on, so a modified source is re-scanned rather than silently trusted.
`scenes` needs an index, which needs a transcript — see `index`.

## index

```
sona index --from <transcript> [--audio <file>] [--force] [--language <code>]
```

Builds the word index for a source's audio. Words get stable ids (`w1`, `w2`, …)
and are cached by the **hash of the extracted 16 kHz mono audio**, so the same
recording is indexed once no matter which container it arrives in.

`--from` accepts a Gemini Transcribe response, a Deepgram nova-3 response, or an
`.srt`/`.vtt` file. SRT only has cue times, so word times inside a cue are
interpolated by character share and the index is marked `approximate: true`.

`--audio` defaults to the first clip's source, then to the already-indexed
source. The audio is downmixed to `.sona/audio/<key>.wav` on first use (deletable
cache).

```json
{"ok": true, "audio_hash": "e6f429cf…", "audio_duration": "00:00:30.988",
 "provider": "gemini", "words": 78, "cached": false,
 "index": ".sona/index/e6f429cf….json"}
```

Re-running with the same audio and transcript reports `cached: true` and takes
milliseconds. A different transcript re-indexes in place and the ids shift — they
are stable for a given index, not across different transcripts.

## view

```
sona view --range <start-end> [--json]
```

Prints the indexed words overlapping the range as one line per utterance:

```console
$ sona view --range 0:05-0:15
00:03.8-00:06.4  w4-w11  It's been ages since I last saw you.
00:06.7-00:08.4  w12-w16  How are you and Jackie?
00:08.5-00:09.6  w17-w19  Yeah, good, thanks.
00:10.1-00:11.3  w20-w23  And your new baby?
00:11.5-00:12.4  w24-w26  George, isn't it?
00:13.1-00:14.1  w27-w31  You've got a good memory.
00:14.4-00:15.6  w32-w35  Yes, he's two now.
```

Each line is `start-end  firstId-lastId  text`. Utterance boundaries fall on a
silence longer than 700 ms, sentence-ending punctuation, or a line getting too
long. The ids are the handles you pass to `trim --from`/`--to` after converting
the time, and the anchor for a caption you write by hand.

`--json` returns the same lines as objects. For a 30-second range the text view
is about **970 bytes against 2,275 for the JSON** — that difference is the whole
reason these two commands print text.

`sona view` and `sona find` are read views: stdout is plain text, not the usual
single JSON object. Pass `--json` when a caller needs the envelope. Errors are
still JSON, and the exit codes are unchanged.

## find

```
sona find "<query>" [--limit 20] [--json]
```

Case- and punctuation-insensitive word-sequence match. One line per hit, with the
matched span bracketed so the reason for the match is visible:

```console
$ sona find "good memory"
00:12.6-00:14.4  w27-w31  You've got a [good memory.]

$ sona find jackie
00:06.7-00:08.4  w12-w16  How are you and [Jackie?]
```

No hits prints nothing and exits 0; add `--json` to get `{"hits": []}` explicitly.
`--limit` caps the hits printed.

## inspect

```
sona inspect
```

`list` plus canvas, overlay/audio/caption presence, and `warnings[]`. Use it
before a render to learn what is wrong without spawning ffmpeg.

## plan

```
sona plan [--preview]
```

The exact argv per step, whether each is already cached, the profile, and the
output path. Runs no ffmpeg (`ffmpeg -version`/`-filters`/`-encoders` are probed
for capability and keying).

```json
{"ok": true, "out": "out.mp4", "steps": [
  {"label": "normalize clip_1", "key": "c0eed4a36e0bcfda", "cached": false,
   "argv": ["ffmpeg", "-y", "-hide_banner", "-loglevel", "error", "-ss", "1.000000", …]}]}
```

This is the debugging lever: if the render is wrong, the argv says why.

## render

```
sona render [-o out.mp4] [--preview] [--loudnorm] [--lufs -14] [--true-peak -1]
            [--lra 11] [--timeout 3600]
```

Executes the plan into `.sona/cache/`, then copies the final step to `-o`.

```json
{"ok": true, "out": "out.mp4", "duration": "00:00:08.333",
 "steps": {"total": 4, "ran": 4, "reused": 0}}
```

`ran`/`reused` are the cache contract: an unchanged re-render reports
`reused == total`; a one-overlay edit reports `ran: 1`.

`--preview` renders a proxy: the canvas fitted into 480x854, 15 fps, crf 30,
ultrafast, 64k audio, written to `preview.mp4` by default. It is a **different
plan**, not a quality knob — every step key includes the profile, so preview and
final caches coexist and neither invalidates the other.

```console
$ sona render --preview
{"ok": true, "profile": "preview", "size": "480x852", "fps": "15",
 "out": "preview.mp4", "steps": {"total": 2, "ran": 2, "reused": 0}}
```

On a 20-second 1080x1920 clip: 1.6 s preview against 7.1 s final, same 4 vCPUs.
Iterate on the preview, ship the final.

`--loudnorm` adds an EBU R128 pass as a final step: video is **copied**, only the
audio is re-encoded through ffmpeg's `loudnorm`. Targets are `--lufs` (integrated,
default `-14`), `--true-peak` (default `-1`) and `--lra` (default `11`).

```console
$ sona render --loudnorm -o loud.mp4
{"ok": true, "loudness": {"i": -14.0, "tp": -1.0, "lra": 11.0},
 "steps": {"total": 2, "ran": 1, "reused": 1}}
```

Measured on a −47.8 LUFS source: `-14.0 LUFS` after, with the base assembly
reused rather than re-encoded. A different `--lufs` is a different cache entry, so
a −20 dB master and a −14 dB master coexist.

It is **one-pass** `loudnorm`, chosen so every step's argv stays knowable at plan
time — the thing `sona plan` promises. Two-pass measurement would make the apply
step depend on a run. The cost is that the gain is applied dynamically across the
program rather than measured once; for a single delivered file that is the usual
trade, and if you need exact integrated numbers you can measure the result with
`ffmpeg -af ebur128` and decide.

The loudness pass is orthogonal to `--preview`: a proxy can be loudness-targeted
too, which is useful for checking a phone speaker mix before the full render.

## frames

```
sona frames [--at <t,t,...>] [--sheet <n>] [-o .sona/preview] [--preview]
```

Writes PNG stills and/or a tiled contact sheet with a burnt-in timecode.
Stills are scaled to 768 px wide for vision-model token cost.

```json
{"ok": true, "frames": [{"path": "/work/proj/.sona/preview/at-00001-000.png"},
                        {"kind": "sheet", "path": "/work/proj/.sona/preview/sheet.png"}]}
```

Reads from the last cached render step, so a render must run first — the final
one by default, or the preview one with `--preview`. The two caches are separate,
so `frames` without the flag cannot see a preview-only render, and each writes to
its own directory (`.sona/preview` vs `.sona/preview-proxy`) so the stills of one
never overwrite the other.

Stills are never upscaled: a proxy still comes out at proxy size, which is both
honest and cheaper to read.

## layers and cameras

The document (version 2) already models layers and camera keys, and `sona` will
refuse to render one rather than ignore it:

```console
$ sona render
{"error":{"code":"composite_not_built",
 "message":"this project has 2 layers, but compositing is designed and not implemented"}}
```

Editing commands still target the `main` layer, and clip ids are unique across the
document, so `trim`, `speed`, `move`, `rm` and `transition` need no layer flag.
The design, including the commands that will exist, is
[design-compositing.md](design-compositing.md).

## history

```
sona history [--limit 20]
```

Every mutating command snapshots `sona.json` into `.sona/history/` **before** it
changes anything. Newest first:

```json
{"ok": true, "undoable": 3, "entries": [
  {"seq": 4, "at": 1794301188, "age": "just now", "command": "trim",
   "clips": 2, "overlays": 1, "captions": false, "index": false}]}
```

The last 50 are kept. `.sona/` stays cache in the sense that deleting it costs
history, never content — but it is the only copy of your undo stack, so do not
clear it reflexively mid-session.

## undo

```
sona undo [--steps 1]
```

Restores the document to how it was before the most recent mutation and drops
that snapshot, so repeating `undo` walks further back.

```console
$ sona undo
{"ok": true, "restored": [{"seq": 4, "command": "trim", "age": "just now", "clips": 2}]}
```

Two behaviours worth knowing:

- **It works on a broken document.** `undo` never parses `sona.json`; it reads
  the snapshot and replaces the file atomically. A corrupted project is one
  command from fixed.
- **No-op snapshots are skipped.** A command that failed after taking a snapshot,
  or a mutation that changed nothing, is consumed silently rather than costing
  you an undo.

`no_history` (exit 2) means there is nothing left to restore.

## doctor

```
sona doctor
```

Read-only capability check: ffmpeg version, `libx264`, `libass`, `drawtext`,
resolved font file, and whether `GEMINI_API_KEY` / `DEEPGRAM_API_KEY` are visible.
Never prints a secret value.
