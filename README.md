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
$ splice duck --db -18                              # lower the bed under speech
$ splice text "Ship faster" --from 0.5 --to 4 --pos bottom
$ splice caption subs.srt
$ splice render --preview                # cheap proxy, own cache
$ splice render --loudnorm -o out.mp4
{"duration":"00:00:42.500","ok":true,"out":"out.mp4","steps":{"ran":5,"reused":0,"total":5}}
```

Edits are cheap because every render step is cached by content hash: change one
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
  (fps, SAR, rotation, VFR, channel count) is normalized per clip before anything
  is assembled.
- **Failures are one line with a fix.** Every warning and error carries the
  command that clears it: `{"code":"clip_past_media","message":"…",
  "fix":"splice trim clip_1 --to 00:00:09.000"}`.
- **`splice plan` shows the exact ffmpeg argv**, so a wrong render is debuggable in
  seconds without reading vendor docs.
- **`splice frames` gives the agent eyes** — stills and contact sheets sized for a
  vision model.
- **`splice render --preview` makes looking cheap.** A proxy with its own cache
  keys: 16 s against 38 s for the same 8-minute source, and switching back to
  final costs nothing.
- **`splice view` and `splice find` make reading cheap.** An indexed transcript
  prints as a few lines with stable word ids (`w1`, `w2`, …), so finding a cut
  point costs bytes instead of a timeline walk.
- **`--dry-run` on every mutation** reports the resulting duration and warnings
  while writing nothing, so a risky edit costs one round trip, not a rollback.
- **`splice undo` exists because agents make mistakes.** Every mutation snapshots
  `splice.json` first, and undo restores it without parsing the file, so a
  corrupted project is one command from fixed.

## Install

```console
$ npm install -g splice-cli
$ splice doctor
```

Per-platform binaries (Linux x64/arm64, macOS x64/arm64, Windows x64) resolved by
npm as optional dependencies — nothing is downloaded at install time, and
`npm ci --ignore-scripts` still works. Linux builds are static, so they run on
Alpine as well as Debian.

**ffmpeg is required and is not bundled.** `splice` shells out to `ffmpeg` and
`ffprobe`, and the build must have `libx264`, `libass` and `freetype`:

```console
$ brew install ffmpeg            # macOS
$ sudo apt install ffmpeg        # Debian/Ubuntu
$ winget install Gyan.FFmpeg     # Windows, then open a new terminal
```

Fonts need no config: lookup tries `SPLICE_FONT`, then `fc-match`, then fonts the
OS already ships (`C:\Windows\Fonts\segoeui.ttf`, `/System/Library/Fonts/…`,
dejavu noto liberation on Linux). `doctor` reports which source won.

From source, or to hack on it:

```console
$ cargo build --release
$ ./target/release/splice doctor
```

`doctor` is the first command to run anywhere new. It reports ffmpeg and ffprobe,
which encoders and filters exist, the resolved font and where it came from, and
whether ASR keys are visible — and **exits 4** if ffmpeg is missing or lacks a
required encoder, rather than letting you find out at render time.

## Commands

| | |
| --- | --- |
| setup | `init` `doctor` |
| timeline | `add` `list` `trim` `speed` `move` `rm` `inspect` |
| picture | `text` `image` `transition` |
| sound | `audio` `duck` `caption` |
| read | `index` `view` `find` `scenes` |
| tighten | `cut` |
| output | `plan` `render` `frames` |
| safety | `history` `undo` |

Full reference: [docs/cli.md](docs/cli.md).

## Contracts

**stdout is one JSON object**, for every command except two. `view` and `find`
print plain text lines — that is what makes them cheap to read (970 bytes against
2,275 for the same thirty seconds); pass `--json` when a caller needs structure.
Errors are always JSON.

**Exit codes** classify the failure for a caller that only sees the code:

| code | meaning |
| --- | --- |
| 0 | success |
| 2 | project or precondition invalid (bad edit, missing cues, ASR refused) |
| 3 | media missing or unreadable |
| 4 | ffmpeg failed, or not installed / not capable |
| 5 | bad usage |

**Warnings never block.** Anything that would still produce a valid file is
reported in `warnings[]` on the command that caused it — an over-long trim, text
past the end of the timeline, a source with no video stream — each with a `fix`.

## Files

```
splice.json          the project (document version 2: layers, clips, overlays, captions, index)
.splice/cache/       content-addressed render steps, safe to delete
.splice/history/     snapshots behind undo, the one thing you cannot re-derive
.splice/index/       word index and detected scenes, keyed by audio hash
.splice/preview/     stills from the final render
.splice/preview-proxy/  stills from the proxy
out.mp4              default render target
```

`splice.json` is the only state. Everything under `.splice/` is derived; deleting
it costs a re-render and an undo stack, never content.

## Documentation

| | |
| --- | --- |
| [docs/cli.md](docs/cli.md) | every command, flag, JSON shape and failure |
| [docs/project-file.md](docs/project-file.md) | `splice.json` field reference and invariants |
| [docs/for-agents.md](docs/for-agents.md) | how an agent should drive it |
| [docs/troubleshooting.md](docs/troubleshooting.md) | error code → cause → fix |
| [docs/design.md](docs/design.md) | why it is shaped this way |
| [docs/design-compositing.md](docs/design-compositing.md) | the compositor and keyframes design |
| [docs/design-portable.md](docs/design-portable.md) | bundling ffmpeg and fonts so one install needs nothing |

The verification skill (launch, doctor, drive, capture, cleanup, eleven-feature map)
ships twice: `.claude/skills/verify/` for Claude Code and `.agents/skills/verify/`
for harnesses that read `.agents/`. The trees are identical except for their own
script paths; `tools/check-skill-mirror.sh` fails when they drift.

## Status

66 unit tests, and every command above has been driven end to end against real
footage — including an 8-minute 4:3 source at 23.976 fps, which is where the
retiming-order and preview-encoder bugs were found.

**Not implemented, and not documented as if it were:** automatic transcription
(the index reads a transcript you provide; the provider formats are pinned from
live Gemini and Deepgram responses, but `splice` does not call them yet),
multi-track compositing and keyframes (the document models layers and camera keys
and refuses to render them — see `design-compositing.md`), and bundled ffmpeg
(see `design-portable.md`).

## Licence

MIT. See [LICENSE](LICENSE).
