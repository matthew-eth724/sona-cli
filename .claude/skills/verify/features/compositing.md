# Layers and camera

## Sub-features

- `layer add|set|rm|list` — simultaneous pictures with a stacking order, a
  placement rectangle and a start time.
- `key <layer> --t --zoom --at` — camera waypoints; `--list`, `--rm`, `--clear`.
- `add --layer <id>` — put a clip on a layer.
- The render gains one `composite` step; clip renders are untouched by placement
  or camera changes.

## How to get to it (user POV)

```bash
$SONA init --size 640x360 --fps 25
$SONA add bg.mp4
$SONA layer add pip --z 10 --place 0.6,0.6,0.3 --start 1s
$SONA add pip.mp4 --layer pip
$SONA key main --t 0 --zoom 1.0 --at 0.5,0.5
$SONA key main --t 5s --zoom 2.0 --at 0.5,0.35
$SONA render -o out.mp4
```

## Driving it with bash + pixels

Assert the window, not the exit code: the PiP should differ from a single-layer
render inside its window and match outside it.

```bash
$SONA plan | jq -r '.steps[].label'          # normalize, normalize, composite
$SONA render -o out.mp4

# the same project without the pip layer
python3 -c "import json;d=json.load(open('sona.json'));d['tracks']['layers']=[l for l in d['tracks']['layers'] if l['id']=='main'];json.dump(d,open('ref.json','w'))"
$SONA --project ref.json render -o base.mp4
scripts/psnr-ab.sh out.mp4 base.mp4 2.0 "192:192:384:216"     # ~5 dB: PiP present
scripts/psnr-ab.sh out.mp4 base.mp4 0.5 "192:192:384:216"     # ~35 dB: before it starts
```

For the camera, **use a still source**. A moving test pattern makes every frame
differ from every other, so timing offsets swamp the zoom and a wrong zoom looks
like a right one:

```bash
ffmpeg -y -f lavfi -i "testsrc2=size=640x360:rate=25:duration=0.04" -frames:v 1 still.png
ffmpeg -y -loop 1 -i still.png -t 6 -r 25 -i anullsrc -c:v libx264 -c:a aac -shortest src.mp4
# then compare frames at 0.1 / 2.5 / 5.0 against crops at 1.0 / 1.5 / 2.0
```

## Gotchas

- `zoompan`'s `z` **is** the displayed zoom. Multiplying it by the pre-scale
  factor doubles the zoom at every key — a bug that shipped into this feature once.
- A moving `testsrc2` cannot verify a camera. Use a still frame.
- `z` must be unique; `z = 0` carries the program audio and must fill the canvas.
- Only the layer at `z = 0` is heard. An overlay layer with audio warns
  `layer_audio_dropped` instead of dropping it silently.
- Placement animates position only (`key --place`), never size. Compute the
  expected x from the keys and diff that box; a box where the layer used to be
  should be identical to a single-layer render by then.
- A camera change moves the `composite` key only; the clip renders are reused.
