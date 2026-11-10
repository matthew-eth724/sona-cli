# Transitions

## Sub-features

- `transition <id> --kind crossfade|dip_to_black --duration <t>`.
- `--none` restores a hard cut.
- A program with any transition assembles through an `xfade` chain instead of the
  copy-concat, and its duration shrinks by the overlaps.

## How to get to it (user POV)

```bash
$SONA add red.mp4; $SONA add green.mp4; $SONA add blue.mp4
$SONA transition clip_2 --kind crossfade --duration 600ms
$SONA transition clip_3 --kind dip_to_black --duration 600ms
$SONA list | jq -r '.duration'      # 00:00:07.800 from 3x3s clips
```

## Driving it with bash + pixels

```bash
$SONA render -o out.mp4
# sample the middle of the transition and check the colour
ffmpeg -hide_banner -loglevel error -ss 2.7 -i out.mp4 -frames:v 1 \
  -vf scale=1:1 -f rawvideo -pix_fmt rgb24 - | od -An -tu1
# red 255,0,0 -> green 0,128,0 shows a blend: 119,69,0
```

`sona plan` shows the offsets, which is the part worth checking:

```text
[0:v][1:v]xfade=transition=fade:duration=0.600000:offset=2.400000[x1]
[0:a][1:a]acrossfade=d=0.600000:c1=tri:c2=tri[a1]
[x1][2:v]xfade=transition=fadeblack:duration=0.600000:offset=4.800000[x2]
```

## Gotchas

- Duration arithmetic: offset for pair *i* is everything before it minus every
  transition so far. For 3s clips and 0.6s transitions that is 2.4 and 4.8.
- A transition replaces the `-c copy` concat with a real encode. Expect the
  assemble step to take longer, and `reused` to drop for that step only.
- The first clip cannot have one (`validate` refuses); a hand-edited project that
  tries gets an error, not a silently ignored field.
- `fadeblack` reaches its darkest around a quarter into the window in ffmpeg
  7.1, not the midpoint. The effect is right; do not assert on the midpoint.
- Adding a transition shortens `duration`. That is the overlap, not a bug.
- Audio crossfades too, so a hard cut's click disappears with one.
