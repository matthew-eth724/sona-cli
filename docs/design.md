# sona — a video editing CLI for agents

Design + rationale. Status: sketch, pre-implementation.

## Problem

An AI agent that edits video has three handicaps a human editor does not: it pays
tokens for everything it reads and writes, it cannot watch a timeline, and it
debugs by reading text. Existing tools fail on all three. ffmpeg's own CLI asks
the agent to author filter graphs by hand — the single most error-prone text an
LLM writes — and hands back a 200-line stderr log. NLE project formats (FCPXML,
OTIO, kdenlive) are hundreds of lines of XML for a 30-second cut.

So the tool is not "ffmpeg with nicer flags." It is a compiler: a small,
diffable project document in, a correct ffmpeg graph out, with every failure
reported as one structured line that names the fix.

Constraints that shaped the design:

- ffmpeg 7.1 on this machine has `libx264`, `libass`, `freetype`/`drawtext`,
  `xfade`, `overlay`, `loudnorm`, `scdet`, `tile`, and 143 system fonts. Every
  v1 feature below is reachable without extra dependencies. Verified, not assumed.
- Sources are heterogeneous (fps, SAR, rotation, VFR, channel count, color
  space). Concatenating them naively produces drift, desync, and green frames.
- Agents hand-edit files. Any state the CLI keeps outside the project file will
  drift from it, so the project file is the only source of truth.
- Three platforms, one of which breaks naive implementations: Windows needs
  `OsString` argv, `MoveFileEx`-style atomic replace, no console window on spawn,
  and — the real trap — path escaping inside ffmpeg filter strings, where `C:`
  and `\` collide with filter syntax. This is a first-class design constraint,
  not a portability chore.
- Captions may come from three unrelated ASR backends (Gemini Transcribe,
  Deepgram, local whisper) that return different shapes. The tool has to make
  them one shape before anything downstream sees them.

## Usage (caller's view)

### Quickstart

```console
$ sona init --size 1920x1080 --fps 30
wrote sona.json

$ sona add intro.mp4
clip_1  intro.mp4  00:00:08.240

$ sona add demo.webm --from 12s --to 28s --speed 1.5
clip_2  demo.webm  00:00:10.666

$ sona add outro.mp4 --at 1        # insert at the front
clip_3  outro.mp4  00:00:04.000

$ sona text "Ship faster" --from 0 --to 4 --pos center --style bold
text_1

$ sona caption subs.srt
captions  42 cues

$ sona audio music.mp3 --gain -18 --duck
audio_1

$ sona render -o out.mp4
```

### The three call sites that matter

**1. One-shot render from a spec** (agent generates a whole job, runs it once):

```console
$ sona render --spec job.json -o out.mp4
{"ok":true,"out":"out.mp4","duration":"00:00:42.500","size":"1920x1080",
 "steps":{"total":9,"ran":2,"reused":7},"warnings":[]}
```

**2. Iterative edit** (the common case — agent tweaks one clip, re-renders):

```console
$ sona list
clip_1  00:00:00.000-00:00:08.240  intro.mp4
clip_2  00:00:08.240-00:00:18.906  demo.webm  (1.5x, from 12s)
clip_3  00:00:18.906-00:00:22.906  outro.mp4

$ sona trim clip_2 --to 16s
{"ok":true,"changed":["clip_2"],"duration":"00:00:20.906",
 "invalidated":1,"warnings":[{"code":"caption_past_end","at":"captions[31]",
 "message":"cue ends 1.28s after the last clip","fix":"sona caption trim captions[31]"}]}

$ sona render -o out.mp4
{"ok":true,...,"steps":{"total":9,"ran":3,"reused":6}}
```

The edit cost 12 tokens and re-rendered only the affected clip plus assembly.
Both properties are the point.

**3. Look at the result** (the verification loop the agent actually needs):

```console
$ sona frames --at 2s,8s --sheet 12 -o .sona/preview
{"ok":true,"frames":[
 {"path":".sona/preview/at-2.000.png","time":"00:00:02.000"},
 {"path":".sona/preview/at-8.000.png","time":"00:00:08.000"},
 {"path":".sona/preview/sheet-01.png","times":["00:00:00.000",...]}]}
