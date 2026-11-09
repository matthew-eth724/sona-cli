# Review frames

## Sub-features

- `frames --at <times>` — one PNG per timestamp, scaled to 768px wide.
- `frames --sheet <n>` — an n-tile contact sheet with a burnt-in timecode.
- Times accept `2s`, `1:02`, `00:00:04.500`, and `25%` of the timeline.

## How to get to it (user POV)

```bash
$VA frames --at 1,3 --sheet 8 -o .va/preview
# {"frames":[{"path":".../at-00001-000.png"}, ..., {"kind":"sheet","path":".../sheet.png"}]}
```

## Driving it with bash + JSON

```bash
out=$($VA frames --at 1,3 --sheet 8 -o .va/preview)
echo "$out" | jq -r '.frames[].path' | while read -r p; do
  [ -s "$p" ] && echo "ok $p" || echo "MISSING $p"
done
```

## Gotchas

- Frames come from the **last plan step's cached output**, so `va render` must
  run first; otherwise expect `nothing rendered yet`.
- Stills are re-encoded at 768px wide for vision-model token cost; do not use
  them to judge pixel-exact quality.
- `--at` and `--sheet` can be combined; passing neither is a usage error.
