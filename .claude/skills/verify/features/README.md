# Feature map

The user-facing surface of `va`, one file per feature. Each file says what it
is, how a person reaches it, how to drive it, and what observable end state
proves it works. Drive the real command; never assert on an internal function.

| feature | entry points | proof of life |
| --- | --- | --- |
| [timeline](timeline.md) | `init` `add` `list` `trim` `speed` `move` `rm` `inspect` | `duration` and clip order match the edits |
| [overlays](overlays.md) | `text` | pixels change in the anchor band, only inside the time window |
| [captions](captions.md) | `caption` | cue count parsed, pixels change in the caption band |
| [render-cache](render-cache.md) | `plan` `render` | cold render `ran==total`; unchanged `reused==total`; one edit `ran<total` |
| [review-frames](review-frames.md) | `frames` | PNG files exist at the requested times, sheet tiled |
| [audio-bed](audio-bed.md) | `audio` | residual RMS between with/without is well above silence |

Unmapped for now: ASR (`caption auto`), multi-track video, keyframes. They are
designed in `docs/design.md` and not implemented.
