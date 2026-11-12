# Troubleshooting

Start with `splice doctor`, then `splice inspect`. Between them they explain most
failures without spawning a render.

```console
$ splice doctor
{"encoders":{"libx264":true},"ffmpeg":"ffmpeg version 7.1.5 …",
 "filters":{"drawtext":true,"libass":true},
 "font":"/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf","ok":true}
```

## Error codes

| code | exit | cause | fix |
| --- | --- | --- | --- |
| `project_invalid` | 2 | `splice.json` unparseable, wrong `version`, zero canvas, `speed <= 0`, `src_out < src_in` | the message names the field; fix the file or re-run the edit |
| `media_missing` | 3 | a source path does not exist | `splice add <correct path>`; the stored path, resolved against the project directory, must point at an existing file |
| `unreadable` | 3 | ffprobe cannot demux the file | re-encode the source, or check it is really a video |
| `ffmpeg_failed` | 4 | one render step failed | `splice plan`, find that step, read its argv |
| `usage` | 5 | bad flags, unknown clip id, `--to` before `--from` | `splice <cmd> --help` |
| `usage: no clip` | 5 | the id is not in the timeline | `splice list` |
| `config_loose_perms` | — (warning) | the config file is readable by other users | `chmod 600 <path>`, shown in the fix |
| `project_invalid: grade …` | 2 | a grade knob outside its range | the message names the field and the bounds |
| `layer_audio_dropped` | — (warning) | an overlay layer has audio, which is not heard | move those clips to `main` |
| `place_off_canvas` | — (warning) | a layer's placement runs past the right edge | `splice layer set <id> --place 0.6,y,size` |
| `asr_no_key` | 2 | no provider has a key | `splice config set gemini.api_key --stdin` (or deepgram) |
| `asr_key_invalid` | 2 | the provider rejected the key | check `splice config list`: an env var may be overriding the file |
| `asr_rate_limited` | 2 | 429 from the provider | wait, or use the other provider |
| `asr_over_limit` | 2 | audio longer than `--max-duration` | `--yes`, or `--max-duration 20m` |
| `asr_unavailable` | 2 | provider 5xx or the network failed | retry; the transcript cache means a retry re-uploads nothing it already has |
| `whisper_missing` | 2 | no whisper program found or `whisper.program` is not a file | `splice config set whisper.program /path/to/whisper-cli` |
| `whisper_model` | 2 | `whisper.model` is unset, a name, or not a file | `splice config set whisper.model /path/to/ggml-small.bin` |
| `whisper_failed` | 2 | the whisper binary exited non-zero | the message carries its last three stderr lines |
| `asr_no_speech` | — (warning) | the provider found no words | check you pointed at the right audio |
| `asr_unreadable` | 2 | the provider answered with a shape splice does not know | report it with the model id |
| `no_history` | 2 | nothing left to undo, or `.splice/history/` was deleted | `splice history` |
| `no_index` | 2 | `view`/`find` ran without an index, or `.splice/index/` was deleted | `splice index --from <transcript>` |
| `no_font` | 2 | no usable font: no `fc-match`, no bundled-path font | `set SPLICE_FONT=/path/to/font.ttf`, or install one; `doctor` shows where it looked |
| `ffmpeg_missing` | 4 | `ffmpeg` or `ffprobe` is not on `PATH` | the `fix` line names the install command for your OS |
| `ffmpeg_incapable` | 4 | the ffmpeg found lacks `libx264`, `libass` or `freetype` | install a full build; `doctor` lists what is missing |

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
| `place_keys_unused` | placement keys on the bottom layer | reported, not applied: the bottom layer (lowest `z`, normally `z = 0`) is the program frame; add a layer above it and key that, or `splice key <id> --clear` |
| `layer_start_unused` | `start` on the bottom layer | reported, not applied: the bottom layer plays from 0; start a layer above it, or `splice layer set <id> --start 0` |
| `place_unused` | `place` on the bottom layer | reported, not applied: the program frame fills the canvas; `--place` applies to layers above it, or `splice layer set <id> --place 0,0,1` |
| `cache_large` | `.splice/cache` is over 1 GiB | `splice cache prune`; the cache is never authoritative, so deleting it costs a re-render at most |

