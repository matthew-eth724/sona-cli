# For agents

How to drive `va` without wasting tokens or trusting yourself.

## The loop

```
doctor → inspect → edit → render → frames → look
```

1. `va doctor` once per machine. If `libx264`, `libass` or `drawtext` is false,
   or `font` is null, stop: every render will fail on a filter, not on your edit.
2. `va inspect` before editing. It is cheap and returns `duration`, the clip
   list and `warnings[]`.
3. Make the smallest edit that expresses the change. One command, one concept.
4. `va render -o out.mp4`. Read `steps.ran` / `steps.reused`.
5. `va frames --at <times>` when the change is visual, and actually look at the
   PNGs.

## Rules

**Read the JSON, never the logs.** stdout is one object. stderr is a repeat of
the failure, at most three lines. There is nothing else to scrape.

**Read `warnings[]` on every mutating command.** An over-long trim, an overlay
past the end, a source without video — all succeed with a warning. A warning
ignored at edit time becomes a mystery at render time.

**Never fabricate a timestamp.** Take durations from `list`/`inspect`, not from
memory. `00:00:08.333` is what the tool computed; `8.3` is a guess that will
offend the caption burn.

**Verify with pixels, not with `ok: true`.** `render` exiting 0 proves ffmpeg
ran. To prove a text overlay is on screen, diff the same frame of a render with
and without it — see below.

**Trust the cache counters.**
- Cold render: `ran == total`.
- Unchanged re-render: `reused == total`.
- One overlay or caption edit: `ran: 1` (decorate only).
- One clip edit: `ran: 3` (normalize + concat + decorate).

If `ran: 0` follows a content change, the cache key is stale: that is a bug in
`va`, and the render is stale too. Do not report the fast path as success.

**`plan` before you guess.** `va plan` prints the exact ffmpeg argv per step and
costs no ffmpeg run. When a render looks wrong, the argv usually shows why in
one line.

**Do not invent flags.** `va <command> --help` is short and authoritative; a
guessed flag is a 5-exit-code round trip.

## Editing the file vs running a command

Both are supported and equivalent: the file is the API, the CLI is sugar. Use
`jq` when you are changing something the CLI does not expose, then let the next
command validate it. Keep times as integer microseconds.

Prefer the CLI for anything that needs media facts (`add`, since it probes and
canonicalizes; `trim`, since it is easier to get the units right).

## Proving a visual change

Same project, two renders, one difference:

```bash
va render -o with.mp4
jq 'del(.tracks.overlays)' va.json > bare.json
va --project bare.json render -o without.mp4
# diff the same timestamp and the same crop
.claude/skills/verify/scripts/psnr-ab.sh with.mp4 without.mp4 1.5 "1080:120:0:600"
```

`average:15` means the crop changed; `average:45+` means it did not. Compare the
same timestamp of the same content — frame N of clip A against frame M of clip B
proves nothing.

For a music bed, diff the waveform instead of the mean volume:

```bash
ffmpeg -hide_banner -i with.mp4 -i without.mp4 -filter_complex \
 "[0:a][1:a]amix=inputs=2:weights=1 -1:normalize=0,astats=measure_overall=RMS_level:measure_perchannel=none" \
 -f null - 2>&1 | grep 'RMS level'
```

## Token budget

- Stills are 768 px wide and a few hundred KB, so they are cheaper than a video.
  Six to twelve images is a normal review pass; a 12-tile contact sheet is one
  image.
- `inspect` and `list` are small. `plan` is large: request it, do not loop it.
- Never dump `va.json` into context. Read the fields you need with `jq`.

## Failure ladder

| code | what to do |
| --- | --- |
| `project_invalid` | the edit broke the file. `va inspect` names the field. Re-run the last command. |
| `media_missing` | a path is wrong or a file moved. Re-`add` with the correct path. |
| `ffmpeg_failed` | read the message, then `va plan` and look at the argv for that step. |
| `ffmpeg timed out` | raise `--timeout`, or shorten the timeline. |
| `no_font` | `doctor` shows no resolvable font; install one or fix fontconfig. |
| `usage` | the flags are wrong; `--help` beats guessing. |

## When to stop and ask a human

`va` can tell you a file is valid and render it. It cannot tell you the cut is
good, the caption text is right, or that "Ship faster" fits the brand. Do the
mechanical work; surface content decisions with a still and a one-line question.

## What `va` cannot do yet

Automatic transcription, image overlays, transitions between clips, multi-track
compositing, keyframes, colour grading, GPU encoding. If a task needs one of
those, say so instead of faking it with `--speed` and hope.
