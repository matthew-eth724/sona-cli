# Command surface

Every command prints **one JSON object** on stdout. `view` and `find` print plain
text unless `--json`. `render` additionally writes progress objects to stderr.

`--project <path>` is global (default `splice.json`). Flags below are the ones an
agent actually needs; `splice <verb> --help` is authoritative for the rest.

## Setup

| command | what it does | key flags |
| --- | --- | --- |
| `init` | Create `splice.json` | `--size WxH`, `--fps N` |
| `doctor` | ffmpeg, fonts, keys; exits 4 if unusable | — |
| `config set <provider>.<field>` | per-user config, never in the project | `--stdin`, `--from-env` |
| `config list` | which keys are set, from where, and the model | — |

Secrets never go in `splice.json` and never in argv: `splice config set
gemini.api_key --stdin`, or an environment variable, which wins over the file.

## Timeline

| command | what it does | notes |
| --- | --- | --- |
| `add <file…>` | append clips | `--layer <id>`, `--at <index>` |
| `list` | the timeline as JSON | cheap; `missing` flags dead sources |
| `trim <id>` | in/out points | `--from`, `--to`, `--words w12-w19`, `--pad`, `--dry-run`; clamps past the medium with `clip_past_media` |
| `speed <id> <factor>` | constant rate, pitch preserved | `atempo` stays inside its documented range by chaining |
| `ramp <id> --from --to --slices` | speed change over the clip | cuts the source at keys, one constant `atempo` per slice |
| `move <id> <index>` | reorder | |
| `rm <id>` | remove | |
| `inspect` | summary + diagnostics | `warnings[]` here is the same as on mutations |

## Picture

| command | what it does |
| --- | --- |
| `text "<s>"` | title overlay; `--from`, `--to`, `--pos top-left/…`, `--size`, colour flags |
| `image <file>` | logo / lower third at a placement over a time range |
| `transition <id>` | `--kind fade/…`, `--duration` |
| `layer add <id> --z N [--place x,y,size]` | add a video layer |
| `layer set <id>` | `--z`, `--start`, `--place x,y,size`; refused on the bottom layer for position/start |
| `key <layer> --t <s> --zoom <z> --at x,y` | camera keyframe |
| `key <layer> --t <s> --place x,y` | slide a layer; refused on the bottom layer |
| `mask <layer> --chroma <colour> \| --vignette` | per-layer mask, applied before placement |
| `grade` | `--brightness`, `--contrast`, `--saturation`, `--gamma`, `--lut`; one `eq`/`lut3d` before text |

The **bottom layer of the z order** is the program frame: its `place`,
`place_keys` and `start` are reported, not applied, and edits to them exit 5.
The layer at `z = 0` carries the program audio and must fill the canvas.

## Sound and captions

| command | what it does |
| --- | --- |
| `audio <file>` | music bed under the clips |
| `duck` | duck the bed under speech using the word index |
| `caption <file.srt>` | burn an SRT in |
| `transcribe` | create `captions.srt` beside the project; `--attach` to burn, `--provider`, `--srt`, `--yes` past `--max-duration` |
| `index` | word index for a source (cached by audio hash) |
| `view` / `find` | read the index; plain text, `--json` for structure |
| `scenes` | shot boundaries into the index |
| `cut` | remove fillers and long silences, splitting the clip |

## Output

| command | what it does |
| --- | --- |
| `plan` | print the ffmpeg argv a render would run, without running it |
| `render` | `-o out.mp4`, `--preview`, `--loudnorm` (`--lufs`, `--true-peak`, `--lra`), `--timeout`, `--progress` / `--no-progress` |
| `frames` | `--at 1,3.5`, `-o dir`; `--sheet` for a contact sheet |
| `history` | snapshots taken before each mutation |
| `undo` | `--steps K` restores K mutations back |
| `cache info` / `cache prune [--all] [--dry-run]` | inspect and reclaim `.splice/cache` |

`render` reports `{"ok":true,"duration":…,"fps":…,"size":…,"steps":{"total":N,"ran":R,"reused":U},"out":…,"loudness":…}`.
`frames` reports the paths it wrote. `plan` reports the steps it would run.

## JSON shapes worth knowing

```json
{"ok":true,"added":[{"id":"clip_1","source":"media/a.mp4","duration":"00:00:06.000"}],"duration":"00:00:06.000"}
{"ok":true,"changed":["clip_1"],"duration":"00:00:05.000","invalidated":2}
{"ok":true,"clips":[{"id":"clip_1","source":"media/a.mp4","in":"00:00:00.000","out":null,"speed":1.0,"missing":false}]}
{"ok":false,"error":{"code":"media_missing","message":"…","fix":"splice list   (shows clips whose source is gone)"}}
```

`warnings[]` appears when something succeeded but will not do what was asked:

```json
{"at":"tracks.layers[pip]","code":"layer_audio_dropped","fix":"move those clips to the main layer, or accept the silence","message":"layer pip has audio, and only the layer at z = 0 is heard"}
```
