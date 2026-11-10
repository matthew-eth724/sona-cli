# Music bed

## Sub-features

- `audio <file> --gain <db>` — one music/voiceover bed mixed under the clips,
  `duration=first` so it never extends the program.

## How to get to it (user POV)

```bash
$SONA audio ./music.mp3 --gain -14      # negative needs no `--`
$SONA render -o out.mp4
```

## Driving it with bash + audio measurement

```bash
$SONA audio ./music.mp3 --gain -14
$SONA render -o out-full.mp4
python3 - <<'PY'
import json; d=json.load(open("sona.json")); d["tracks"]["audio"]=None
json.dump(d, open("sona-bare.json","w"))
PY
$SONA --project sona-bare.json render -o out-bare.mp4
ffmpeg -hide_banner -i out-full.mp4 -i out-bare.mp4 -filter_complex \
 "[0:a][1:a]amix=inputs=2:weights=1 -1:normalize=0,astats=measure_overall=RMS_level:measure_perchannel=none" \
 -f null - 2>&1 | grep 'RMS level'
# -18 dB residual == the bed is present. Compare against the bare file's own RMS.
```

## Gotchas

- Mean volume is a bad proof: a loud sine dominates the sum and the mean barely
  moves (measured: -24.0 vs -24.1 dB with the bed clearly present). Diff the
  waveforms instead.
- `--gain` is negative by convention; `allow_hyphen_values` is set for this flag
  only, so `--gain -14` works but `--gain=-14` also does.
- One bed only. A second `audio` call replaces the first.
- The bed does not loop or trim to the timeline: `duration=first` ends it with
  the video.
