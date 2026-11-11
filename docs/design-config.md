# Keys and model configuration — design

Status: **storage built; the call that consumes it is next.**

## Problem

The tool publishes keys it cannot use. `doctor` reports whether `GEMINI_API_KEY`
and `DEEPGRAM_API_KEY` are set, and nothing else in the binary reads them — there
is no way to add a key except exporting it in the right shell, and no way to say
which model. The ask is for users to add keys and configure a model.

The trap is building a settings file nobody reads. A config surface has to be
load-bearing from the commit that introduces it, or it is a file that rots
unnoticed while everyone keeps exporting environment variables.

## Usage

### A person, adding a key

```console
$ echo "$GEMINI_API_KEY" | splice config set gemini.api_key --stdin
{"ok":true,"provider":"gemini","field":"api_key","action":"set",
 "path":"/home/ana/.config/splice/config.json","fingerprint":"b4e1c07a"}

$ splice config set deepgram.api_key --from-env DEEPGRAM_API_KEY
{"ok":true,"provider":"deepgram","field":"api_key","action":"set","source":"env:DEEPGRAM_API_KEY"}

$ splice config set gemini.model gemini-3.5-transcribe
{"ok":true,"provider":"gemini","field":"model","action":"set","value":"gemini-3.5-transcribe"}
```

**There is no `--api-key` flag, on purpose.** A key in argv lands in `ps`, in
shell history, and in the JSON that agents paste into transcripts. `--stdin` and
`--from-env` are the only ways a secret enters the file.

### Checking, without leaking

```console
$ splice config list
{"path":"/home/ana/.config/splice/config.json","exists":true,"writable":true,
 "providers":{
   "gemini":{"api_key":{"set":true,"source":"file","fingerprint":"b4e1c07a"},
             "model":{"value":"gemini-3.5-transcribe","source":"default"}},
   "deepgram":{"api_key":{"set":true,"source":"env:DEEPGRAM_API_KEY","fingerprint":"9f21c4de"},
               "model":{"value":"nova-3","source":"file"}}}}
```

`fingerprint` is the first 8 hex of the key's SHA-256. It answers the two
questions that matter — *did the environment win over the file*, and *is this the
same key I set yesterday* — and nothing else. No command prints a key, and no
flag makes it: `config get gemini.api_key` returns `{set, source, fingerprint}`.

## Shape

### Where it lives

| platform | path |
| --- | --- |
| any, overridden | `$SPLICE_CONFIG` (a path to the file) |
| Linux/macOS | `$XDG_CONFIG_HOME/splice/config.json`, else `~/.config/splice/config.json` |
| Windows | `%APPDATA%\splice\config.json` |

Never project-local. A `./.env` next to `splice.json` is how a key gets committed,
and precedence that depends on the working directory is a bug waiting for a `cd`.

### The file

```json
{
  "version": 1,
  "providers": {
    "gemini":   { "api_key": "…", "model": "gemini-3.5-transcribe" },
    "deepgram": { "model": "nova-3" },
    "whisper":  { "model": "small" }
  }
}
```

Permissions: directory `0700`, file `0600`, written temp-then-rename. A loosened
file produces a `config_loose_perms` warning rather than being silently fixed.

### Precedence

| thing | order |
| --- | --- |
| key | environment variable → config file → unset |
| model | `--model` flag → config file → the pinned default (`gemini-3.5-transcribe`, `nova-3`, `small`) |

Environment wins for keys because that is what CI does, and a container that
mounts a token should not have to also write a file. Models have no environment
variable: three more knobs for something the file already answers.

### Modules

| module | owns |
| --- | --- |
| `src/config.rs` (new) | path resolution, load/save, `Secret`, precedence, fingerprint |
| `src/cli.rs` | the `config` command and `doctor`'s report |
| `src/project.rs` | `IndexRef.model`, so a transcript records what produced it |

```rust
pub struct Secret(String);          // no Display, no Debug, no Serialize
impl Secret {
    pub fn expose(&self) -> &str;   // blessed, greppable call sites
    pub fn fingerprint(&self) -> String;   // sha256[..8]
}

pub struct Resolved {
    pub key: Option<Secret>,
    pub key_source: KeySource,      // Env("VAR") | File | None
    pub model: String,
    pub model_source: Source,       // Flag | File | Default
}

pub fn resolve(provider: Provider, file: &Config) -> Resolved;
pub fn config_path() -> Result<PathBuf>;
pub fn load() -> Result<Config>;
pub fn save(&Config) -> Result<()>;
```

## Consumers

Config exists before the ASR call, and has two real consumers today — which is
what keeps it honest:

1. **`doctor`** stops reporting a bare boolean and reports the *resolution*: which
   source supplied the key, its fingerprint, the model and where that came from,
   plus the config path, whether it exists, and whether it is writable. A
   container with an unwritable home learns why its key was ignored.
2. **`index --model <id>`** records the model in `IndexRef`. This closes a
   present-tense defect: the transcript cache is keyed by audio hash + provider +
   language, so switching `nova-3` to `nova-4` would silently keep the old words.
   With the model recorded, a changed model forces a re-index.

The third consumer is `splice caption auto`, which is the next commit and the
reason any of this exists.

## Synthesis decision

Two designs were sketched. The surviving one — from an independent candidate —
puts the file and the precedence chain first and refuses to add the HTTP call in
the same change. The other, which failed to produce output but whose shape I
wrote out and screened, bundled config with `caption auto` and a `reqwest`
dependency so that the first version shipped the call.

The file-first design wins on one argument: the storage is small, testable, and
gives `doctor` and the index two improvements that are verifiable against
existing fixtures with no network at all. Bundling the call would have made the
first commit larger and untestable offline, and the network client is a separate
decision — it grows a 2.7 MB per-platform binary and has to build for static musl.

Rejected outright: **OS keyring** (`keyring` crate). Correct answer for a desktop
app, wrong for this one: no headless story, nothing to inspect in CI, and
`config list` could not tell you which key is in play. Rejected: **project-local
`.env`**, because it is one `git add` from a leak.

## Tradeoffs accepted

- **A plain-text file, not a keyring**, in exchange for being inspectable,
  CI-friendly and cross-platform without a daemon. The file is `0600` and lives
  outside the project; a laptop with a shared account was already lost.
- **No `--api-key` flag**, in exchange for one more line of shell in the
  documented path.
- **Models have no environment variable**, in exchange for fewer knobs.
- **`doctor` gets noisier**, in exchange for being the one place that answers
  "why is it using that key".

## Open questions

1. **`whisper` has no key and no endpoint** — it is an external binary. Its
   "model" is a model name passed to that binary. Does it belong in the same
   config, or is `SPLICE_WHISPER` a path plus a flag later?
2. **Rotation.** Changing a key does not invalidate cached transcripts. Should it?
   The cache is keyed by audio, provider, language and model; a key rotation is
   not a quality change, so probably not — but a *provider* change should be.
3. **`default_provider`** is deliberately absent. With several keys configured,
   which does `caption auto` pick — first configured, alphabetical, or an
   explicit `default_provider` in the file? Today nothing reads it, so it is not
   in the file.

## Next implementation step

`caption auto --provider <p> [--model] [--language] [--yes] [--max-duration]`:
normalise the audio to 16 kHz mono, call the provider, hand the response to the
existing `src/transcript.rs` parsers, cache it in `.splice/asr/`, and write the
index. That is the commit that makes the key file do real work.
