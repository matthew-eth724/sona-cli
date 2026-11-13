# CLI reference

```
splice [--project <path>] <command> [options]
```

`--project` is global and defaults to `splice.json` in the current directory. Paths
inside the project resolve against the project file's directory, so
`splice --project /elsewhere/splice.json` works from any cwd.

## Output contract

Every command prints one JSON object to stdout with `"ok": true|false`. On
failure stdout still carries the object, and stderr repeats two lines:

```
media_missing: no such file: /nope.mp4
fix: splice add ./nope.mp4
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
 "fix": "splice trim clip_1 --to 00:00:09.000"}
```

Errors carry the same `fix` field, so a caller never has to guess the next move.

### --dry-run

Every mutating command accepts `--dry-run`: it computes the change, reports the
resulting `duration` and `warnings`, and writes nothing — no document write, no
history snapshot, no index file.

```console
$ splice add clip-a.mp4 --dry-run
{"ok": true, "dry_run": true, "added": [{"id": "clip_1", "duration": "00:00:06.000"}],
 "duration": "00:00:06.000"}
$ ls .splice/history 2>/dev/null | wc -l
0
```

Use it to preview a trim, a cut, or a captions swap before committing. `cut`
already had `--dry-run`; it now returns the same envelope as everything else.

## Times

Accepted anywhere a time is expected: `4`, `4s`, `4.5s`, `250ms`,
`00:00:04.500`, `1:02.5`. The fractional part is optional everywhere.
`splice frames --at` additionally accepts percentages of the timeline: `25%`.

## Geometry

`--size WxH` and `--fps` are fixed per project at `init`. `--fps` accepts `30`,
`29.97`, `30000/1001`. Every clip is conformed to the canvas: `fit` pads
(letterbox), `--fill` crops.

---

## init

```
splice init [--size 1920x1080] [--fps 30]
```

Creates `splice.json` and `.splice/`. Fails if the file exists.

```json
{"ok": true, "project": "splice.json", "canvas": {"width": 1920, "height": 1080, "fps": "30"}}
```

## add

```
splice add <file>... [--from <t>] [--to <t>] [--words <range>] [--pad <t>]
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
$ splice find "good memory"
00:12.6-00:14.4  w27-w31  You've got a [good memory.]
$ splice add podcast.mp4 --words w27-w31
{"ok": true, "added": [{"id": "clip_1", "duration": "00:02.040"}]}
```

Notes:

- A source inside the project directory is stored relative to it; a source
  outside is stored absolute. The value is read back against the project
  directory, so the project keeps working when the directory moves. `add`'s
  response reports `source` in that stored form, matching what `list` shows.
- `--to` past the end of the medium is clamped to the medium and reported as a
  `clip_past_media` warning. The `duration` reported is what will render.
- A file with no video stream is rejected (`2`).

## list

```
splice list
```

```json
{"ok": true, "duration": "00:00:08.333", "clips": [
  {"index": 0, "id": "clip_1", "source": "/media/a.mp4", "in": "00:00:01.000",
   "duration": "00:00:05.000", "speed": 1.0, "missing": false}]}
```

`missing: true` means the source file is gone, not that the project is invalid.

## trim

```
splice trim <id> [--from <t>] [--to <t>] [--words <range>] [--pad <t>]
```

Moves the clip's source in/out points. Omitted side is left alone. `--to` must be
after `--from`.

`--words w12-w19` sets both points from the index, padding each side by `--pad`
(default `120ms`). The clip's source must be the file the index describes.

## speed

```
splice speed <id> <factor>
```

Pitch-preserving speed change. Factors outside 0.5–2.0 are chained into multiple
`atempo` filters internally, so `--speed 4` works.

## move

```
splice move <id> <index>
```

Moves a clip to a 0-based position. An index past the end appends.

## rm

```
splice rm <id>
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

## ramp

```
splice ramp <id> --from <speed> --to <speed> [--slices 4] [--none] [--dry-run]
```

Speed that changes across a clip. Implemented by **slicing**: the source is cut at
the key times and each slice plays at one constant speed — the average of the ramp
at its two ends — so the audio keeps its pitch (each slice gets its own `atempo`)
and both streams are retimed identically, which is what keeps them in sync.

```console
$ splice ramp clip_1 --from 1 --to 2 --slices 4
{"ok":true,"changed":["clip_1"],"from":1.0,"to":2.0,"slices":4,"clip_duration":"00:00:05.529"}

