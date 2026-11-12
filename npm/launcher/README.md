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

Newest work lives under `@next` until it has been run by a person on macOS and
Windows: `npx splice-cli@next --version`.

Full docs: <https://github.com/matthew-eth724/splice-cli>.
