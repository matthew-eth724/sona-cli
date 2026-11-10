# splice

A video editing CLI built for AI agents. Describe an edit in a small JSON
project file, render it with ffmpeg, look at the result as PNGs.

```console
$ splice init --size 1080x1920 --fps 30
$ splice add intro.mp4
$ splice add demo.webm --from 12s --to 28s --speed 1.5
$ splice index --from transcript.json --audio podcast.mp4
$ splice find "ship faster"            # 04:12.0-04:15.4  w118-w126  … [ship faster] …
$ splice add podcast.mp4 --words w118-w126
$ splice cut clip_1 --fillers --silences '>0.6s'   # drop the ums and the dead air
$ splice scenes                                     # shot boundaries, into the index
$ splice text "Ship faster" --from 0.5 --to 4 --pos bottom
$ splice caption subs.srt
$ splice render --preview                # cheap proxy, own cache
$ splice render -o out.mp4
{"duration":"00:00:42.500","ok":true,"out":"out.mp4","steps":{"ran":4,"reused":0,"total":4}}
```

Every command prints exactly one JSON object. Nothing else goes to stdout. Edits
are cheap because each step of the render is cached by content hash: change one
clip and only its normalize step plus the assemble steps re-run; change only a
text overlay and only the assemble step does.

## Why another ffmpeg wrapper

An agent editing video has three handicaps a human editor does not: it pays
tokens for everything it reads and writes, it cannot watch a timeline, and it
debugs by reading text.

So `splice` is a compiler, not a flag translator:

- **The project is one diffable file.** An edit is a small `jq` or CLI change,
  not a re-described job spec.
- **The filter graph is generated, never hand-written.** Source heterogenity
  (fps, SAR, rotation, VFR, channel count) is normalized per clip before
  anything is assembled.
- **Failures are one line with a fix.** `{"error":{"code":"media_missing",
  "message":"…","fix":"…"}}`, not a 200-line stderr dump.
- **`splice plan` shows the exact ffmpeg argv**, so a wrong render is debuggable in
  seconds without reading vendor docs.
- **`splice frames` gives the agent eyes** — stills and contact sheets sized for a
  vision model.
- **`splice render --preview` makes looking cheap.** A 480x854 proxy with its own
  cache keys: 1.6 s against 7.1 s for the same 20-second clip, and switching
  back to final costs nothing.
- **`--dry-run` on every mutation** reports the resulting duration and warnings
  while writing nothing, so a risky edit costs one round trip, not a rollback.
- **`splice undo` exists because agents make mistakes.** Every mutation snapshots
  `splice.json` first, and undo restores it without parsing the file, so a
  corrupted project is one command from fixed.
- **`splice view` and `splice find` make reading cheap.** An indexed transcript
  prints as a handful of text lines with stable word ids (`w1`, `w2`, …), so
  finding a cut point costs a few hundred bytes instead of a timeline walk.

## Install

Requires `ffmpeg` and `ffprobe` on `PATH`, built with `libx264`, `libass`
(subtitles) and `freetype` (drawtext). ffmpeg 7.x is what it is tested against.

```console
$ cargo build --release
$ ./target/release/splice doctor
{"encoders":{"libx264":true},"ffmpeg":"ffmpeg version 7.1.5 …",
 "filters":{"drawtext":true,"libass":true},
 "font":"/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf","ok":true}
```

Either way, **ffmpeg is required and is not bundled**: `splice` shells out to
`ffmpeg`, `ffprobe` and `fc-match`, and the build must have `libx264`, `libass`
and `freetype`. `brew install ffmpeg`, `apt install ffmpeg`, or
`winget install Gyan.FFmpeg`. ffmpeg 7.x is what it is tested against.

`doctor` is the first command to run anywhere new: it reports which encoders and
filters exist, which font `drawtext` will use, and whether ASR keys are visible.

## Commands

| | |
| --- | --- |
| setup | `init` `doctor` |
| timeline | `add` `list` `trim` `speed` `move` `rm` `inspect` |
| layers | `text` `image` `audio` `caption` |
| joining | `transition` |
| reading | `index` `view` `find` `scenes` |
| tighten | `cut` |
| output | `plan` `render` `frames` |
| safety | `history` `undo` |

Full reference: [docs/cli.md](docs/cli.md).

## Contracts

**stdout** is one JSON object, always. Machine-readable by default; there is no
`--json` flag to forget.

**Exit codes** classify the failure for a caller that only sees the code:

| code | meaning |
| --- | --- |
| 0 | success |
| 2 | project or precondition invalid (bad edit, missing cues, ASR refused) |
| 3 | media missing or unreadable |
| 4 | ffmpeg failed |
| 5 | bad usage |

**Warnings never block.** Anything that would still produce a valid file is
reported in `warnings[]` on the command that caused it — an over-long trim, text
past the end of the timeline, a source with no video stream.

## Files

```
splice.json          the project: canvas, clips, overlays, captions
.splice/cache/       content-addressed render steps (safe to delete)
.splice/preview/     default output for `splice frames`
out.mp4          default render target
```

`splice.json` is the only state. `.splice/` is pure cache; deleting it costs a re-render,
never information.

## Documentation

| | |
| --- | --- |
| [docs/cli.md](docs/cli.md) | every command, flag, JSON shape and failure |
| [docs/project-file.md](docs/project-file.md) | `splice.json` field reference and invariants |
| [docs/for-agents.md](docs/for-agents.md) | how an agent should drive it |
| [docs/troubleshooting.md](docs/troubleshooting.md) | error code → cause → fix |
| [docs/design.md](docs/design.md) | why it is shaped this way |
| [docs/design-compositing.md](docs/design-compositing.md) | the compositor and keyframes design |

The verification skill (launch, doctor, drive, capture, cleanup, six-feature map)
ships twice: `.claude/skills/verify/` for Claude Code and
`.agents/skills/verify/` for harnesses that read `.agents/`. The two trees are
identical except for their own script paths; `tools/check-skill-mirror.sh` fails
when they drift.

Not implemented yet, and not documented as if it were: automatic transcription,
image overlays, transitions between clips, multi-track compositing, keyframes.
