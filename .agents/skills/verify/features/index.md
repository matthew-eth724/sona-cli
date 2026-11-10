# Word index, view and find

## Sub-features

- `index` — build a word index from a transcript, cached by the hash of the
  extracted 16 kHz mono audio.
- `view --range` — print indexed words as compact text lines.
- `find "<query>"` — case- and punctuation-insensitive phrase search with the
  match bracketed.

## How to get to it (user POV)

```bash
$SONA index --from transcript.json --audio podcast.mp4
$SONA view --range 0:11-0:19
$SONA find "good memory"
```

## Driving it with bash + JSON

Fixtures are committed, so no network and no ASR needed:

```bash
cp <repo>/resource/audio1.mp3 .                      # 31s mono, 78 words
cp <repo>/tests/fixtures/asr/gemini-verbatim.json .

$SONA init >/dev/null
$SONA index --from gemini-verbatim.json --audio audio1.mp3 | jq '.words, .cached'
# 78, false
$SONA index --from gemini-verbatim.json | jq '.cached'
# true  (same audio hash, same transcript -> cached)

$SONA view --range 0:05-0:15
# 00:13.1-00:14.1  w27-w31  You've got a good memory.

$SONA find "good memory"
# 00:12.6-00:14.4  w27-w31  You've got a [good memory.]
```

Checks worth asserting: the word count matches the source response (78 for
Gemini on this clip, 77 for Deepgram — they tokenize differently); ids are
`w1..wN` in order; a second `index` is `cached: true` and takes milliseconds.

## Gotchas

- `view` and `find` print **text**, not the JSON envelope. That is deliberate:
  970 bytes of lines against 2,275 bytes of JSON for the same 30 seconds. Use
  `--json` when the caller needs structure; errors are still JSON.
- No hits prints nothing and exits 0. Do not read empty output as failure; use
  `--json` when the distinction matters.
- Index ids are stable for one index, not across transcripts. Re-indexing with a
  different transcript rewrites the words in place and ids shift.
- An SRT index is marked `approximate: true`: word times inside a cue are
  interpolated by character share, so do not expect word-accurate cuts.
- Gemini writes `"3s"` as well as `"0.700s"`; the parser accepts both. A
  stricter parser silently drops about 13% of words.
- `index_past_audio` means the transcript runs past the end of the audio —
  usually a mismatched export, not a bug in the index.
- Piping is safe: `view | head -2` ends quietly. It used to panic with a broken
  pipe, which broke every `| head` and `| jq` pipeline in this repo.
- The extracted audio lives in `.sona/audio/` and the index in `.sona/index/`.
  Both are cache; deleting them costs one re-index, never information.