## Symptoms

**Render reports `ran: 0` after an edit.** The step's cache key did not move,
which means the output on disk does not reflect your change. Delete `.splice/cache`
for a correct render and report it: the key must cover the step's own argv.

**Render "succeeds" in 0 steps but the output is broken or silent.** A stale or
zero-byte cache entry. `ls -la .splice/cache`, then `rm -rf .splice/cache` and re-render.

**Text does not appear.** Three causes, in order of likelihood: `doctor` shows
`font: null`; the overlay window is outside the timeline; `--pos` puts it under
the burned caption band. Check `splice plan` for a `drawtext` in the decorate step.

**Captions do not appear.** The SRT path moved after `caption` was run, or the
cues are outside the timeline. `splice caption <file.srt>` again to re-parse and
re-see the cue count.

**Audio is silent after adding a music bed.** `amix` ends with the *first* input,
which is the clips. If the timeline is shorter than you think, the bed is cut.
`inspect` reports the real `duration`.

**`no layer at z = 0`.** A hand edit removed or moved the base layer. The layer at
z = 0 carries the program audio and must fill the canvas; `splice undo` restores it.

**Keyframes have no visible effect.** `splice key <id> --place`, `splice layer set
<id> --start` and a non-canvas `splice layer set <id> --place` on the bottom layer
(the lowest `z`, normally `z = 0`) are refused — exit 5, `usage`, nothing
written and no snapshot — because the bottom layer is the program frame and
those values never reach the composite. A document that already carries them
(hand-edited, or written before this check) keeps rendering and reports them
as `warnings[]` (`place_keys_unused`, `layer_start_unused`, `place_unused`) in
`splice inspect`, `splice plan`, `splice render` and `splice frames` instead
of failing. Move the edit to a layer above (`splice layer add pip --z 10`,
then key that layer), or clear them with `splice key <id> --clear`,
`splice layer set <id> --start 0` or `splice layer set <id> --place 0,0,1`.
`--clear`, `--rm`, `--list` and `splice layer rm` are never refused, so there
is always a way out.

**Loudness lands a dB or two off target.** One-pass `loudnorm` applies gain
dynamically, so a program with long quiet and loud sections can miss the target
slightly. Measure the file (`ffmpeg -i out.mp4 -af ebur128 -f null -`) and adjust
`--lufs`, or split the program. The video stream is never re-encoded, so the fix
is cheap.

**A loudness render re-runs the whole assembly.** It should not: the loudness step
is appended after the existing ones. If `ran` equals `total`, something upstream
changed too — check `splice plan --loudnorm`.

**Ducking is off by a few seconds.** The spans are mapped through the clips at the
moment you run `splice duck`. Re-cut the timeline and they go stale; `duck_stale`
says so and `splice duck` re-maps them. If a clip is not cut from the indexed file,
`duck` refuses rather than guessing.

**The bed ducks where nobody speaks.** The transcript and the picture disagree —
usually an index built from one export and a clip from another. `splice view
--range` shows the word times `duck` is using.

**The program got shorter after adding a transition.** Correct: the transition
overlaps the two clips, so `duration` drops by the transition length. Check with
`splice list`.

**`transition_too_long`.** The fade is longer than one of the clips it joins.
`fix` names a duration that fits.

**An image overlay does not appear.** Four things, in order: the file is not a
PNG with an alpha channel (a fully transparent asset composites to nothing), the
window is outside the timeline, `--opacity 0`, or the anchor placed it off-canvas
because `--margin` is larger than the frame.

