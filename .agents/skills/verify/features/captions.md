# Captions

## Sub-features

- `caption <file.srt>` — attaches an SRT, parsed for cue count and burned by
  libass in the assemble pass.

## How to get to it (user POV)

```bash
$SPLICE caption ./subs.srt        # {"changed":["captions"],"cues":2}
$SPLICE render -o out.mp4
```

## Driving it with bash + pixels

```bash
$SPLICE caption ./subs.srt
$SPLICE render -o out-subs.mp4
python3 - <<'PY'
import json; d=json.load(open("splice.json")); d["tracks"]["captions"]=None
json.dump(d, open("splice-bare.json","w"))
PY
$SPLICE --project splice-bare.json render -o out-bare.mp4
.agents/skills/verify/scripts/psnr-ab.sh out-subs.mp4 out-bare.mp4 1.5 "1280:120:0:520"
```

## Gotchas

- The `subtitles` filter reads the SRT by path; it is **not** an `-i` input.
  Adding it as one shifts the music bed's index and breaks the audio graph.
- SRT timestamps use a comma (`00:00:01,500`); the parser accepts comma or dot.
- Captions burn into pixels, so they cannot be turned off after encoding.
- Cue count comes from `caption`, not from ffmpeg; a wrong count means the SRT
  parse, not libass.