$ splice plan | jq -r '.steps[].label'
normalize clip_1 1/4
normalize clip_1 2/4
normalize clip_1 3/4
normalize clip_1 4/4
concat 4 segments
```

The trade is explicit: each slice is its own render step and its own cache entry,
and the motion is a staircase rather than a true curve. More slices is smoother and
costs more steps. Measured on 8 seconds at 1×→2×, the duration converges on the
true integral as slices increase: 5.333 s (1), 5.485 (2), 5.529 (4), 5.541 (8),
5.544 (16), against 8·ln2 = 5.545.

`splice speed <id> <n>` still sets a constant speed, and it **clears any ramp** —
a constant speed and a ramp are two ways to say the same thing.

## text

```
splice text "<content>" --from <t> --to <t> [--pos bottom|center|top] [--size 48] [--color white]
```

Burns text into the frame for the given window. `--color` takes any ffmpeg
colour (`white`, `0x00FF00`, `red@0.8`). Content may contain `:` `,` `'` `%`.

```json
{"ok": true, "changed": ["text_1"], "invalidated": 1, "duration": "00:00:08.333"}
```

A window past the end of the timeline is a `text_past_end` warning, not an error.

## image

```
splice image <file> --from <t> --to <t> [--pos bottom-left] [--width <px>]
                 [--opacity 1.0] [--margin <px>]
```

Places a PNG over the picture: a logo, a lower third, a sting. `--pos` takes
`top`, `top-left`, `top-right`, `center`, `bottom`, `bottom-left`,
`bottom-right`; the corners are what a logo or a lower third actually wants.

```console
$ splice image logo.png --from 1 --to 5 --pos top-right --width 120 --opacity 0.9
{"ok": true, "changed": ["image_1"], "width_px": 120, "invalidated": 1}
$ splice image lower-third.png --from 2 --to 4 --pos bottom --width 560
```

`--width` and `--margin` are pixels at the **project canvas** and are stored as a
fraction of it, so the same project renders correctly at preview size. At 640 px
wide with `--width 120`, the preview profile draws it 90 px wide without being
told. `--opacity` is 0–1 and multiplies the PNG's own alpha.

Images are composited after the text and captions, so a lower third covers burned
subtitles rather than the reverse.

## transition

```
splice transition <id> [--kind crossfade|dip_to_black] [--duration 500ms] [--none]
```

Sets the transition that brings clip `<id>` in from its predecessor. `--none`
restores a hard cut. The clip must not be first, and the duration must be shorter
than both neighbours — the command refuses otherwise, and `diagnose` warns
`transition_too_long` with a fix if a hand-edited project tries it.

```console
$ splice transition clip_2 --kind crossfade --duration 600ms
{"ok": true, "changed": ["clip_2"], "kind": "crossfade", "duration": "00:00:08.400"}
$ splice transition clip_3 --kind dip_to_black --duration 600ms
$ splice list | jq -r '.duration'      # 7.800 — 9s of clips, two 0.6s overlaps
```

**Transitions cost a re-encode.** A plain concat copies streams; an `xfade` chain
has to decode and blend, so the assemble step becomes a real encode. Audio
crossfades in step, and a program with any transition reports its duration minus
the overlaps.

## audio

```
splice audio <file> [--gain <db>]
```

Sets the music bed played under the clips, mixed at `duration=first` so it never
extends the program. Negative gain needs no `--` escape. Replaces any previous
bed; there is exactly one.

## duck

```
splice duck [--db -12] [--ramp 140ms] [--gap 400ms] [--pad 80ms] [--none]
```

Lowers the music bed wherever the word index says there is speech. This is the
payoff of having a transcript: a gain envelope built from words, not a signal
compressor guessing at loudness.

```console
$ splice audio bed.mp3 --gain -6
$ splice duck --db -18
{"ok": true, "changed": ["audio"], "duck_db": -18.0, "phrases": 2, "invalidated": 1}

$ splice plan | grep -o "volume='[^']*'"
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
  is `splice duck`.
- `--none` removes the ducking and leaves the bed alone.

Every clip must be cut from the file the index describes, or there is no honest
mapping and the command refuses.

## transcribe

```
splice transcribe [--provider gemini|deepgram] [--model <id>] [--language <code>]
                  [--audio <file>] [--yes] [--max-duration 10m]
                  [--srt captions.srt] [--attach] [--dry-run]
