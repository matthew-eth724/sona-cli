# The project file

`splice.json` holds the whole edit. It is the only state: `.splice/` is cache, and
deleting it costs a re-render, never information.

The file is meant to be both machine-written (by the CLI) and hand-edited (by an
agent with `jq`, or by a person). Every command re-parses and re-validates it, so
a hand edit is legal — it just has to survive the rules below.

```json
{
  "version": 2,
  "canvas": { "width": 1080, "height": 1920, "fps": "30" },
  "tracks": {
    "layers": [
      {
        "id": "main",
        "z": 0,
        "start": 0,
        "place": { "x": 0.0, "y": 0.0, "size": 1.0 },
        "clips": [
          {
            "id": "clip_1",
            "source": "/media/intro.mp4",
            "src_in": 1000000,
            "src_out": 6000000,
            "speed": 1.0,
            "transform": "fit",
            "gain_db": 0.0
          }
        ]
      }
    ],
    "audio": { "source": "/media/music.mp3", "gain_db": -14.0 },
    "overlays": [
      {
        "kind": "text",
        "id": "text_1",
        "content": "Ship faster",
        "at": { "start": 500000, "end": 3000000 },
        "anchor": "bottom",
        "size": 48,
        "color": "white"
      }
    ],
    "captions": { "source": "/media/subs.srt", "burn": true }
  }
}
```

## Units

**All times are integer microseconds.** `1000000` is one second. No floats, no
seconds, no frames. The CLI accepts `4s` / `00:00:04.000` and converts; the file
always stores integers, so a round trip through `jq` never drifts.

Serializers also accept a time *string* on input (either form parses), but the
CLI always writes integers.

## canvas

| field | type | notes |
| --- | --- | --- |
| `width`, `height` | u32 | non-zero; every clip is conformed to this |
| `fps` | `"30"`, `"29.97"`, `"30000/1001"` | rational, so NTSC rates are exact |

Changing the canvas invalidates every clip, because each normalized segment
carries the size and rate.

## version

`2` is current. A version-1 document (one flat `tracks.video` array) is upgraded
in memory on open and written back as version 2 on the next save. Nothing is lost
in the upgrade: v1 was always exactly one layer, named `main`, at `z: 0`, filling
the canvas, with no camera.

## tracks.layers

An unordered array; **`z` is the stacking order** and it is unique, so there is
one total order and no tie-breaking rule. Version 1's `tracks.video` is now
`layers[0].clips`.

| field | type | default | notes |
| --- | --- | --- | --- |
| `id` | string | required | `main` for the layer carried over from v1 |
| `z` | i32 | required | higher is on top; `z: 0` is the program layer and carries audio |
| `start` | time | `0` | where this layer's t=0 sits on the program clock |
| `place` | `{x, y, size}` | `{0, 0, 1}` | where its frame lands on the canvas, in canvas fractions |
| `motion` | object | omitted | the camera: `keys: [{t, zoom, at}]`, linear between keys |
| `place_keys` | array | omitted | `[{t, x, y}]` in layer-local time: slides the layer's position. `place.size` does not animate |
| `clips` | array | required | sequential inside the layer: version 1's model, unchanged |

The layer at `z: 0` must fill the canvas (`place` of `{0,0,1}`): a base layer with
a hole is a bug, not a look.

**Multi-layer projects and `motion` do not render yet.** They validate and save,
and `splice render` refuses with `composite_not_built` rather than producing a
render that silently ignores the extra layers. The design is
[design-compositing.md](design-compositing.md).

## layers[].clips

An ordered array. **The order of the array is the timeline order inside that
layer** — there is no separate sequence or timeline object to keep in sync.

| field | type | default | notes |
| --- | --- | --- | --- |
| `id` | string | required | unique; allocated by the CLI as `clip_<n>` |
| `source` | string | required | absolute path; `add` canonicalizes |
| `src_in` | time | required | offset into the source |
| `src_out` | time | omitted = end of source | clamped to the media duration |
| `speed` | f64 | `1.0` | must be > 0; pitch preserved |
| `transform` | `fit` \| `fill` | `fit` | pad or crop to the canvas |
| `gain_db` | f32 | `0.0` | per-clip audio gain |

A clip's contribution to its layer is
`(min(src_out, media_duration) - src_in) / speed`. Program duration is
**the longest layer** — `max over layers (start + layer duration)`, where a
layer's duration is its clips minus the transitions that overlap them.

## clip.transition

