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
 "duration": "00:00:07.666", "warnings": []}
```

`invalidated` is the number of render steps the edit can affect. `warnings` is
omitted when empty.

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
sona add <file>... [--from <t>] [--to <t>] [--speed <n>] [--at <index>] [--fill]
```

Appends clips, or inserts them at `--at`. `--from`/`--to` are offsets **inside
the source file**, not the timeline. `--speed 1.5` shortens the clip;
`--speed 0.5` lengthens it. Audio pitch is preserved.

```json
{"ok": true, "added": [{"id": "clip_2", "source": "/media/demo.webm", "duration": "00:00:10.666"}],
 "duration": "00:00:18.906"}
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
sona trim <id> [--from <t>] [--to <t>]
```

Moves the clip's source in/out points. Omitted side is left alone. `--to` must be
after `--from`.

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

Removes a clip. Removing the last clip leaves an empty project; `render` then
fails with `2`.

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

## audio

```
sona audio <file> [--gain <db>]
```

Sets the music bed played under the clips, mixed at `duration=first` so it never
extends the program. Negative gain needs no `--` escape. Replaces any previous
bed; there is exactly one.

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

## inspect

```
sona inspect
```

`list` plus canvas, overlay/audio/caption presence, and `warnings[]`. Use it
before a render to learn what is wrong without spawning ffmpeg.

## plan

```
sona plan
```

The exact argv per step, whether each is already cached, and the output path.
Runs no ffmpeg.

```json
{"ok": true, "out": "out.mp4", "steps": [
  {"label": "normalize clip_1", "key": "c0eed4a36e0bcfda", "cached": false,
   "argv": ["ffmpeg", "-y", "-hide_banner", "-loglevel", "error", "-ss", "1.000000", …]}]}
```

This is the debugging lever: if the render is wrong, the argv says why.

## render

```
sona render [-o out.mp4] [--timeout 3600]
```

Executes the plan into `.sona/cache/`, then copies the final step to `-o`.

```json
{"ok": true, "out": "out.mp4", "duration": "00:00:08.333",
 "steps": {"total": 4, "ran": 4, "reused": 0}}
```

`ran`/`reused` are the cache contract: an unchanged re-render reports
`reused == total`; a one-overlay edit reports `ran: 1`.

## frames

```
sona frames [--at <t,t,...>] [--sheet <n>] [-o .sona/preview]
```

Writes PNG stills and/or a tiled contact sheet with a burnt-in timecode.
Stills are scaled to 768 px wide for vision-model token cost.

```json
{"ok": true, "frames": [{"path": "/work/proj/.sona/preview/at-00001-000.png"},
                        {"kind": "sheet", "path": "/work/proj/.sona/preview/sheet.png"}]}
```

Reads from the last cached render step, so a render must run first.

## doctor

```
sona doctor
```

Read-only capability check: ffmpeg version, `libx264`, `libass`, `drawtext`,
resolved font file, and whether `GEMINI_API_KEY` / `DEEPGRAM_API_KEY` are visible.
Never prints a secret value.