```

Calls the provider and **writes the subtitles**. This is what a key is for: it
normalises the audio, uploads it, parses the reply with the same code that reads
saved transcripts, writes the index, and writes an SRT to `--srt`
(`captions.srt` beside the project by default).

**It does not burn them.** Making captions and putting them on the picture are
separate decisions, so attaching is a second command — or `--attach` when you
mean it:

```console
$ splice transcribe
{"ok":true,…,"captions":"/proj/captions.srt","attached":false,
 "next":"splice caption /proj/captions.srt"}

$ splice caption captions.srt     # now they burn on the next render
```

The SRT goes beside the project, never into `.splice/`: the cache is documented as
deletable, and a file the project points at must not be.

```console
$ splice transcribe --dry-run
{"provider":"gemini","model":"gemini-3.5-transcribe","audio":".splice/asr/…mp3",
 "bytes":481451,"duration":"00:01:00.239","cache_key":"07a3d28f…","endpoint":"…"}

$ splice transcribe
{"ok":true,"provider":"gemini","model":"gemini-3.5-transcribe","words":122,
 "cues":20,"cached":false,"captions":".splice/asr/07a3d28f….srt",
 "index":".splice/asr/07a3d28f….json","duration":"00:01:00.239"}
```

- **Provider**: `--provider`, else the first one with a key, else `asr_no_key`
  naming both `config set` commands. `whisper` is the third option and runs
  locally: no key, no network, no cost — but it needs the program and a model
  file configured:

  ```console
  $ splice config set whisper.program /opt/whisper.cpp/whisper-cli
  $ splice config set whisper.model /opt/whisper.cpp/ggml-small.bin
  $ splice transcribe --provider whisper
  ```

  `whisper.model` is a **path to a `ggml-*.bin`**, not a name — the program takes
  a file. `whisper.program` defaults to `whisper-cli` or `whisper` on `PATH`.
  It is slow by local-transcription standards: measured on 4 vCPUs with
  `ggml-large-v3-turbo`, 15 seconds of audio took 12–15 seconds, and 40 seconds
  took 2 m 29 s. The 10-minute cap applies here too, because a long local run is
  also a surprise.
- **Model**: `--model`, else the configured one, else the pinned default.
- **Audio**: `--audio`, else the first clip's source. Downmixed to mono 16 kHz
  mp3 at 64 kbit — ten minutes is about 4.8 MB, comfortably inside Gemini's 20 MB
  inline limit.
- **Cap**: audio longer than `--max-duration` (default `10m`) is refused with
  `asr_over_limit` unless `--yes`. The check runs *before* the upload, so a
  refusal costs nothing.
- **Cache**: `.splice/asr/<hash>.json` keyed by audio hash + provider + model +
  language. A second run is `cached: true` and makes no request. Different model
  or provider, different entry.
- **Captions**: the SRT is built from the same words as the index, so the two
  cannot disagree. It is written, not attached; `--attach` collapses the two steps
  when you are certain.
- **`duck_stale`**: if ducking is configured, new word times invalidate the spans
  it mapped. The fix line says `sona duck`.

Silence is a valid answer, not an error: a provider that finds no speech
returns `{"words":0,"speech":false}` with an `asr_no_speech` warning and attaches
nothing. Gemini signals it by omitting `steps` entirely from a completed
interaction.

Failures are classified so a caller can act without parsing prose: `asr_no_key`,
`asr_key_invalid` (Gemini answers a bad key with **400**, not 401 — matched on its
`API_KEY_INVALID` reason), `asr_bad_request`, `asr_rate_limited`,
`asr_too_large`, `asr_unavailable`, `asr_unreadable`.

## caption

```
splice caption <file.srt>
```

Attaches an SRT and burns it in during the assemble pass. The cue count comes
from parsing the file, so a wrong `cues` value is a parse problem, not a render
problem.

```json
{"ok": true, "changed": ["captions"], "cues": 42, "invalidated": 1, "duration": "00:00:42.500"}
```

## cut

```
splice cut <id> [--fillers] [--silences <threshold>] [--keep 200ms] [--pad 120ms] [--dry-run]
```

Removes filler words and long silences from one clip, replacing it with the spans
that survive. Needs an index for that clip's source.

```console
$ splice cut clip_1 --silences '>0.6s' --fillers
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
splice scenes [--threshold 0.4] [--min 500ms] [--json]
```

Detects shot boundaries with ffmpeg's `scene` score, merges anything shorter than
`--min` into its neighbour, and writes the result into the index:

```console
$ splice scenes
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
splice index --from <transcript> [--audio <file>] [--force] [--language <code>]
```

Builds the word index for a source's audio. Words get stable ids (`w1`, `w2`, …)
and are cached by the **hash of the extracted 16 kHz mono audio**, so the same
recording is indexed once no matter which container it arrives in.

`--from` accepts a Gemini Transcribe response, a Deepgram nova-3 response, or an
`.srt`/`.vtt` file. SRT only has cue times, so word times inside a cue are
interpolated by character share and the index is marked `approximate: true`.

`--model` records which model produced the words. It defaults to the configured
model for the detected provider, is stored in `splice.json`, and **a change forces
a re-index**: cached words from `nova-3` are not the words `nova-4` would return,
and reusing them silently is the kind of cache bug this tool has already had once.

`--audio` defaults to the first clip's source, then to the already-indexed
source. The audio is downmixed to `.splice/audio/<key>.wav` on first use (deletable
cache).

```json
{"ok": true, "audio_hash": "e6f429cf…", "audio_duration": "00:00:30.988",
 "provider": "gemini", "words": 78, "cached": false,
 "index": ".splice/index/e6f429cf….json"}