**`scenes` finds nothing, or everything.** `--threshold` is ffmpeg's scene score:
0.4 is a normal cut, lower finds more. Sourced from a single long take, you get one
scene and that is correct. A music video cut every beat needs a much lower
threshold and a `--min` above the beat length.

**`scenes` re-detects every run.** The cache key includes the source's size and
mtime plus `--threshold` and `--min`. Touching the file or changing a flag
re-scans by design.

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
separate caches. `splice frames --preview` reads the proxy; without the flag it
wants the final render. Run the matching `render` first.

**The project is unreadable and nothing opens.** `splice undo`. It restores the
last snapshot without parsing the broken file. If that fails, the previous
document is still at `.splice/history/<seq>.json` under the `project` key.

**Whisper is much slower than the hosted providers.** It is a local model: 15
seconds of audio took 12–15 s on 4 vCPUs, 40 seconds took 2 m 29 s. Use a smaller
`ggml-*.bin` for iteration, or the hosted providers when a network is available.

**`transcribe` worked but the render has no captions.** It writes the file, it
does not attach it. `splice caption captions.srt`, or pass `--attach` next time.

**`transcribe` says `asr_no_key` although you set a key.** `splice config list`
shows the resolved source per provider. A key in a *project* file is never read —
there is no such field on purpose — and an empty environment variable does not
count, so `GEMINI_API_KEY=` falls back to the file.

**`transcribe` returns `cached: true` and never calls the API.** That is the
cache working: keyed by audio, provider, model and language. Change the model or
use a different source to force a call, or delete `.splice/asr/`.

**Text or captions fail with `no_font`, but the rest of the tool works.** Font
lookup tries `SPLICE_FONT`, then `fc-match`, then fonts that ship with the OS
(`C:\Windows\Fonts\segoeui.ttf`, `/System/Library/Fonts/...`,
`/usr/share/fonts/truetype/dejavu/...`). `fc-match` does not exist on a clean
Windows or macOS install, which is why the built-in paths exist. `doctor` reports
which source won.

**A ramp made the motion look steppy.** That is what it is: a staircase of
constant speeds. Raise `--slices`. The duration converges on the true curve as
slices increase (measured: 5.333 s at 1 slice, 5.544 at 16, for a 1×→2× ramp over
8 s whose exact length is 5.545).

**`plan` shows more steps than clips after a ramp.** Expected: each slice is its
own normalize step plus the concat, because each has to be retimed separately.

**A chroma key left a green fringe.** Raise `--similarity` (0.3 default) or add
`--blend` for soft edges; a LUT and a key on the same layer both work, the key
runs first.

**`mask` refuses a layer at z = 0.** There is nothing underneath it to reveal.
Mask a layer above, or `splice layer add`.

**A key I set isn't being used.** `splice config list` shows `source` per key.
`env` means an environment variable is overriding the file — that is deliberate,
because CI mounts tokens as variables. `splice config path` shows whether the file
is even where you think, and whether that directory is writable.

**`config set deepgram.api_key abc123` is refused.** Keys are not accepted as
arguments: they would land in `ps`, in shell history and in any JSON an agent
echoes back. Pipe it (`--stdin`) or name the variable (`--from-env`).

**`find` returns nothing but you can see the words.** Matching ignores case and
punctuation but not word boundaries: `"shipf"` will not match `ship faster`, and
a phrase split by a long pause matches only inside one utterance. Search one
distinct word instead.

**Index times do not line up with the video.** The index describes the audio
file, in that file's time. If you indexed the original recording and are cutting
a clip of it, source time is what `add --from/--to` wants, so they agree; if you
indexed a different export, they do not. `splice index` warns `index_past_audio`
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

**Disk filling up.** `.splice/cache/` keeps one intermediate per step ever rendered
for this project. It is safe to delete at any time.

## Still stuck

`splice plan` is the ground truth: it shows exactly what would run. Compare the step
you expect with the step that exists, and the bug is usually visible in the argv.
