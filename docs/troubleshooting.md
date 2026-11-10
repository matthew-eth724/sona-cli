# Troubleshooting

Start with `sona doctor`, then `sona inspect`. Between them they explain most
failures without spawning a render.

```console
$ sona doctor
{"encoders":{"libx264":true},"ffmpeg":"ffmpeg version 7.1.5 …",
 "filters":{"drawtext":true,"libass":true},
 "font":"/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf","ok":true}
```

## Error codes

| code | exit | cause | fix |
| --- | --- | --- | --- |
| `project_invalid` | 2 | `sona.json` unparseable, wrong `version`, zero canvas, `speed <= 0`, `src_out < src_in` | the message names the field; fix the file or re-run the edit |
| `media_missing` | 3 | a source path does not exist | `sona add <correct path>`; sources are canonicalized at add time |
| `unreadable` | 3 | ffprobe cannot demux the file | re-encode the source, or check it is really a video |
| `ffmpeg_failed` | 4 | one render step failed | `sona plan`, find that step, read its argv |
| `usage` | 5 | bad flags, unknown clip id, `--to` before `--from` | `sona <cmd> --help` |
| `usage: no clip` | 5 | the id is not in the timeline | `sona list` |
| `no_history` | 2 | nothing left to undo, or `.sona/history/` was deleted | `sona history` |
| `no_index` | 2 | `view`/`find` ran without an index, or `.sona/index/` was deleted | `sona index --from <transcript>` |
| `no_font` | 2 | `fc-match` resolved no sans font | install a font or fix fontconfig; `doctor` shows the resolved path |

## Warnings

Warnings never fail a render. They appear in `warnings[]` on the command that
created them, and in `inspect`.

| code | meaning | what actually happens |
| --- | --- | --- |
| `clip_past_media` | `src_out` is beyond the source's length | the clip is clamped; reported `duration` is the truth |
| `clip_has_no_video` | source has no video stream | the clip is rejected by `add`, not warned |
| `clip_empty` | the trim left nothing | the clip is skipped in the render |
| `captions_missing` | the SRT path is gone | the render fails; re-`caption` |
| `media_missing` | a source file moved | `list` shows `missing: true` |
| `text_past_end` | an overlay extends past the timeline | the overlay is simply cut short |

## Symptoms

**Render reports `ran: 0` after an edit.** The step's cache key did not move,
which means the output on disk does not reflect your change. Delete `.sona/cache`
for a correct render and report it: the key must cover the step's own argv.

**Render "succeeds" in 0 steps but the output is broken or silent.** A stale or
zero-byte cache entry. `ls -la .sona/cache`, then `rm -rf .sona/cache` and re-render.

**Text does not appear.** Three causes, in order of likelihood: `doctor` shows
`font: null`; the overlay window is outside the timeline; `--pos` puts it under
the burned caption band. Check `sona plan` for a `drawtext` in the decorate step.

**Captions do not appear.** The SRT path moved after `caption` was run, or the
cues are outside the timeline. `sona caption <file.srt>` again to re-parse and
re-see the cue count.

**Audio is silent after adding a music bed.** `amix` ends with the *first* input,
which is the clips. If the timeline is shorter than you think, the bed is cut.
`inspect` reports the real `duration`.

**`cut` removed nothing.** Either the transcript has no filler words and no gap
over the threshold, or the index is for a different file (that case is an error,
not silence). Try `--dry-run` with a lower threshold, and remember `--silences`
shortens a gap rather than deleting it: `--keep 200ms` still leaves 200 ms.

**`cut` says it would remove the whole clip.** The threshold is above every gap
and `--keep` is large enough to leave nothing. Lower `--keep`, then re-run.

**Captions or overlays are now out of sync.** `cut` removes time, so anything
timed against the old timeline moved. `cut_shifts_layers` warned about it. Recut
captions after the edit, not before.

**`frames` says nothing rendered, but you rendered.** Preview and final keep
separate caches. `sona frames --preview` reads the proxy; without the flag it
wants the final render. Run the matching `render` first.

**The project is unreadable and nothing opens.** `sona undo`. It restores the
last snapshot without parsing the broken file. If that fails, the previous
document is still at `.sona/history/<seq>.json` under the `project` key.

**`find` returns nothing but you can see the words.** Matching ignores case and
punctuation but not word boundaries: `"shipf"` will not match `ship faster`, and
a phrase split by a long pause matches only inside one utterance. Search one
distinct word instead.

**Index times do not line up with the video.** The index describes the audio
file, in that file's time. If you indexed the original recording and are cutting
a clip of it, source time is what `add --from/--to` wants, so they agree; if you
indexed a different export, they do not. `sona index` warns `index_past_audio`
when the transcript runs past the end of the audio, which catches the common
version-skew mistake.

**Two clips play out of sync after concatenation.** They should not: each clip is
normalized to the canvas fps, SAR and channel layout first. If it happens, the
source is probably variable-frame-rate and lying about its rate — check with
`ffprobe -show_streams` for a differing `r_frame_rate` and `avg_frame_rate`, and
re-encode that source.

**Duration reported does not match the file.** Within one frame, it will:
`duration` is integer microseconds from the sources, and the output rounds to
frame boundaries. A gap of more than a frame means a clip was clamped or skipped
— look at `warnings[]`.

**`ffmpeg failed: … No such filter`.** A path got into a filter string
unescaped, which on Windows means a `C:` drive letter. This is a bug: every
filter path must go through `paths::filter_escape`. `cargo test paths` covers it.

**Render takes forever.** Each clip is decoded once, assembled once, and the
assemble pass re-encodes. A 36-second 1080x1920 clip takes about 13 seconds at
`crf 16` on 4 vCPUs. Re-renders reuse everything unaffected by the edit, so a
single overlay change is one step, not four.

**Disk filling up.** `.sona/cache/` keeps one intermediate per step ever rendered
for this project. It is safe to delete at any time.

## Still stuck

`sona plan` is the ground truth: it shows exactly what would run. Compare the step
you expect with the step that exists, and the bug is usually visible in the argv.
