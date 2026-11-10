# sona

A video editing CLI for AI agents. One JSON document in, ffmpeg out.

```console
$ npx @matthew-eth724/sona init --size 1080x1920 --fps 30
$ npx @matthew-eth724/sona add intro.mp4
$ npx @matthew-eth724/sona text "Ship faster" --from 0.5 --to 4
$ npx @matthew-eth724/sona render -o out.mp4
```

**ffmpeg is required.** `sona` shells out to `ffmpeg`, `ffprobe` and `fc-match`
(fontconfig). It does not bundle them, because the build must have `libx264`,
`libass` and `freetype`:

```console
$ brew install ffmpeg          # macOS
$ sudo apt install ffmpeg      # Debian/Ubuntu
$ winget install Gyan.FFmpeg   # Windows
$ sona doctor                  # the first command to run anywhere new
```

`doctor` reports which encoders and filters exist, and which font `drawtext` will
use, before a render fails on one.

The binary is per-platform and resolved by npm, so `npm ci --ignore-scripts`
works. Full docs: <https://github.com/matthew-eth724/sona-cli>.
