# Keys and model configuration

## Sub-features

- `config set <provider>.api_key --stdin | --from-env VAR` — a key, never as an
  argument.
- `config set <provider>.model <id>` / `get` / `unset` / `list` / `path`.
- Precedence: key from env then file; model from `--model`, then file, then the
  pinned default.
- `doctor` reports the resolution: source, fingerprint, model, and whether the
  config file is writable.

## How to get to it (user POV)

```bash
export SPLICE_CONFIG=/tmp/cfg/config.json     # keep tests out of your real config
$SONA config list                              # nothing set yet
echo "test-key-value" | $SONA config set gemini.api_key --stdin
$SONA config set deepgram.model nova-3
$SONA config list | jq '.providers.gemini.api_key'
```

## Driving it with bash + JSON

```bash
export SPLICE_CONFIG=$(mktemp -d)/config.json
echo "k-123" | $SONA config set gemini.api_key --stdin | jq '.fingerprint'
$SONA config list | jq -r '.providers.gemini.api_key.source'   # file
GEMINI_API_KEY=env-k $SONA config list | jq -r '.providers.gemini.api_key.source'  # env

# the file must not be world-readable
stat -c '%a' "$SPLICE_CONFIG"        # 600
grep -c 'k-123' "$SPLICE_CONFIG"     # 1 — it is stored, not echoed
$SONA config get gemini.api_key | grep -c 'k-123'   # 0 — and never printed back
```

## Gotchas

- A key passed as an argument is refused on purpose. `--stdin` or `--from-env`.
- `config get …api_key` returns a fingerprint, never the value, and no flag changes
  that. If you need to verify which key is live, compare fingerprints.
- An environment variable beats the file. `list` says `source: env`, which is
  usually the answer to "why is it using the wrong key".
- Nothing calls a provider yet: `config` stores values that `caption auto` will
  read. `index --model` is the one thing that consumes a model today.
- Set `SPLICE_CONFIG` when testing, or you will overwrite the developer's real key
  file. The verification scripts do this.