```

The agent reads PNGs as images. Stills are cheap in tokens and the only way an
agent perceives frame content; 6–12 stills or one contact sheet per review pass.

**4. Transcribe** (one flag to choose the backend, one shape to reason about):

```console
$ sona caption auto --provider whisper --model small
{"ok":true,"provider":"whisper","segments":184,"cues":42,"audio":"demo.webm"}

$ sona caption auto --provider deepgram --language en
$ sona caption auto --provider gemini --model gemini-transcribe
```

Provider credentials come from the environment only (`GEMINI_API_KEY`,
`DEEPGRAM_API_KEY`) and never enter `sona.json`. Without `--provider`, the first
configured backend wins; with none configured, the error names all three paths.
Transcripts cache under `.sona/asr/` keyed by source hash + provider + language, so
re-running never re-pays for a transcription.

Anything over **10 minutes of audio is refused** until confirmed:

```console
$ sona caption auto --provider deepgram
{"ok":false,"error":{"code":"asr_over_limit",
 "message":"audio is 01:12:30, limit is 00:10:00",
 "fix":"sona caption auto --yes   (or --max-duration 2h)"}}
```

Exit code `2`, nothing uploaded, nothing charged. `--yes` overrides for one call;
`--max-duration` moves the ceiling for one call. The cap applies to every
provider including local whisper — a three-hour whisper run is also a surprise,
and one rule is cheaper to learn than two.

### Contract

- **stdout is always one JSON object** (a `null` or `{}` for commands with
  nothing to report). Human prose only under `--human`. Never logs, never
  progress.
- **stderr is at most 3 lines**, always `<code>: <message>` then `fix: <command>`.
- Exit codes: `0` ok, `2` invalid project, `3` media problem, `4` ffmpeg failed,
  `5` usage.
- Relative times everywhere: `4`, `4s`, `4.5s`, `00:00:04.500`, `120f`, `25%`.
- Renders land at `-o`, default `out.mp4`. Nothing is written into the source tree.

## Shape

### Module map

```
src/main.rs        clap parse → one dispatch call. No logic.
src/cli/           one file per subcommand: parse → project::Store → JSON out
src/project.rs     Project, Clip, Overlay, Captions; Store (the only writer); validate
src/time.rs        Time, TimeRange, Rational fps, parsers
src/media.rs       ffprobe → Media, MediaIndex (probe cache)
src/graph.rs       Project → RenderPlan: normalize steps + assemble step
src/ffmpeg.rs      Invocation (argv only), run, capability probe
src/cache.rs       content-addressed step cache
src/render.rs      execute a RenderPlan; verify output
src/frames.rs      stills, contact sheets, scene detection
src/caption.rs     SRT/VTT/ASS parse+emit, cue layout from words
src/asr/mod.rs     Provider enum, dispatch, transcript cache
src/asr/gemini.rs  src/asr/deepgram.rs  src/asr/whisper.rs   → one Transcript
src/style.rs       presets → ASS style / drawtext params
src/paths.rs       platform-correct argv and filter-string escaping
src/report.rs      the JSON envelope and Diagnostic
```

`lib.rs` is absent. There is no public API but the binary and `sona.json`.
Tests reach internals through `mod` under `#[cfg(test)]` and through argv
snapshots, not through exports — `per importable-internals`.

### Types

```rust
// time.rs — one unit, no floats, ever.
pub struct Time(i64);                          // microseconds
impl Time { fn parse(s: &str) -> Result<Time>; fn from_frames(f: i64, fps: Rational) -> Time; }
pub struct TimeRange { start: Time, end: Time } // end-exclusive, start <= end by construction
pub struct Rational { num: u32, den: u32 }      // 30000/1001 must work
```

