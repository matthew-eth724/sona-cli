# Text overlays

## Sub-features

- `text "<content>" --from --to` with `--pos top|center|bottom`, `--size`,
  `--color`. Rendered by ffmpeg `drawtext` with a boxed background.

## How to get to it (user POV)

```bash
$VA text "Ship faster" --from 0.5 --to 3 --pos bottom
$VA render -o out.mp4
```

## Driving it with bash + pixels

```bash
$VA text "Ship faster" --from 0.5 --to 3 --pos bottom
$VA render -o out-with.mp4
python3 - <<'PY'
import json; d=json.load(open("va.json")); d["tracks"]["overlays"]=[]
json.dump(d, open("va-bare.json","w"))
PY
$VA --project va-bare.json render -o out-bare.mp4
.claude/skills/verify/scripts/psnr-ab.sh out-with.mp4 out-bare.mp4 1.5 "1280:120:0:520"
# average:15.9   -> text is in the pixels
.claude/skills/verify/scripts/psnr-ab.sh out-with.mp4 out-bare.mp4 4.0 "1280:120:0:520"
# average:40+    -> outside the window, nothing changed
```

## Gotchas

- Proof requires two renders of the *same* timestamp and crop. Comparing frame
  N of clip A against frame M of clip B proves nothing.
- `--to` past the timeline end is a `text_past_end` warning, not an error.
- Text needs a resolvable font; `va doctor` prints it. A null font makes the
  render fail with `no_font`.
- Overlay text is escaped by `paths::text_escape`; `:` `,` `'` `%` in content are
  safe, but do not hand-write drawtext strings elsewhere.
