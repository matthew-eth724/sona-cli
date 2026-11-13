# splice

A video editing CLI for AI agents. One JSON document in, ffmpeg out.

```console
$ npx splice-cli init --size 1080x1920 --fps 30
$ npx splice-cli add intro.mp4
$ npx splice-cli text "Ship faster" --from 0.5 --to 4
$ npx splice-cli render -o out.mp4
```

Every command prints exactly one JSON object with `ok` set, and every error
carries a `fix` naming the command that clears it. Captions, compositing, camera
keyframes, grading, masks and speed ramps are all there:

```console
$ npx splice-cli transcribe --attach        # whisper, Gemini or Deepgram
$ npx splice-cli layer add pip --z 10 --place 0.7,0.7,0.3
$ npx splice-cli key main --t 0 --zoom 1.0
$ npx splice-cli grade --saturation 1.3
$ npx splice-cli ramp clip_1 --from 1 --to 2 --slices 4
```

**ffmpeg is required**, and a system one can be broken: Homebrew on macOS can
install a build without libass, which cannot draw captions or titles. If `splice
doctor` reports `ffmpeg_missing` or `ffmpeg_incapable`, run:

```console
$ splice ffmpeg install       # verified static build, ~56 MB down
$ splice doctor               # ffmpeg_source: "splice cache"
```

To go back to your system build, delete that cache directory or set `SPLICE_FFMPEG`.

**ffmpeg is required and is not bundled.** `splice` needs `ffmpeg` and `ffprobe`
built with `libx264` and `libass`:

```console
$ brew install ffmpeg          # macOS
$ sudo apt install ffmpeg      # Debian/Ubuntu
$ winget install Gyan.FFmpeg   # Windows, then open a new terminal
$ splice doctor                # the first command to run anywhere new
```

Lookup is `SPLICE_FFMPEG`, then a copy sitting beside the `splice` binary, then
`PATH`, and `doctor` reports which one won. No font configuration is needed: it
tries `SPLICE_FONT`, then `fc-match`, then the fonts the OS ships.

The binary is per-platform (Linux x64/arm64, macOS x64/arm64, Windows x64) and
resolved by npm, so `npm ci --ignore-scripts` works. Linux is statically linked
and runs on Alpine too.

## The agent skill

The package ships a `splice` skill — the edit loop, the JSON contract, a command
reference and every error code with its fix — and installs it into the skill
directories this machine already has: `~/.claude/skills`, `~/.agents/skills`,
`~/.pi/agent/skills`. Nothing is created in a home directory that has never used
one of those harnesses, and a failed copy never fails the install.

```console
$ npm i -g splice-cli          # installs the skill too, where it is welcome
$ splice-skill                 # or run it yourself
$ splice-skill --dir ~/my/skills
$ SPLICE_SKILL=none npm i -g splice-cli   # skip it
```

Installed with `--ignore-scripts`? Run `npx splice-skill`. The skill asserts
0.1.4 behaviour, so its `scripts/smoke.sh` refuses to run against an older binary.

Newest work lives under `@next` until it has been run by a person on macOS and
Windows: `npx splice-cli@next --version`.

Full docs: <https://github.com/matthew-eth724/splice-cli>.