```rust
// project.rs — the document. One owner, one writer.
pub struct Project {
    version: u32,
    canvas: Canvas,           // width, height, fps: Rational, background
    tracks: Tracks,
}
pub struct Tracks {
    video: Vec<Clip>,         // ordered; the timeline order *is* this vec
    audio: Option<AudioBed>,  // music/voiceover under the clips
    overlays: Vec<Overlay>,   // text and image layers
    captions: CaptionTrack,   // cues + style; burn-in or soft mux
}
pub struct Clip {
    id: ClipId,               // opaque, allocated by Store, never reused
    source: PathBuf,
    src_in: Time,             // offset into the source file
    src_out: Option<Time>,    // None = to end of source, resolved at compile time
    speed: f64,               // 1.0 default; pitch-preserved via atempo
    transform: Transform,
    gain_db: f32,
    fade_in: Option<Time>, fade_out: Option<Time>,
}
pub enum Transform { Fit, Fill, Crop { x: f32, y: f32, w: f32, h: f32 }, Reframe(Preset) }
pub enum Preset { Vertical, Square, Wide, ZoomIn, PanLeft }   // closed set; adding one edits one file
pub enum Overlay { Text(TextOverlay), Image(ImageOverlay) }
pub struct TextOverlay { id, content: String, at: TimeRange, anchor: Anchor, style: StyleRef }
```

```rust
// project.rs — the only code that writes sona.json.
pub struct Store { path: PathBuf, project: Project }
impl Store {
    pub fn open(path: &Path) -> Result<Store>;   // parse + validate on every open
    pub fn mutate<T>(&mut self, f: impl FnOnce(&mut Project) -> Result<T>) -> Result<T>; // write-back, atomic rename
}
```

```rust
// asr — three backends, one canonical shape. Layout runs once, downstream.
pub enum Provider { Gemini, Deepgram, Whisper }
pub struct Caps { word_timestamps: bool, diarization: bool, vocab_hints: bool, max_audio: Time }
// gemini:   { true, true, false, 30min }   vocab rejected alongside timestamps
// deepgram: { true, true, true,  Time::MAX } vocab via `keyterm` on nova-3
// whisper:  { true, false, true, Time::MAX } initial_prompt biases decoding
impl Provider { pub fn caps(&self) -> Caps; }
pub struct Transcript { pub language: Option<String>, pub words: Vec<Word>, pub source: AsrMeta }
pub struct Word {
    pub text: String,
    pub start: Time,
    pub end: Time,
    pub confidence: Option<f32>,   // None: Gemini does not report it
    pub speaker: Option<Speaker>,  // None unless diarization was requested
}
pub struct Speaker(u32);           // Gemini "spk:1" and Deepgram 1 both parse to Speaker(1)
impl Transcript { pub fn cues(&self, opts: &CueOpts) -> Vec<Cue>; }   // grouping is provider-independent
pub fn transcribe(media: &Media, p: Provider, opts: &AsrOpts) -> Result<Transcript>;  // refuses above caps.max_audio

// asr/{gemini,deepgram,whisper}.rs — two whole formats, pinned on this machine:
fn parse_offset(s: &str) -> Result<Time>;      // Gemini: "3s", "0.700s" — fraction OPTIONAL
impl Time { fn from_secs_f64(x: f64) -> Time } // Deepgram: 0.71999997 — ROUND, do not truncate

// paths.rs — the only place a filesystem path becomes bytes of an ffmpeg argument.
pub fn arg(path: &Path) -> OsString;                     // -i inputs: verbatim, no quoting
pub fn filter_escape(path: &Path, cwd: &Path) -> String; // filter_complex/subtitles=: relative + escaped

// graph.rs — the compiler. Pure function, no I/O, fully testable.
pub fn plan(project: &Project, media: &MediaIndex, opts: &RenderOpts) -> Result<RenderPlan>;

pub struct RenderPlan { pub steps: Vec<Step>, pub output: OutputSpec }
pub struct Step {
    pub key: StepKey,            // sha256 of the resolved spec
    pub kind: StepKind,          // Normalize(ClipId) | Assemble | BurnCaptions | MuxAudio
    pub inputs: Vec<StepKey>,    // dependencies
    pub invocation: Invocation,  // exact argv, deterministic
}
```

