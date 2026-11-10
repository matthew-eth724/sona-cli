# Text and image overlays

## Sub-features

- `text "<content>" --from --to` with `--pos`, `--size`, `--color`.
- `image <png> --from --to` with `--pos` (including corners), `--width`,
  `--opacity`, `--margin`.
- Width and margin are stored as fractions of the canvas, so previews scale them.

## How to get to it (user POV)

```bash
$SONA text "Ship faster" --from 0.5 --to 3 --pos bottom
$SONA image logo.png --from 1 --to 5 --pos top-right --width 120 --opacity 0.9
$SONA render -o out.mp4
```

## Driving it with bash + pixels

```bash
# a logo with real alpha: black background keyed out
ffmpeg -hide_banner -loglevel error -y -f lavfi -i "color=c=black:size=200x100:rate=1:duration=1" \
  -vf "drawbox=x=0:y=0:w=200:h=100:color=red@1:t=fill,drawbox=x=140:y=20:w=40:h=60:color=yellow@1:t=fill,colorkey=black:0.01:0.0" \
  -frames:v 1 logo.png

$SONA image logo.png --from 1 --to 5 --pos top-right --width 120
$SONA render -o out.mp4
scripts/psnr-ab.sh out.mp4 out.mp4 2 "<crop of the logo region>"   # sanity
```

Prove it by cropping the same region at two timestamps: inside the window it
differs from the background, outside it is byte-identical (`average:inf`).

## Gotchas

- An image needs a PNG with an alpha channel. A fully transparent asset
  composites to nothing and looks like the overlay was ignored — check with
  `ffprobe -show_entries stream=pix_fmt` and an `alphaextract` max.
- `overlay` takes two inputs, so each image is its own chain node; the graph is
  `[base][img]overlay[out]` repeatedly, not one filter with several images.
- Images are composited **after** text and captions, so a lower third covers
  burned subtitles.
- `--width`/`--margin` are pixels at the project canvas and stored as fractions:
  a 120px logo at 640 wide becomes 90px in a 480-wide preview without being told.
- Corners matter: `--pos bottom` centres a lower third, `bottom-left` puts it at
  the margin.
- Transition and overlay both live in the decorate/assemble step, so editing
  either invalidates that one step and nothing else.
