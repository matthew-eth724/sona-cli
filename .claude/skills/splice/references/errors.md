# Errors and warnings

Every failure is one JSON object with `error.code`, `error.message` and
`error.fix`. Read `fix` first: it names a command that clears the error. Exit
codes classify for a caller that only sees the status.

| exit | class |
| --- | --- |
| 0 | success |
| 2 | project, precondition, transcription or configuration |
| 3 | media missing or unreadable |
| 4 | ffmpeg failed, missing, or incapable |
| 5 | bad usage |

## Error codes

| code | exit | what happened | fix |
| --- | --- | --- | --- |
| `usage` | 5 | wrong arguments, or an edit the compositor cannot apply (bottom-layer `place`/`start`) | `splice <command> --help` |
| `project_invalid` | 2 | the document breaks an invariant: a duplicate `z`, no layer at `z = 0`, `src_out < src_in` | `splice undo` |
| `clip_empty` | 2 | a trim left nothing | check the reported `duration` |
| `media_missing` | 3 | a stored `source` does not resolve against the project directory | `splice list` |
| `unreadable` | 3 | a file exists but ffprobe cannot read it | replace the file |
| `ffmpeg_missing` | 4 | no ffmpeg found: neither `SPLICE_FFMPEG`, nor beside the binary, nor on `PATH` | install ffmpeg |
| `ffmpeg_incapable` | 4 | ffmpeg lacks `libx264` or `libass` | use a fuller build |
| `ffmpeg_failed` | 4 | a render step exited non-zero; `message` carries its last stderr lines | `splice plan` shows the argv |
| `no_font` | 2 | no usable font for text or captions | install a font, then `splice doctor` |
| `nothing_rendered` | 2 | `frames` before any render for that profile | `splice render` (or `--preview`) |
| `asr_no_key` | 2 | no transcription provider has a key | `splice config set gemini.api_key --stdin` |
| `asr_over_limit` | 2 | audio longer than `--max-duration` | `--yes`, or raise `--max-duration` |
| `asr_key_invalid` | 2 | the provider rejected the key | `splice config list`: an env var may override the file |
| `asr_unavailable` | 2 | provider 5xx or the network failed | retry; the transcript cache re-uploads nothing |
| `asr_unreadable` | 2 | the provider answered with a shape splice does not know | try another provider |
| `asr_no_speech` | 2 | the audio has no words in it | check the source has speech |
| `asr_rate_limited` | 2 | the provider throttled the call | retry later |
| `whisper_missing` | 2 | `whisper.program` unset or not a file | `splice config set whisper.program /path/to/whisper-cli` |
| `whisper_model` | 2 | `whisper.model` unset, a name, or not a file | `splice config set whisper.model /path/to/ggml-small.bin` |
| `whisper_failed` | 2 | the whisper binary exited non-zero | the message carries its last lines |
| `no_index` | 2 | an index-driven command ran before the index exists | `splice index --from <transcript.json\|.srt>` |
| `no_history` | 2 | `undo` with nothing recorded | `splice history` |
| `internal` | 2 | a bug in splice | `splice doctor`, then report it with the message |

## Warnings worth acting on

Warnings mean the command succeeded but the result will not do what you asked.
They appear as `warnings[]` on the response, and again from `inspect`, `plan`,
`render` and `frames`.

| code | meaning | what to do |
| --- | --- | --- |
| `clip_past_media` | trim ran past the end and was clamped | check the reported `duration` |
| `layer_audio_dropped` | an upper layer has audio; only `z = 0` is heard | move those clips to the main layer |
| `place_keys_unused` | the bottom layer carries placement keys that never apply | `splice key <id> --clear`, or move the layer above |
| `layer_start_unused` | the bottom layer's `start` is not applied | `splice layer set <id> --start 0` |
| `place_unused` | the bottom layer's `place` is ignored; the frame is the canvas | `splice layer set <id> --place 0,0,1` |
| `place_off_canvas` | an overlay sits outside the frame at some point | fix the placement |
| `text_past_end` | a title ends after the picture | adjust `--to` |
| `captions_missing` | the SRT does not exist where the document says | re-attach or fix the path |
| `cache_large` | the render cache passed 1 GiB | `splice cache prune` |
| `config_loose_perms` | the config file is readable by other users; `fix` carries the `chmod` | `chmod 600 <path>` |
| `clip_has_no_video` | `add` rejected a file with no video stream | use a file with video |

## Reading a failure properly

1. `error.code` says which class, and therefore which exit status a caller sees.
2. `error.fix` is a command. Run it unless you know better.
3. If the fix is `splice plan`, run it and read the argv: the failure is in the
   filter graph, and `plan` shows exactly what ffmpeg was asked to do.
4. If a render fails after an edit that reported `ok:true`, re-read that edit's
   `warnings[]`. That is where the reason usually is.