```rust
// ffmpeg.rs — argv only. No shell, no string interpolation into a command.
pub struct Invocation { pub program: &'static str, pub args: Vec<OsString> }
impl Invocation { pub fn render(&self) -> String; }        // shell-quoted, for `sona plan`
pub fn run(inv: &Invocation, timeout: Duration) -> Result<Completed>;
pub fn capabilities() -> Capabilities;                     // cached; hard-fails on missing libass/x264
```

```rust
// report.rs — every command returns this.
pub struct Report { pub ok: bool, pub warnings: Vec<Diagnostic>, pub data: serde_json::Value }
pub struct Diagnostic { pub code: &'static str, pub at: String, pub message: String, pub fix: Option<String> }
```

### Data flow

```
sona.json ──load+validate──► Project ──plan(project, media)──► RenderPlan{argv[]}
                                   │                          │
                                   ▼                          ▼
                             MediaIndex (probe cache)    cache lookup by StepKey
                                                              │
                                        miss ──► ffmpeg run ──► .sona/cache/<key>.mp4
                                        hit  ──────────────────┘
                                                              ▼
                                                    assemble ──► out.mp4 ──verify──► Report
```

### Command surface (v1, complete)

| | |
| --- | --- |
| setup | `init` `doctor` |
| timeline | `add` `trim` `speed` `move` `rm` `list` |
| layers | `text` `image` `audio` `caption` |
| captions | `caption <srt>` `caption auto --provider` `caption style` |
| look | `inspect` `frames` `plan` |
| output | `render` `render --spec` `render --preview` |

Fifteen verbs. Every one takes `--project <path>` and prints one JSON object.

### Load-bearing decisions

**Two render stages, not one filter graph.** Every clip is first *normalized* to
one segment (`fps`, `scale`+`pad`/`crop` to canvas, `setsar=1`, VFR→CFR,
`speed` via `setpts`+`atempo`, gain, fades) encoded to an intermediate. Then a
single *assemble* pass concatenates (stream copy when there are no transitions,
`xfade` chain when there are), burns overlays and captions, and muxes audio.
One giant `filter_complex` over heterogeneous sources is where these tools die;
the split also makes each clip independently cacheable and re-encodable.
`per foundational-thinking`.

**Cache key = hash of the resolved step spec**, where "resolved" includes each
source's `(path, size, mtime, probe hash)` and the ffmpeg version string. Edit
one clip, re-render 2 of 9 steps. `.sona/` is pure cache — deletable at any time,
never authoritative. `per idempotent-operations`.

**The project file is the API; the CLI is sugar over it.** An agent may hand-edit
`sona.json` or run `sona add`; both are legal because every command parses and
validates the file on open, and nothing lives outside it. This is one task with
two paths on purpose — the alternative (CLI-only mutation) breaks the moment an
agent uses `jq`, which it will. The price is that `validate()` must be complete;
that is the trade. `per boundary-discipline`: `sona.json` is the boundary, so it is
parsed into domain types there and nowhere else.

**Types carry the invariants, validation carries the rest.** Illegal states that
are cheap to encode are encoded: `Time` is integer microseconds (no float drift),
`TimeRange` cannot be inverted, `Transform` is closed, `src_out` is `Option` so
"to end of source" survives the source growing. States that need media facts
(clip longer than its source, transition longer than either neighbor, text off
canvas, caption past the last clip) are `Diagnostic`s from `validate()`, because
the project must load before media is probed. Coarse codes, agent-actionable
`fix` strings. `per type-system-discipline`.

