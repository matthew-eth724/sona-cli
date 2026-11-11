# Transcribe

## Sub-features

- `transcribe` — call the configured provider, write the index and an SRT to
  `captions.srt`. It does **not** attach them; `--attach` does.
- `--dry-run` — say what would be sent (provider, model, bytes, endpoint) and
  send nothing.
- Cache in `.splice/asr/` keyed by audio + provider + model + language.
- `--provider`, `--model`, `--language`, `--audio`, `--max-duration`, `--yes`.

## How to get to it (user POV)

```bash
echo "$DEEPGRAM_API_KEY" | $SONA config set deepgram.api_key --stdin
$SONA transcribe                 # writes captions.srt, attached: false
$SONA caption captions.srt       # attaching is the separate decision
$SONA render -o out.mp4
```

## Driving it with bash + JSON

The offline half needs no key and no network, so run that first:

```bash
export SPLICE_CONFIG=$(mktemp -d)/config.json
$SONA transcribe --dry-run | jq '{provider, model, bytes, duration, endpoint}'
$SONA transcribe --max-duration 0s          # asr_over_limit, before any upload
```

The live half needs a key. With one configured:

```bash
$SONA transcribe | jq '{provider, model, words, cues, cached}'
$SONA transcribe | jq '.cached'              # true: no request the second time
```

## Gotchas

- **`transcribe` does not burn.** The JSON says `attached: false` and carries a
  `next` command. A render straight after it will have no captions, and that is
  correct — check `attached` before blaming the render.
- The SRT is written beside the project, not into `.splice/`, so deleting the
  cache cannot orphan captions the project points at.
- **A cached result never calls the API**, by design. To force a call change
  `--model`, use a different source, or delete `.splice/asr/`.
- **Gemini answers a bad key with 400, not 401**, and its error body is a JSON
  *array*. Both are handled; if you add a provider, expect neither.
- The 10-minute cap is checked **before** the upload, so refusing is free and the
  exit code is 2.
- Captions and the index come from one set of words, so an SRT that disagrees with
  `view` would mean a bug, not a stale file.
- **Silence is not a failure.** A sine tone or a silent clip returns
  `{"words":0,"speech":false}` with an `asr_no_speech` warning and attaches
  nothing. Use that as the offline-ish live test, since it needs a key but no
  real speech.
- `.splice/asr/*.mp3` is the extracted 16 kHz mono audio; it is cache, and the
  transcript cache is keyed to it.
- Without a key nothing here is testable end to end. `--dry-run` plus the
  `asr_no_key` refusal is the honest offline proof.