An optional incoming transition from the previous clip. Absent means a hard cut.

| field | type | notes |
| --- | --- | --- |
| `kind` | `crossfade` \| `dip_to_black` | ffmpeg `fade` / `fadeblack` |
| `duration` | time | must be shorter than both neighbours |

The first clip may not have one, and the program duration subtracts each
transition, because the two clips overlap for that long.

## tracks.audio

One optional music bed, mixed under the clips and ended by them.

| field | type | default |
| --- | --- | --- |
| `source` | string | required |
| `gain_db` | f32 | `0.0` |
| `duck` | object | omitted when not ducking |

`duck` is written by `splice duck`:

| field | type | notes |
| --- | --- | --- |
| `db` | f32 | negative; how far the bed drops |
| `ramp` | time | fade length for each phrase |
| `spans` | array of `{start,end}` | speech windows **in timeline time**, already mapped |
| `layout` | string | fingerprint of the cut these spans were mapped against |

`layout` is what makes staleness detectable: if the clips move, `diagnose` reports
`duck_stale` and the fix is to run `splice duck` again. Spans are timeline time, so
a render never needs the index.

## tracks.overlays

A tagged array; the tag is `kind`.

`kind: "image"` — a PNG over the picture:

| field | type | default | notes |
| --- | --- | --- | --- |
| `id` | string | required | `image_<n>` |
| `source` | string | required | absolute path to a PNG (alpha respected) |
| `at` | `{start, end}` | required | microseconds |
| `anchor` | `top`, `top_left`, `top_right`, `center`, `bottom`, `bottom_left`, `bottom_right` | `bottom` |
| `width` | f32 | required | fraction of canvas width, so preview renders scale |
| `opacity` | f32 | `1.0` | 0–1, multiplies the PNG's alpha |
| `margin` | f32 | `0.03` | fraction of canvas width |

`kind: "text"`:

| field | type | default | notes |
| --- | --- | --- | --- |
| `id` | string | required | `text_<n>` |
| `content` | string | required | escaped for drawtext internally |
| `at` | `{start, end}` | required | microseconds, end-exclusive |
| `anchor` | `top` \| `center` \| `bottom` | `bottom` |
| `size` | u32 | `48` | font size in canvas pixels |
| `color` | string | `white` | any ffmpeg colour |

## tracks.index

A pointer to the word index this project reads. The words themselves live in
`.splice/index/<audio_hash>.json`; the project only records which one is current.

| field | type | notes |
| --- | --- | --- |
| `audio_hash` | string | SHA-256 of the extracted 16 kHz mono audio |
| `source` | string | the media file the audio came from |
| `transcript` | string | the transcript file that produced the words |
| `provider` | string | `gemini`, `deepgram` or `srt` |
| `language` | string | optional, when the provider reported one |
| `words` | number | count, for a cheap sanity check |
| `approximate` | bool | omitted when false; true when timings were interpolated from SRT |
| `model` | string | which model produced the words; omitted when unknown. A different model re-indexes rather than reusing the cache |

Deleting `.splice/index/` leaves this pointer dangling, and `view`/`find` will say
`no_index` until `splice index` runs again.

## tracks.captions

| field | type | default | notes |
| --- | --- | --- | --- |
| `source` | string | required | absolute path to an `.srt` |
| `burn` | bool | `true` | burn into pixels; `false` currently renders nothing |

## Invariants

Hard errors — the file will not load:

- `version` must equal `1`.
- `canvas.width` and `canvas.height` must be non-zero.
- Every clip's `speed` must be > 0.
- `src_out` must be ≥ `src_in`.
- `tracks.video` must not be empty at render time (a project with no clips can
  exist; it just cannot render).

Warnings — reported in the `warnings[]` of whichever command caused them, and in
`inspect`:

| code | meaning |
| --- | --- |
| `media_missing` | a source path does not exist |
| `clip_past_media` | `src_out` is beyond the source's duration (clamped) |
| `clip_has_no_video` | the source has no video stream |
| `clip_empty` | trimming left the clip with no duration |
| `captions_missing` | the SRT path does not exist |
| `text_past_end` | an overlay window runs past the end of the timeline |

## What not to put in it

Credentials. `GEMINI_API_KEY` and `DEEPGRAM_API_KEY` are read from the
environment only. A project file gets committed, shared and pasted into
transcripts; secrets do not belong in it.

Rendered output paths. The render target is a flag (`splice render -o`), not project
state, so the same project can produce a preview and a master without an edit.