**ASR providers normalize to one `Transcript` before anything else touches
words.** Gemini, Deepgram, and whisper.cpp disagree on framing (sentences vs
segments vs words), on timestamp precision, and on whether they punctuate. Each
adapter's only job is producing `Vec<Word>`; cue grouping, wrapping, and ASS
styling run once, provider-independent. Adding a fourth provider is one file and
one enum arm — `per model-the-domain`.

**Capabilities are declared per provider, and unsupported flags error instead of
being ignored.** The three backends do not offer the same features: Gemini
rejects `custom_vocabulary` when word timestamps are on, and its `smart` mode is
incompatible with timestamps entirely. So each provider exposes `Caps`, and
`sona caption auto --vocab terms.txt --provider gemini` fails with
`asr_unsupported: gemini cannot combine vocabulary hints with word timestamps`
rather than quietly dropping the flag. A silently ignored flag is the class of
bug an agent cannot detect.

**Gemini always runs verbatim, and cleanup happens on our side.** Word
timestamps require `mode.type = "verbatim"`, which leaves disfluencies in. We
never ask Gemini to clean: filler removal, punctuation repair, and capitalization
run in `Transcript::cues()`, where all three providers get treated the same.

**The wire formats are pinned by live calls, not by reading docs.** Four real
requests against `resource/audio1.mp3` (31 s, mono), all committed as fixtures
in `tests/fixtures/asr/`: Gemini plain + diarized, Deepgram nova-3 diarized +
language-detected. The two providers disagree on nearly every field:

| | Gemini `gemini-3.5-transcribe` | Deepgram `nova-3` |
| --- | --- | --- |
| timestamps | `"3s"`, `"0.700s"` — strings, **fraction optional** | `0.71999997` — float seconds |
| confidence | absent | per word |
| speaker | `"spk:0"`, only with `diarization_mode` | `0` (int), only with `diarize` |
| punctuation | on the word (`"Hi."`) | separate `punctuated_word` |
| language | not returned | `detected_language: "en"`, `language_confidence` |
| vocab hints | rejected alongside timestamps | `keyterm` (`keywords` is a 400 on nova-3) |
| errors | HTTP **400** + `error.details[].reason` (`API_KEY_INVALID`) | HTTP 400 + `{err_code,err_msg,request_id}` |
| cost signal | `usage.total_input_tokens` (~25 tok/s) | `metadata.duration` |
| hard limit | 30 min with word timestamps | 2 GB file; **504** if processing exceeds 10 min |

Parse rules that follow, each with a fixture test:

- **Optional fraction.** 10 of 78 Gemini words used `"3s"`. A parser expecting
  `\d+\.\d{3}s` silently drops 13% of every transcript.
- **Round, don't truncate.** `0.71999997 * 1e6` truncates to `719_999 µs`.
  `Time::from_secs_f64` rounds.
- **Speaker is a number.** `"spk:1"` and `1` both become `Speaker(1)`; the string
  form never leaks past the adapter.
- **Prefer `punctuated_word`** when present, else `word`. Cue layout must not
  re-punctuate.
- **Bad-key detection keys off `reason`, not HTTP status.** Gemini answers an
  invalid key with **400**, not 401. Matching on status would report "bad
  request" for the most common user error in the world.
- **Deprecated parameters fail silently and plausibly.** `diarize=true` is
  deprecated on Deepgram and routes to the v1 diarizer; on a two-person clip it
  produced *every word on speaker 0* — no error, no warning, just a wrong
  transcript. `diarize_model=latest` gets v2 and separates the speakers
  correctly, matching Gemini's assignment. So: never `diarize`, always
  `diarize_model=latest`, and a golden test asserts the exact query string so a
  future edit to `deepgram.rs` cannot quietly reintroduce it. `per
  encode-lessons-in-structure`.
- **`keyterm`, repeated, not `keywords`.** `keywords` is a 400 on nova-3. 500-token
  budget across all terms per request; Deepgram's own guidance is 20–50 terms.