```

Re-running with the same audio and transcript reports `cached: true` and takes
milliseconds. A different transcript re-indexes in place and the ids shift — they
are stable for a given index, not across different transcripts.

## view

```
splice view --range <start-end> [--json]
```

Prints the indexed words overlapping the range as one line per utterance:

```console
$ splice view --range 0:05-0:15
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

`splice view` and `splice find` are read views: stdout is plain text, not the usual
single JSON object. Pass `--json` when a caller needs the envelope. Errors are
still JSON, and the exit codes are unchanged.

## find

```
splice find "<query>" [--limit 20] [--json]
```

Case- and punctuation-insensitive word-sequence match. One line per hit, with the
matched span bracketed so the reason for the match is visible:

```console
$ splice find "good memory"
00:12.6-00:14.4  w27-w31  You've got a [good memory.]

$ splice find jackie
00:06.7-00:08.4  w12-w16  How are you and [Jackie?]
```

No hits prints nothing and exits 0; add `--json` to get `{"hits": []}` explicitly.
`--limit` caps the hits printed.

## inspect

```
splice inspect
```

`list` plus canvas, overlay/audio/caption presence, and `warnings[]`. Use it
before a render to learn what is wrong without spawning ffmpeg.

## plan

```
splice plan [--preview]
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
splice render [-o out.mp4] [--preview] [--loudnorm] [--lufs -14] [--true-peak -1]
            [--lra 11] [--timeout 3600] [--progress] [--no-progress]
```

Executes the plan into `.splice/cache/`, then copies the final step to `-o`.

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
$ splice render --preview
{"ok": true, "profile": "preview", "size": "480x852", "fps": "15",
 "out": "preview.mp4", "steps": {"total": 2, "ran": 2, "reused": 0}}
```

On a 20-second 1080x1920 clip: 1.6 s preview against 7.1 s final, same 4 vCPUs.
Iterate on the preview, ship the final.

`--loudnorm` adds an EBU R128 pass as a final step: video is **copied**, only the
audio is re-encoded through ffmpeg's `loudnorm`. Targets are `--lufs` (integrated,
default `-14`), `--true-peak` (default `-1`) and `--lra` (default `11`).

```console
$ splice render --loudnorm -o loud.mp4
{"ok": true, "loudness": {"i": -14.0, "tp": -1.0, "lra": 11.0},
 "steps": {"total": 2, "ran": 1, "reused": 1}}
