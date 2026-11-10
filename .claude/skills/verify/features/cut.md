# Fillers and silences

## Sub-features

- `cut <id> --fillers` — drop `um`, `uh`, `hmm` and friends.
- `cut <id> --silences <t>` — shorten every gap longer than the threshold,
  keeping `--keep` (200 ms) split at both ends.
- `--dry-run` — report the spans and time saved without editing.
- The clip is replaced by one clip per surviving span, in order.

## How to get to it (user POV)

```bash
$SONA index --from transcript.json --audio talk.mp4
$SONA cut clip_1 --silences '>0.6s' --fillers --dry-run
$SONA cut clip_1 --silences '>0.6s' --fillers
$SONA list | jq -r '.duration, (.clips | length)'
```

## Driving it with bash + JSON

A transcript with fillers, built from an SRT so it needs no network:

```bash
cat > f.srt <<'SRT'
1
00:00:01,000 --> 00:00:01,300
um,

2
00:00:01,400 --> 00:00:02,000
we ship faster.

3
00:00:02,100 --> 00:00:02,400
uh

4
00:00:02,500 --> 00:00:03,500
today.
SRT

$SONA index --from f.srt --audio clip-a.mp4
$SONA cut clip_1 --fillers --dry-run | jq '.removed, .saved'
# two spans, "saved": "00:00:01.080"

$SONA cut clip_1 --fillers | jq '.replaced_by, .saved'
$SONA render -o out.mp4
```

Assert: `saved` equals the sum of the removed spans; `replaced_by` has one id per
kept span; the rendered duration matches the reported one, not the original.

## Gotchas

- `--silences` **shortens** a gap to `--keep`, it does not delete it. A 2 s gap at
  `--keep 200ms` leaves 200 ms of air. `--keep 0` removes the whole gap.
- Leading and trailing silence are shortened too, so a clip usually loses a
  little length even with no fillers.
- Run `--dry-run` first. A cut is destructive: the original clip id is reused by
  the first surviving span, so the timeline no longer means what it did.
- Cutting removes time, so captions and overlays timed against the old timeline
  are now offset. `cut_shifts_layers` warns. Caption after cutting, not before.
- Kept spans under 120 ms are dropped as clicks, so `saved` can be slightly less
  than the original duration minus the kept spans.
- The filler list is conservative on purpose. Editing it is a one-line change to
  `index::FILLERS`, and the tests pin the merge behaviour around it.