**Anything over 30 minutes is refused, not chunked.** `gemini-3.5-transcribe`
hard-caps word timestamps at 30 minutes per request, so longer audio has no
single-request path. v1 returns `asr_too_long` at exit `2` and says so. Chunking
on silence boundaries is real work (split, transcribe, stitch offsets, or a word
lands in two chunks) and buys nothing until someone asks for it. The 10-minute
confirm cap sits below this line, so in practice an agent hits the cheap
refusal first.

**Whisper is a separate program, not a linked library.** `whisper-cli` on
`PATH`, invoked like any other external tool. Keeps the build free of a C
dependency on three platforms, and a missing binary is a one-line diagnostic
instead of a build failure.

**Audio is downmixed to mono 16 kHz mp3 before any provider sees it.** ffmpeg
extracts it once per source and caches it like any other step. It normalizes
container quirks, makes the inline path always fit (10 min ≈ 1.2 MB, so the
Gemini Files API fallback never fires), and matches Gemini's own internal 16 kHz
downsample so nothing is lost. Deepgram answers in 1.5 s on this clip at that
size.

**No path ever enters a filter string unescaped, and never an absolute one on
Windows.** `filter_escape` converts to a path relative to the ffmpeg working
directory, then applies filter-syntax escaping (`:`, `\`, `'`). Every filter
graph is built through it; the rule is enforced in one function, not by
convention, because the failure is a cryptic "No such filter" on one platform.

**ASR cost is bounded by a pre-flight check, not by hope.** The duration check
runs against the probe result *before* any byte is uploaded or any subprocess
spawned, so the refusal is free. The limit is a constant (`10 min`), not config:
config invites an agent to quietly raise it, `--yes` does not. `per
boundary-discipline` — the check sits at the one door all three providers walk
through.

**`sona plan` prints the argv it would run.** This is the debugging lever and the
test surface: golden snapshots assert argv without running ffmpeg, so the
compiler is testable in milliseconds. An agent that reads 30 argv lines can
usually see its own bug; an agent that reads ffmpeg stderr cannot.

**Interface depth.** The public surface is a 15-command vocabulary, one JSON
document shape, and one JSON envelope. Hidden behind it: filter-graph
construction, VFR/SAR/rotation normalization, ASS generation and font
resolution, step caching, parallel execution, output verification. A caller who
learns the CLI never needs to learn the graph, and a caller who reads `sona plan`
output never needs to learn ffmpeg — the two views are independent. No option
exposes an internal stage: `--preview` changes quality, not pipeline shape.

### What it deliberately does not do

Multi-track video compositing, keyframes, masks, color grading beyond
`grayworld`/LUT, GPU encoding, streaming, and a GUI. Each would turn the project
file into a language and the "one diffable document" property dies first.

## Synthesis decision

Three candidates were sketched and compared on token cost per edit and on
crash/partial-failure behavior:

- **A. EDL compiler** — project document is the source of truth; render compiles
  it; CLI mutates it. *Chosen as the base.*
- **B. Operation log** — every command appends to `ops.jsonl`; render replays.
  Append-only is crash-safe and undo is free, but "what is the timeline now"
  requires a replay plus a `state` dump, so the agent pays for an indirection it
  never asked for, and a 400-line log is unreadable in a debugging loop.
- **C. Stateless primitives** — one operation per call, composed in shell, no
  project state. Fewest concepts, but n round trips and a full re-encode per
  edit: the worst token and wall-clock cost on exactly the loop this tool serves.

A is the base. From B it keeps atomic write-back plus idempotent, `--dry-run`-able
render, without the log. From C it keeps `sona render --spec` for the
generate-a-whole-job-in-one-shot case, so a caller never has to create a project
it will not revisit.

Note on process: the arena's runner fan-out was skipped. In this runtime every
architect runner alias resolves to the same model (`pstack-models.md → pi
models`), so four parallel runners would produce four copies of one perspective.
The three candidates above were written and screened inline against
`design-red-flags.md` instead.

Red flags screened: *two ways to do one task* and *split ownership* are both
live concerns here and are addressed in "load-bearing decisions" (file and CLI
converge on one validated document; `.sona/` is cache only, owned by `cache.rs`).
No hand-synced lists: presets, overlay kinds, and style refs each have one list.

## Tradeoffs accepted

- We accept two encodes for the segment that changed, in exchange for
  per-clip cache reuse and immunity to the heterogeneous-source class of bugs.
- We accept a project file that can be corrupted by a bad hand-edit, in exchange
  for it being the whole state and readable by `cat`.
- We accept per-clip intermediate files on disk (roughly 2–4× output size,
  prunable) in exchange for re-render speed.
- We accept a JSON project document over YAML in exchange for zero custom
  parsing and `jq` compatibility, despite ~10% more tokens when hand-written.
- We accept burning subtitles into pixels as the default (the only reliable path
  across players) in exchange for a second, non-cached render when text changes.
- We accept one process per ffmpeg invocation over libav bindings: no
  cross-build pain, and parallelism is process parallelism.
- We accept three ASR provider adapters (three network/local failure modes) in
exchange for never shipping model weights and never forcing a sandbox with no
outbound network to go without captions.
- We accept network I/O in a tool that was otherwise offline and deterministic,
  confined to `sona caption auto`, and make its results cacheable so a successful
transcription can be committed and replayed.
- We accept absolute paths resolving against a fixed working directory so filter
  escaping stays trivial, in exchange for `sona` chdir-ing its ffmpeg children.

## Alternatives considered

- **Stateless operations composed in shell** (candidate C, rejected above).
  Interface is trivially deep — one verb — but it exposes re-encode cost and
  ordering to the caller on every edit.
- **Timeline model instead of tracks + ordered clip list.** A track/crossfade
  timeline is the "correct" NLE model and would let two clips overlap. It also
  forces the agent to reason about z-order and gaps for a single-video-track use
  case. Rejected until a second video track is actually required; the ordered
  vec is the 95% shape at a third of the conceptual surface.
- **Emit OTIO/EDL and shell out to `ffmpeg` via a generic converter.** Rejected:
  it moves the hard part (normalization, caching, diagnostics) into a format
  nobody validates, and buys interoperability nobody asked for.
- **Go instead of Rust.** Nearly equivalent here; Rust wins on the closed-enum
  domain model, `OsString` argv handling (no shell), and serde round-tripping of
  the project document. Go would win on build times.

## Implementation reconciliation

Built: `time`, `paths`, `project`, `media`, `graph`, `ffmpeg`, `cache`, `render`,
`frames`, `caption`, `report`, `cli` — the whole v1 command surface except
`caption auto`. 13 unit tests, one verification skill with a 6-feature map and
14 captured artefacts.

Deviations accepted during fill-in:

- **Decorate step key includes its own argv.** The sketch keyed steps on their
  inputs. Adding a text overlay then reused the previous render (`ran: 0`):
  a cache key must cover everything that can change the output. Found by
  driving, not by reading.
- **A step is "cached" only when its output is non-empty.** ffmpeg creates the
  output file before failing, so a failed step left a zero-byte artefact the
  next render trusted.
- **`subtitles` is not an `-i` input.** Adding the SRT as one shifted the music
  bed's index and broke `[1:a]`. Caught by the audio-graph test.
- **`add` canonicalizes source paths.** `resolve()` joins the project directory,
  so a cwd-relative path added from elsewhere pointed at nothing.
- **No render parallelism yet.** The sketch allowed it; steps are executed
  serially because correctness came first and the cache already removes most of
  the cost.
- **Renders still land in `.sona/cache/` as intermediates.** Pruning is a later
  concern; `.sona/` remains deletable at any time.

Verified on real footage (`resource/video1.mp4`, 35.7 s portrait 576x1024@30,
aac 44.1 kHz) rather than only on synthetic testsrc:

- 1080x1920 upscale + text overlay: 13.5 s wall, output 1080x1920@30, text proven
  in pixels (15.5 dB inside its window, 50.9 dB outside).
- Source segments and speed changes: `--from 30s --to 35.7s --speed 2` plus
  `--speed 0.5`, claimed 8.85 s, rendered 8.78 s.
- `--fill` on a 1080x1080 canvas: `scale=…:increase,crop=…`, output 1080x1080.

Two behaviours changed after that run:

- **`--to` past the end of a source is clamped to the media duration**, so the
  reported duration matches the rendered one (claimed 1:40 vs rendered 35.7 s
  before the fix). The `clip_past_media` warning still fires.
- **Mutations return their own diagnostics.** Trimming past the end reported
  `ok:true` with no warning until `inspect` was run; `add`/`trim`/`speed`/`move`/
  `rm` now attach the resulting `warnings[]`.

Added after the first build, for the clipping workflow:

- **`sona index` / `view` / `find`.** A word index keyed by the SHA-256 of the
  extracted 16 kHz mono audio, with stable `w1..wN` ids, read back as compact text
  lines. `view` and `find` are the documented exception to "stdout is one JSON
  object": they are read views, so they print text, take `--json` for the
  envelope, and keep JSON for errors. Measured at 970 bytes against 2,275 for the
  same 30-second range, which is the whole point of the exception.

Unimplemented on purpose, still in the plan: ASR adapters (fixtures are
committed), image overlays, reframe presets beyond Fit/Fill, audio ducking,
transition/xfade between clips, `--preview` quality tier.

## Decisions made, with the evidence

| decision | evidence |
| --- | --- |
| Project file is the API; CLI is sugar | edit cost is the token cost that matters |
| Two-stage normalize→assemble render | heterogeneous sources, per-clip cache |
| `sona plan` prints argv | debugging lever + millisecond tests |
| 10 min ASR cap, `--yes` overrides | live calls on a 31 s clip |
| >30 min refused, not chunked | Gemini limits word timestamps to 30 min/request |
| Whisper via external binary | no C dependency on three platforms |
| Rust | closed enums, `OsString` argv, serde round-trip |

## Open questions and risks

1. **Audio scope** — music bed + ducking (`sidechaincompress`) is in the sketch;
   a full mixer with per-clip gain envelopes is not. Confirm before `audio` lands.
2. **Windows CI** — cross-platform argv tests can run on Linux; do we need a
   real Windows runner before v1, or after?
3. **Font determinism** — 143 system fonts make `style` presets machine-dependent.
   Risk: agent renders on one box, human reviews on another. Proposal: `sona doctor`
   prints resolved font file paths; presets pin to a bundled font later.
4. **Provider choice is a quality decision, not a cost one.** With the correct
   parameters the two providers agree on speakers (`0`/`1` vs `spk:0`/`spk:1` for
   the same lines), but still differ on word count (77 vs 78) and timing by
   ~100 ms. So `sona caption auto` records `source.provider` in the project and
   never blends two providers' output.
5. **Risk — the 10-minute *processing* limit, not audio length.** Deepgram
   returns `504 Gateway Timeout` when a pre-recorded request takes longer than
   10 minutes to process, and the 2 GB file cap is separate. An hour of audio
   may or may not fit inside that budget on a loaded day, so `caption auto`
   needs a timeout with a diagnostic that names the callback API as the fix
   rather than hanging an agent.
6. **Risk — verification blindness.** `sona frames` gives frames *with overlays
   burned*, which is what we want, but it re-renders. Mitigation: stills render
   from the last assemble cache entry at low priority and are not counted
   against the step cache.

## Next implementation step

`time.rs` + `paths.rs` + `project.rs` + `sona plan` with golden argv snapshots,
including the Windows-escaped case — the compiler skeleton and the one
platform-specific trap, both testable before any video is encoded.