```

Measured on a −47.8 LUFS source: `-14.0 LUFS` after, with the base assembly
reused rather than re-encoded. A different `--lufs` is a different cache entry, so
a −20 dB master and a −14 dB master coexist.

It is **one-pass** `loudnorm`, chosen so every step's argv stays knowable at plan
time — the thing `splice plan` promises. Two-pass measurement would make the apply
step depend on a run. The cost is that the gain is applied dynamically across the
program rather than measured once; for a single delivered file that is the usual
trade, and if you need exact integrated numbers you can measure the result with
`ffmpeg -af ebur128` and decide.

The loudness pass is orthogonal to `--preview`: a proxy can be loudness-targeted
too, which is useful for checking a phone speaker mix before the full render.

## render progress

`render` writes one JSON progress object per line to **stderr** — never stdout —
so a caller watching a pipe can tell a long render from a hung process:

```json
{"progress":"step","index":1,"total":4,"label":"normalize clip_1","state":"start"}
{"progress":"ffmpeg","index":1,"total":4,"out_time_us":1500000}
{"progress":"step","index":1,"total":4,"label":"normalize clip_1","state":"ran","ms":1234}
```

Step lines (`start`, then `ran`/`reused`) are on by default for every step
that reaches a result — a failing step emits `start` and then the error JSON,
with no `ran`/`reused`.
ffmpeg tick lines (`out_time_us`) are gated: on by default when stderr is a TTY,
forced with `--progress` or `SPLICE_PROGRESS=verbose`, and silenced by
`--no-progress` or `SPLICE_PROGRESS=0` (`--no-progress` wins over
`--progress`; `SPLICE_PROGRESS=0` wins over both). stdout stays exactly one
JSON object.

## frames

```
splice frames [--at <t,t,...>] [--sheet <n>] [-o .splice/preview] [--preview]
```

Writes PNG stills and/or a tiled contact sheet with a burnt-in timecode.
Stills are scaled to 768 px wide for vision-model token cost.

```json
{"ok": true, "frames": [{"path": "/work/proj/.splice/preview/at-00001-000.png"},
                        {"kind": "sheet", "path": "/work/proj/.splice/preview/sheet.png"}]}
```

Reads from the last cached render step, so a render must run first — the final
one by default, or the preview one with `--preview`. The two caches are separate,
so `frames` without the flag cannot see a preview-only render, and each writes to
its own directory (`.splice/preview` vs `.splice/preview-proxy`) so the stills of one
never overwrite the other.

Stills are never upscaled: a proxy still comes out at proxy size, which is both
honest and cheaper to read.

## grade

```
splice grade [--brightness -1..1] [--contrast 0..4] [--saturation 0..4]
             [--gamma 0.05..10] [--lut <file.cube>] [--clear] [--dry-run]
```

One look applied to the whole program, compiled into a single `eq` filter (plus
`lut3d` when a LUT is given) and **placed before the text and captions**, so a
grade changes the picture and not the words on top of it.

```console
$ splice grade --brightness 0.15 --contrast 1.3 --saturation 1.6
{"ok":true,"changed":["grade"],"brightness":0.15,"contrast":1.3,"saturation":1.6,"lut":null}
```

Measured on a test pattern: mean luma 125 → 158 and saturation 113 → 144, which
is what those numbers ask for. An identity LUT lands within 43.5 dB of the
ungraded render, so the wiring does not tint anything by itself.

Knobs not given keep their previous value, so `splice grade --gamma 1.2` after a
contrast setting does not reset the contrast. `--clear` removes the lot.

## mask

```
splice mask <layer> --chroma <color> [--similarity 0.30] [--blend 0.0]
splice mask <layer> --vignette [--angle 0.628]
splice mask <layer> --none
```

A mask shapes one layer's frames, before placement scales them, so it composes
with a picture-in-picture.

```console
$ splice mask pip --chroma 0x00FF00 --similarity 0.3
{"ok":true,"changed":["pip"],"mask":"colorkey=0x00FF00:0.3000:0.0000"}
```

Verified on a green card over a blue base: at one pixel the unkeyed render shows
green (1,254,0) and the keyed render shows the base (35,68,171); a red patch
survives in both; outside the layer both are identical.

A mask on the layer at `z = 0` is refused: there is nothing underneath it to
reveal.

## layer

```
splice layer add <id> --z <n> [--place x,y,size] [--start <t>] [--dry-run]
splice layer set <id> [--z <n>] [--place x,y,size] [--start <t>] [--dry-run]
splice layer rm <id> [--dry-run]
splice layer list
```

A layer is one simultaneous picture. `z` is the stacking order and must be
unique; the layer at `z = 0` carries the program audio and must fill the
canvas, and the bottom layer (normally the one at `z = 0`) is the program
frame. `--place x,y,size` is the rectangle it occupies, in canvas fractions.
`--start` is where its t=0 sits on the program clock.

Two rules, kept separate:

- the bottom layer (lowest `z`) is the program frame: its `place`,
  `place_keys` and `start` are reported, not applied (a `start` on the bottom
  emits a `setpts` that the encoder re-bases away);
- the layer at `z = 0` carries the program audio and must fill the canvas
  (`place` of `{0,0,1}`).

`splice key <id> --place`, `splice layer set <id> --start <t>` and
`splice layer set <id> --place x,y,size` are refused (exit 5, `usage`) when
`<id>` is the bottom layer — add a layer above it (`splice layer add pip --z
10`) and edit that, or use `--zoom`/`--at` for a camera move on the bottom.
`--start 0` and `--place 0,0,1` are no-ops there and are allowed, as are
`--clear`, `--rm`, `--list` and `splice layer rm`: they are how a user gets out
of the state. A hand-edited document whose bottom layer still carries those
values keeps rendering; they are reported as `warnings[]` (`place_keys_unused`,
`layer_start_unused`, `place_unused`) by `splice inspect`, `splice plan`,
`splice render` and `splice frames`, never as errors.

```console
$ splice layer add pip --z 10 --place 0.6,0.6,0.3 --start 1s
$ splice add demo.mp4 --layer pip
{"added":[{"id":"clip_2","duration":"00:00:03.000"}],"duration":"00:00:06.000"}

