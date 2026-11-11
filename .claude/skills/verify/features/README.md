# Feature map

The user-facing surface of `splice`, one file per feature. Each file says what it
is, how a person reaches it, how to drive it, and what observable end state
proves it works. Drive the real command; never assert on an internal function.

| feature | entry points | proof of life |
| --- | --- | --- |
| [timeline](timeline.md) | `init` `add` `list` `trim` `speed` `move` `rm` `inspect` | `duration` and clip order match the edits |
| [overlays](overlays.md) | `text` `image` | pixels change in the anchor band, only inside the time window |
| [captions](captions.md) | `caption` | cue count parsed, pixels change in the caption band |
| [render-cache](render-cache.md) | `plan` `render` | cold render `ran==total`; unchanged `reused==total`; one edit `ran<total` |
| [review-frames](review-frames.md) | `frames` | PNG files exist at the requested times, sheet tiled |
| [index](index.md) | `index` `view` `find` | word count matches the transcript, ids are `w1..wN`, second index is `cached: true` |
| [cut](cut.md) | `cut` | `saved` equals the removed spans, timeline splits into surviving spans, render length matches |
| [transitions](transitions.md) | `transition` | 3x3s clips report 7.8s, offsets 2.4/4.8, a sampled blend colour |
| [scenes](scenes.md) | `scenes` | three cuts in a three-shot clip, a 0.2s flash merged at the default `--min` |
| [transcribe](transcribe.md) | `transcribe` | dry run names the endpoint; no key gives `asr_no_key`; a live run caches |
| [config](config.md) | `config` | a key stored 0600, never printed back, env beating file, model recorded by `index` |
| [history](history.md) | `history` `undo` | after undo the duration is identical to before the mutation; undo works on a corrupt document |
| [audio-bed](audio-bed.md) | `audio` | residual RMS between with/without is well above silence |

Unmapped for now: ASR providers (`caption auto`), multi-track video, keyframes.
They are designed in `docs/design.md` and not implemented.
