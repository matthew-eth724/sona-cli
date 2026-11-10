# Music bed

## Sub-features

- `audio <file> --gain <db>` — one music/voiceover bed mixed under the clips,
  `duration=first` so it never extends the program.

## How to get to it (user POV)

```bash
$SPLICE audio ./music.mp3 --gain -14      # negative needs no `--`
$SPLICE render -o out.mp4
```

## Driving it with bash + audio measurement

```bash
$SPLICE audio ./music.mp3 --gain -14
$SPLICE render -o out-full.mp4
python3 - <<'PY'
import json; d=json.load(open("splice.json")); d["tracks"]["audio"]=None
json.dump(d, open("splice-bare.json","w"))
PY
$SPLICE --project splice-bare.json render -o out-bare.mp4
ffmpeg -hide_banner -i out-full.mp4 -i out-bare.mp4 -filter_complex \
 "[0:a][1:a]amix=inputs=2:weights=1 -1:normalize=0,astats=measure_overall=RMS_level:measure_perchannel=none" \
 -f null - 2>&1 | grep 'RMS level'
# -18 dB residual == the bed is present. Compare against the bare file's own RMS.
```

## Ducking

```bash
$SPLICE duck --db -18          # envelope from the word index
$SPLICE plan | grep -o "volume='[^']*'" | head -1
$SPLICE duck --none            # back to a flat bed
```

Prove it with numbers, not by listening. A clip whose audio is silent makes the
bed the only signal, so the RMS difference is exactly the duck:

```bash
ffmpeg -hide_banner -loglevel error -y -f lavfi -i "color=c=0x223344:size=320x240:rate=25:duration=8" \
  -f lavfi -i "anullsrc=r=48000:cl=stereo:d=8" -c:v libx264 -pix_fmt yuv420p -c:a aac -shortest silent.mp4

# astats over 0.6s windows: expect ~-db^-18 during a phrase, unchanged in a gap
```

Measured: `-24.5 dB` both windows without ducking; `-42.5 dB` at the speech window
and `-24.5 dB` in a gap with `--db -18`. Eighteen decibels, exactly as asked.

## Gotchas

- Mean volume is a bad proof: a loud sine dominates the sum and the mean barely
  moves (measured: -24.0 vs -24.1 dB with the bed clearly present). Diff the
  waveforms instead.
- `--gain` is negative by convention; `allow_hyphen_values` is set for this flag
  only, so `--gain -14` works but `--gain=-14` also does.
- One bed only. A second `audio` call replaces the first.
- Duck spans are resolved when `duck` runs and stored in the project. Move the
  cut and they are stale: `duck_stale` warns, and the fix is `splice duck`.
- Every clip must come from the indexed file, or `duck` refuses — there is no
  honest mapping from one recording's words onto another's timeline.
- `--db` must be negative; the flag takes hyphen values (`--db -18`, not `-18`).
- Overlapping phrases sum and clamp, so twenty words spoken over each other still
  duck by `--db`, never more.
- The bed does not loop or trim to the timeline: `duration=first` ends it with
  the video.