$ splice layer list
{"layers":[{"id":"main","z":0,"clips":1,"keys":0,"place":{"x":0,"y":0,"size":1}},
           {"id":"pip","z":10,"clips":1,"keys":0,"place":{"x":0.6,"y":0.6,"size":0.3}}],
 "duration":"00:00:06.000"}
```

Program duration becomes **the longest layer**, not the sum: layers play at the
same time.

## key

```
splice key <layer> --t <time> --zoom <n> --at x,y [--dry-run]     # camera
splice key <layer> --t <time> --place x,y [--dry-run]            # slide the layer
splice key <layer> --list
splice key <layer> --rm <time> | --clear
```

Camera waypoints. `zoom` is 1.0 for the whole layer and 2.0 for twice as close;
`--at` is the centre of the view in layer fractions. Keys interpolate linearly
and hold flat outside their range.

```console
$ splice key main --t 0  --zoom 1.0 --at 0.5,0.5
$ splice key main --t 2.5s --zoom 1.6 --at 0.55,0.48
$ splice key main --t 5s  --zoom 2.0 --at 0.5,0.35
$ splice key main --list
{"keys":[{"t":"00:00:00.000","zoom":1.0,"at":{"x":0.5,"y":0.5}}, …]}
```

A constant camera compiles to `crop`+`scale`; a moving one compiles to
`scale`+`zoompan`. Either way the clip footage is never re-encoded: `plan` shows a
single `composite` step whose key carries the expressions.

`--place` is a **placement** key, not a camera key: it moves where a layer above
the bottom of the z order sits on the canvas, in layer-local time, and it compiles
to an `overlay` x/y expression; the bottom layer keeps its position.

`--place` on the bottom layer is refused (exit 5, `usage`): its keys never
reach the composite, so `splice key <id> --place ...` on the bottom layer
prints an error and takes no snapshot. Add a layer above it
(`splice layer add pip --z 10`) and key that, or use `--zoom`/`--at` for a
camera move, which the bottom layer does support. `splice key <id> --clear`
removes the keys and is never refused.

```console
$ splice key pip --t 0   --place 0.05,0.05
$ splice key pip --t 5s  --place 0.70,0.70
$ splice key pip --list
{"keys":[{"kind":"place","t":"00:00:00.000","place":{"x":0.05,"y":0.05}}, …]}
```

Placement animates **position only**. A layer can slide but not grow: `scale` has
no time variable, and faking a growing layer with a second mechanism would give
the document two ways to say how big something is. `place.size` stays a single
value.

## composite

With more than one layer, or a camera, `splice plan` shows one `composite <n> layers`
step; a single layer with no camera skips it entirely. Program audio comes from `z = 0`.

Clip ids are unique across the document, so `trim`, `speed`, `move`, `rm` and
`transition` find the clip in any layer and need no layer flag.
The design is [design-compositing.md](design-compositing.md).

## config

```
splice config set <provider>.<field> [value] [--stdin | --from-env <VAR>]
splice config get <provider>.<field>
splice config list
splice config path
splice config unset <provider>.<field>
```

Keys and model ids live in one per-user file — `$SPLICE_CONFIG`, else
`$XDG_CONFIG_HOME/splice/config.json` / `~/.config/splice/config.json`, else
`%APPDATA%\splice\config.json` on Windows. Never in `splice.json`: that file is
committed and pasted into transcripts.

```console
$ echo "$GEMINI_API_KEY" | splice config set gemini.api_key --stdin
{"ok":true,"provider":"gemini","field":"api_key","action":"set",
 "fingerprint":"dec5895b","path":"/home/ana/.config/splice/config.json"}

