# Shot boundaries

## Sub-features

- `scenes` — detect cuts with ffmpeg's `scene` score, merge anything under
  `--min`, write `{start, end, keyframe}` into the index.
- `--threshold` (default 0.4) and `--min` (default 500ms) are part of the cache
  key, so changing either re-detects.
- `--json` returns the scenes as objects.

## How to get to it (user POV)

```bash
$SPLICE index --from t.srt --audio shots.mp4
$SPLICE scenes
00:00.0-00:02.0  key 00:00.2  scene 1
00:02.0-00:04.0  key 00:02.2  scene 2
$SPLICE scenes --json | jq '.cached, (.scenes | length)'
```

## Driving it with bash + JSON

Media with known cuts, built from three solid colours:

```bash
ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i "color=c=red:size=320x240:rate=25:duration=2" \
  -f lavfi -i "color=c=blue:size=320x240:rate=25:duration=2" \
  -f lavfi -i "color=c=green:size=320x240:rate=25:duration=2" \
  -f lavfi -i "sine=frequency=440:duration=6" \
  -filter_complex "[0:v][1:v][2:v]concat=n=3:v=1:a=0[v]" \
  -map "[v]" -map 3:a -c:v libx264 -pix_fmt yuv420p -c:a aac shots.mp4

printf '1\n00:00:00,000 --> 00:00:06,000\nsix seconds.\n' > t.srt
$SPLICE init --size 320x240 --fps 25
$SPLICE index --from t.srt --audio shots.mp4
$SPLICE scenes | wc -l          # 3
```

A 0.2s flash between two shots is the merge test: default `--min 500ms` absorbs
it (2 scenes), `--min 100ms` keeps it (3 scenes).

## Gotchas

- Needs an index first (`no_index`), because that is where scenes are stored.
- Scenes are cached against the source's size+mtime plus threshold and `--min`.
  `touch` re-detects; that is the staleness check working.
- `key` is a timestamp inside the shot (10% in, capped at 300ms), not the first
  frame: the first frame of a cut is often a flash or a blur.
- A single long take legitimately yields one scene. Empty output would be a bug;
  one line is the answer.
- Detection shells out to ffmpeg and reads `showinfo` from stderr, so it costs a
  full decode. It is cached for that reason.
- Scenes describe the picture but live in the audio-keyed index. The recorded
  `source_stat` is what keeps that honest.
