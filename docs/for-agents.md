# For agents

How to drive `sona` without wasting tokens or trusting yourself.

## The loop

```
doctor → inspect → edit → render → frames → look
```

1. `sona doctor` once per machine. If `libx264`, `libass` or `drawtext` is false,
   or `font` is null, stop: every render will fail on a filter, not on your edit.
2. `sona inspect` before editing. It is cheap and returns `duration`, the clip
   list and `warnings[]`.
3. Make the smallest edit that expresses the change. One command, one concept.
4. `sona render --preview` while it is wrong: 480x854 at 15 fps, its own cache,
   roughly a quarter of the wall time. `sona render -o out.mp4` when it is right.
5. `sona frames --at <times>` when the change is visual, and actually look at the
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

**Preview caches are not final caches.** `render --preview` and `render` share no
steps, and `frames` reads one or the other. `frames` saying `nothing rendered yet`
usually means you previewed and asked for final stills, or the reverse; pass
`--preview` to match.

**Trust the cache counters.**
- Cold render: `ran == total`.
- Unchanged re-render: `reused == total`.
- One overlay or caption edit: `ran: 1` (decorate only).
- One clip edit: `ran: 3` (normalize + concat + decorate).

If `ran: 0` follows a content change, the cache key is stale: that is a bug in
`sona`, and the render is stale too. Do not report the fast path as success.

**`plan` before you guess.** `sona plan` prints the exact ffmpeg argv per step and
costs no ffmpeg run. When a render looks wrong, the argv usually shows why in
one line.

**Dry-run before anything you would not want to explain.** Every mutating command
takes `--dry-run` and returns the same envelope, including the resulting
`duration` and `warnings`, without writing anything:

```bash
sona trim clip_1 --to 30s --dry-run     # see the duration and the warnings first
sona cut clip_1 --fillers --dry-run
```

**Every warning and error carries a `fix`.** Read it: it is the command that
clears the problem, usually with your own ids already filled in.

**Loudness is a delivery flag, not an iteration flag.** `--loudnorm` costs one
extra encode of the audio and nothing else, so use it on the render you keep.
Targets: `--lufs -14` for web, `--lufs -16` for podcast, `--lufs -23` for
broadcast EBU R128.

**Duck the bed from the transcript, not by ear.** `sona duck --db -18` builds the
gain envelope from word times, so the music drops exactly where someone speaks
and lifts in the gaps. Re-run it after any change to the cut: `duck_stale` tells
you when, and the fix is the command itself.

**Transitions cost a re-encode; cuts do not.** Adding `transition` to an assemble
step turns the copy-concat into an `xfade` chain. Worth it between shots, not
worth it between two halves of one sentence.

**Do not invent flags.** `sona <command> --help` is short and authoritative; a
guessed flag is a 5-exit-code round trip.

## Reading a long recording

Do not dump a transcript into context. Index it once, then ask narrow questions:

```bash
sona index --from transcript.json --audio podcast.mp4   # once, cached by audio hash
sona find "good memory"                                  # one line per hit, match bracketed
sona view --range 0:11-0:19                              # ~10 lines, ids for anchoring
```

`view` and `find` print plain text lines (`start-end  firstId-lastId  text`), not
the JSON envelope — that is what makes them cheap; add `--json` when a caller
needs structure.

The ids (`w27`) are the handles you cut with. You never have to convert them to
times:

```bash
sona find "good memory"              # 00:12.6-00:14.4  w27-w31  …
sona add podcast.mp4 --words w27-w31 # cuts with 120ms of air on each side
sona trim clip_1 --words w27-w31     # same range, applied to an existing clip
```

After cutting a clip to the right sentence, take the dead air and the ums out
before you caption it:

```bash
sona cut clip_1 --silences '>0.6s' --fillers --dry-run   # see the spans first
sona cut clip_1 --silences '>0.6s' --fillers
sona list | jq -r '.duration'                            # shorter, split into spans
```

Do it before captions, not after: cutting shifts everything timed against the
timeline, and `cut_shifts_layers` will tell you that you did it in the wrong
order.

`--pad` controls that air: raise it when a cut clips a consonant, set `--pad 0`
when you want the word boundary exactly as reported. Word times from ASR are
approximate (±100ms or so), so `--pad 0` plus a tight cut is how you get a
chopped first phoneme.

## Finding the interesting part

```bash
sona index --from transcript.json --audio podcast.mp4
sona find "that's the thing"       # word ranges
sona scenes                        # shot boundaries, with a keyframe to sample
sona view --range 12:30-12:50
```

`scenes` gives you cuts where the picture changes, `find` gives you cuts where the
words are. Use both before deciding a window: a good quote that lands mid-shot is
a worse clip than an okay quote at a shot boundary.

## When you break it

Every mutation snapshots the document first:

```bash
sona history                 # newest first, labelled with the command
sona undo                    # one step back
sona undo --steps 3          # three
```

`undo` does not parse `sona.json`, so it works when your edit made the file
unreadable. It also skips snapshots that would restore an identical document, so
a command that failed half-way does not cost you an undo.

Do not hand-edit `.sona/history/` and do not clear `.sona/` mid-session while you
expect to undo.

## Editing the file vs running a command

Both are supported and equivalent: the file is the API, the CLI is sugar. Use
`jq` when you are changing something the CLI does not expose, then let the next
command validate it. Keep times as integer microseconds.

Prefer the CLI for anything that needs media facts (`add`, since it probes and
canonicalizes; `trim`, since it is easier to get the units right).

## Proving a visual change

Same project, two renders, one difference:

```bash
sona render -o with.mp4
jq 'del(.tracks.overlays)' sona.json > bare.json
sona --project bare.json render -o without.mp4
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
- Never dump `sona.json` into context. Read the fields you need with `jq`.

## Failure ladder

| code | what to do |
| --- | --- |
| `project_invalid` | the edit broke the file. `sona inspect` names the field. Re-run the last command. |
| `media_missing` | a path is wrong or a file moved. Re-`add` with the correct path. |
| `ffmpeg_failed` | read the message, then `sona plan` and look at the argv for that step. |
| `ffmpeg timed out` | raise `--timeout`, or shorten the timeline. |
| `no_font` | `doctor` shows no resolvable font; install one or fix fontconfig. |
| `usage` | the flags are wrong; `--help` beats guessing. |

## When to stop and ask a human

`sona` can tell you a file is valid and render it. It cannot tell you the cut is
good, the caption text is right, or that "Ship faster" fits the brand. Do the
mechanical work; surface content decisions with a still and a one-line question.

## What `sona` cannot do yet

Automatic transcription, image overlays, transitions between clips, multi-track
compositing, keyframes, colour grading, GPU encoding. If a task needs one of
those, say so instead of faking it with `--speed` and hope.