$ splice config set deepgram.model nova-3
$ splice config list
{"path":"…/config.json","exists":true,"writable":true,"providers":{
  "gemini":{"api_key":{"set":true,"source":"env","fingerprint":"dec5895b"},
            "model":{"value":"gemini-3.5-transcribe","source":"default"}},
  "deepgram":{"api_key":{"set":true,"source":"file","fingerprint":"16e78059"},
              "model":{"value":"nova-3","source":"file"}},
  "whisper":{"api_key":{"set":false,"source":null},"model":{"value":"small","source":"default"}}}}
```

`config set <provider>.program <path>` exists for whisper, the one provider that
is a program rather than an endpoint. `doctor` reports `program`, `model_file`
and a `ready` flag for it, because for whisper "set" is not "usable".

**Precedence.** A key comes from the environment first, then the file; the
environment wins because that is what CI mounts. A model comes from `--model` on
the command, then the file, then the pinned default (`gemini-3.5-transcribe`,
`nova-3`, `small`). Models have no environment variable.

**Secrets never round-trip.** There is no `--api-key` flag — argv lands in `ps`,
in shell history and in whatever an agent echoes. `--stdin` and `--from-env` are
the only ways in. `config get …api_key` returns `{set, source, fingerprint}`,
where the fingerprint is the first 8 hex of the key's SHA-256: enough to tell two
keys apart and to check whether the environment overrode the file, useless for
reconstruction. The file is written `0600` in a `0700` directory, and a loosened
file produces a `config_loose_perms` warning rather than being silently fixed.

`config path` reports `{path, exists, writable, from}` — what a container with no
writable home needs in order to understand why its key was ignored.

## history

```
splice history [--limit 20]
```

Every mutating command snapshots `splice.json` into `.splice/history/` **before** it
changes anything. Newest first:

```json
{"ok": true, "undoable": 3, "entries": [
  {"seq": 4, "at": 1794301188, "age": "just now", "command": "trim",
   "clips": 2, "overlays": 1, "captions": false, "index": false}]}
```

The last 50 are kept. `.splice/` stays cache in the sense that deleting it costs
history, never content — but it is the only copy of your undo stack, so do not
clear it reflexively mid-session.

## undo

```
splice undo [--steps 1]
```

Restores the document to how it was before the most recent mutation and drops
that snapshot, so repeating `undo` walks further back.

```console
$ splice undo
{"ok": true, "restored": [{"seq": 4, "command": "trim", "age": "just now", "clips": 2}]}
```

Two behaviours worth knowing:

- **It works on a broken document.** `undo` never parses `splice.json`; it reads
  the snapshot and replaces the file atomically. A corrupted project is one
  command from fixed.
- **No-op snapshots are skipped.** A command that failed after taking a snapshot,
  or a mutation that changed nothing, is consumed silently rather than costing
  you an undo.

`no_history` (exit 2) means there is nothing left to restore.

## update

```
splice update [--check]
```

Asks the npm registry what `latest` is, and — unless `--check` — installs it with
`npm install -g splice-cli@latest`. The binary is not replaced by this process: a
running executable cannot be overwritten on Windows, and npm already knows where the
per-platform package lives and how the skill's postinstall runs.

```json
{"ok":true,"current":"0.1.6","latest":"0.1.6","updated":false}
{"ok":true,"check":true,"current":"0.1.5","latest":"0.1.6","updated":false}
{"ok":true,"current":"0.1.5","latest":"0.1.6","updated":true,"restart":"the new version runs from the next command"}
```

- Needs `npm` on `PATH`. Without it: `update_no_npm`, exit 2, and the fix is the
  install line to run yourself.
- Only moves forward. A build newer than the registry (a local 0.2.0 against a
  published 0.1.6) reports `updated:false` rather than downgrading.
- npm's own output goes to stderr; stdout stays the single JSON object.
- Errors: `update_network` (registry unreachable or not JSON), `update_no_npm`,
  `update_failed` (npm exited non-zero, its message included).

## ffmpeg

```
splice ffmpeg install
```

Downloads and pins a static ffmpeg+ffprobe build (eugeneware/ffmpeg-static b6.1.1,
verified by sha256 of the uncompressed binaries) into the user cache, so `splice`
can run without a system ffmpeg. It is explicit by design: nothing is downloaded
behind your back, and `SPLICE_FFMPEG` still wins if you have your own build.

There are no flags today.

On success stdout is one JSON object:

```json
{"ok":true,"already_installed":false,"dir":"…","ffmpeg_sha256":"…",
 "ffprobe_sha256":"…","installed":true,"platform":"linux-x64"}
