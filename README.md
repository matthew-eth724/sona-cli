# splice

A video editing CLI built for AI agents. Describe an edit in a small JSON
project file, render it with ffmpeg, look at the result as PNGs.

```console
$ splice init --size 1080x1920 --fps 30
$ splice add intro.mp4
$ splice add demo.webm --from 12s --to 28s --speed 1.5
$ splice transcribe                      # calls the provider, writes captions.srt
$ splice find "ship faster"            # 04:12.0-04:15.4  w118-w126  … [ship faster] …
$ splice add podcast.mp4 --words w118-w126
$ splice cut clip_1 --fillers --silences '>0.6s'   # drop the ums and the dead air
$ splice duck --db -18                              # lower the bed under speech
$ splice layer add pip --z 10 --place 0.62,0.62,0.3 --start 4s   # picture-in-picture
$ splice key main --t 0 --zoom 1.0 --at 0.5,0.5                   # slow push-in
$ splice key main --t 8s --zoom 1.8 --at 0.5,0.45
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

Newest work ships under the `next` label until someone has run it on macOS and
Windows: `npm install -g splice-cli@next`. Promotion to default is one command
(`npm dist-tag add splice-cli@<version> latest`).

Per-platform binaries (Linux x64/arm64, macOS x64/arm64, Windows x64) resolved by
npm as optional dependencies — nothing is downloaded at install time, and
`npm ci --ignore-scripts` still works. Linux builds are static, so they run on
Alpine as well as Debian.

Add a key and pick a model once, then `splice transcribe` writes `captions.srt`
(attaching it for burn-in is a separate step — `splice caption captions.srt`, or
`--attach`):

```console
$ echo "$GEMINI_API_KEY" | splice config set gemini.api_key --stdin
$ splice config set deepgram.model nova-3
$ splice config list          # set?, from where, and which model wins — never the value
```

**ffmpeg is required and is not bundled yet.** `splice` shells out to `ffmpeg` and
`ffprobe`, and the build must have `libx264` and `libass` — `drawtext` and
freetype are no longer needed, because titles burn through the same ASS filter
captions do:

```console
$ brew install ffmpeg            # macOS
$ sudo apt install ffmpeg        # Debian/Ubuntu
$ winget install Gyan.FFmpeg     # Windows, then open a new terminal
$ ffmpeg -filters | grep -w subtitles   # must print a line: that is libass
```

If your system ffmpeg turns out to be missing libass, or you would rather not
depend on it at all:

```console
$ splice ffmpeg install        # verified static build, ~56 MB download, ~160 MB on disk, into the user cache
$ splice doctor                # ffmpeg_source: "splice cache"
```

It is explicit, never a postinstall: nothing is downloaded behind your back, and
`SPLICE_FFMPEG` still wins if you have your own build.

`brew install ffmpeg` can install a build **without libass** — GitHub's macOS
runners do exactly that (ffmpeg 9.0.2, no `ass` or `subtitles` filter), and a
distro bottle can too. Titles and captions are drawn by libass, so on such a
machine every render with a caption fails with `No such filter: 'subtitles'`
while everything else works. `splice doctor` reports it as `ffmpeg_incapable`,
and the check above is what tells you before a render does.

Resolution order, so a bundled copy can win without touching `PATH`:
`SPLICE_FFMPEG` (a directory holding both, or a path whose sibling `ffprobe` is
preferred), then a sibling of the `splice` binary, then `PATH`. `doctor` names the
file it chose and which rule chose it.

Fonts need no config: lookup tries `SPLICE_FONT`, then `fc-match`, then fonts the
OS already ships (`C:\Windows\Fonts\segoeui.ttf`, `/System/Library/Fonts/…`,
dejavu noto liberation on Linux). `doctor` reports which source won.

A release ships under the `next` tag and is promoted to `latest` deliberately.
`splice update` asks the registry and, with npm, installs the newest version;
`splice update --check` only reports:

```console
$ splice update --check
{"ok":true,"check":true,"current":"0.1.5","latest":"0.1.6","updated":false}
```

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
| timeline | `add` `list` `trim` `speed` `ramp` `move` `rm` `inspect` |
| layers | `layer` `key` |
| picture | `text` `image` `transition` `mask` `grade` |
| sound | `audio` `duck` `caption` |
| read | `transcribe` `index` `view` `find` `scenes` |
| tighten | `cut` |
| output | `plan` `render` `frames` `cache` |
| safety | `history` `undo` |

Full reference: [docs/cli.md](docs/cli.md).

## Contracts

**stdout is one JSON object**, for every command except two. `view` and `find`
print plain text lines — that is what makes them cheap to read (970 bytes against
2,275 for the same thirty seconds); pass `--json` when a caller needs structure.
`render` writes one JSON progress object per line to stderr by default
(`--no-progress` silences it); stdout is still exactly one object. Errors are
always JSON.

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

Two skills ship in the repo, each twice — `.claude/skills/<name>/` for Claude Code
and `.agents/skills/<name>/` for harnesses that read `.agents/`. The trees are
identical except for their own script paths; `tools/check-skill-mirror.sh` fails
when they drift.

- **`verify`** — for working on splice: launch, doctor, drive, capture, cleanup, and
  an eleven-feature map.
- **`splice`** — for *using* splice, from any project: the edit loop, the JSON
  contract, where paths and layers surprise people, a command reference, an
  error-code table, and `scripts/smoke.sh`, which proves an install end to end in a
  throwaway directory and refuses to run against a binary older than 0.1.4.

The `splice` skill also ships **inside the npm package** and is installed by the
launcher's postinstall into whichever of `~/.claude/skills`, `~/.agents/skills` and
`~/.pi/agent/skills` already exist — never creating one, and never failing an
install over a copy. `splice-skill` runs the same installer by hand (needed after
`--ignore-scripts`, and `--dir <path>` puts it elsewhere); `SPLICE_SKILL=none`
skips it. The package copy is generated at publish time from `.claude/skills/splice`
by `npm/bundle-skill.mjs`, so the two cannot drift.

## Status

144 unit tests, and every command above has been driven end to end against real
footage — including an 8-minute 4:3 source at 23.976 fps, which is where the
retiming-order and preview-encoder bugs were found.

Implemented and proven: transcription through Gemini, Deepgram and whisper.cpp;
multi-track compositing with camera keyframes, sliding placement, masks and
grading; speed ramps; titles and captions on one ASS engine.

**ffmpeg is not bundled, deliberately.** `splice ffmpeg install` fetches a
pinned, hash-verified static build (with libass) into the user cache, and the
lookup prefers it over `PATH`; that covers the case bundling existed for, without
~11 MB per platform package and two build recipes nobody here can test. Bundling
is set aside until someone needs an install with no network at all. The measured
alternative is in [docs/design-portable.md](docs/design-portable.md).

## Licence

MIT. See [LICENSE](LICENSE).