```

`dir` is the cache directory it installed into (or found), and `platform` is the
OS/arch of the pinned build. Progress lines go to stderr.

A second run is a no-op:

```json
{"ok":true,"already_installed":true,"installed":false,"dir":"…","platform":"…",…}
```

Installs to:

- Linux: `~/.cache/splice/ffmpeg`
- macOS: `~/Library/Caches/splice/ffmpeg`
- Windows: `%LOCALAPPDATA%\splice\ffmpeg`

Lookup order, so a bundled copy can win without touching `PATH`:

1. `SPLICE_FFMPEG` — a directory holding both, or a path whose sibling `ffprobe`
   is preferred
2. beside the `splice` binary
3. the splice cache (this command installs there)
4. `PATH`

To go back to the system build, delete that directory (or set `SPLICE_FFMPEG` to the binary you want); there is no `ffmpeg uninstall`.

On failure it exits 4 with one of:

| code | meaning |
| --- | --- |
| `ffmpeg_install_network` | the download failed |
| `ffmpeg_install_io` | a local filesystem operation failed: cache unwritable, path is a file, disk full |
| `ffmpeg_install_hash` | the download did not match its pinned sha256 |
| `ffmpeg_install_unsupported` | no pinned build for this OS/arch |

`splice doctor` reports `ffmpeg_source: "splice cache"` when the cache build won.

## cache

```
splice cache info
splice cache prune [--all] [--dry-run]
```

The render cache lives at `.splice/cache` and is never authoritative: deleting
it any time is safe, and the next render rebuilds whatever was deleted. Pruning
is therefore safe by construction and never deletes more than `rm -rf
.splice/cache` would.

One caveat: every `splice.json` in the same directory shares that single
`.splice/cache`, so pruning from one project deletes the other's entries too.
The cache is per directory, not per document — still safe, since nothing under
`.splice/` is authoritative, but surprising if two projects share a folder.

Cache keys embed the ffmpeg build that made them (which build, and its
encoders/filters), so pruning under a different ffmpeg — e.g. with versus
without `SPLICE_FFMPEG` — treats every entry as unreferenced and the next
render rebuilds them.

`cache info` prints what the cache holds — path, bytes, file count. It needs no
readable project: a missing directory reports zero bytes and zero files.

```json
{"ok":true,"path":"/proj/.splice/cache","bytes":2456789,"files":12}
```

`cache prune` deletes cache entries the current project's plans do not
reference. The keep set is every `plan.steps[].key` and every
`plan.steps[].sidecar.path` file stem, across four plans: Final and Preview,
each with and without the default loudness pass — delete by reference, never
by age. A `--lufs`/`--true-peak`/`--lra` variant keys differently and is not
planned, so its entry is pruned and rebuilt by the next render with those flags.
If planning fails (missing media, broken document) it deletes nothing and fails
with a `project_invalid`-class error (exit 2) whose fix is `splice cache prune --all`.

```json
{"ok":true,"removed":4,"freed_bytes":1234567,"kept_files":8,"kept_bytes":1222222}
```

`--dry-run` reports what would be deleted and deletes nothing; its numbers
equal what a real prune would do, and the output adds `"dry_run":true`.
`--all` deletes every entry, referenced or not, and needs no readable project.

## doctor

```
splice doctor
```

Read-only capability check: ffmpeg and ffprobe presence, `libx264`, `libass`,
`drawtext`, the resolved font file and **where it came from** (`fc-match`,
`SPLICE_FONT`, or a built-in OS path), the config file's path / existence /
writability, and per provider the key's state, source and fingerprint plus the
model and its source. Never prints a secret value.

```json
"providers": {"gemini": {"key_set": true, "key_source": "env",
                         "fingerprint": "dec5895b",
                         "model": "gemini-3.5-transcribe", "model_source": "default"}}
```

That replaced a bare `keys_in_env: {gemini: bool}` — a boolean could not answer
"why is it using *that* key".

`doctor` exits `4` when ffmpeg or ffprobe is missing, or when the ffmpeg found
lacks a required encoder or filter — with the install command for your OS in the
`fix` line. Reporting `ok: true` while ffmpeg was absent used to mean the failure
surfaced later, at render time, as a mystery.

The report also carries a `cache` block — `{"path": …, "bytes": …, "files": …}`,
or `null` when no project opens — and a `cache_large` warning in `warnings[]`
when the cache exceeds 1 GiB, with `fix: splice cache prune`. The cache is never
authoritative, so the warning is advice, not a limit: deleting it costs a
re-render at most.
