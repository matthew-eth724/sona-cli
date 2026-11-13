---
name: splice
description: Drive the `splice` video-editing CLI (init, cut, caption, composite, grade, render) through its one-JSON-object-per-command contract. Use when a task involves editing, retiming, captioning or rendering video on a machine where `splice` is installed, or when deciding between splice and raw ffmpeg.
---

# splice

`splice` is a video-editing CLI built to be driven by an agent: one command, one
concept, one JSON object on stdout. You edit a diffable `splice.json` with typed
commands and render through a per-step cache.

## Purpose and when to activate

Use this skill when a task means *editing* video — cutting, retiming,
captioning, compositing, grading, exporting stills — and `splice` is available.
Prefer it over hand-written `ffmpeg` invocations: it validates the document,
reports what a render will run before it runs, caches per step, and refuses edits
it cannot honour instead of silently ignoring them.

Do not use it for encoding jobs that are not editing (format transcoding,
streaming, muxing arbitrary inputs), or when the machine has no ffmpeg with
`libx264` and `libass`.

## First: is this install usable?

```console
$ splice --version
$ splice doctor
```

`doctor` prints one JSON object and **exits 4** when ffmpeg is missing or lacks a
required encoder. Do not start editing until `ok` is true and the things you need
are true:

| field | what it gates |
| --- | --- |
| `encoders.libx264` | any render |
| `filters.libass` | captions and text overlays (titles compile to ASS) |
| `font` (non-null) | text; `font_source` says where it came from |
| `providers[]`, `default_provider` | `transcribe`; `key_set:false` means no key |

If `doctor` reports `ffmpeg_missing` or `ffmpeg_incapable`, do not go hunting:
`splice ffmpeg install` fetches a pinned static build (with libass, so titles and
captions work), verifies its sha256, and puts it ahead of `PATH`. Then
`doctor` reports `ffmpeg_source: "splice cache"`. This matters most on macOS,
where Homebrew's ffmpeg can be built without libass.

It is about 56 MB to download and 160 MB on disk, it needs network once, and it is
explicit — nothing is fetched behind your back. It arrived in 0.1.5; on an older
build the command does not exist, so fall back to `SPLICE_FFMPEG` or a package
manager build that includes libass. To go back to the system build, delete
`~/.cache/splice/ffmpeg` (Linux), `~/Library/Caches/splice/ffmpeg` (macOS) or
`%LOCALAPPDATA%\splice\ffmpeg` (Windows), or point `SPLICE_FFMPEG` at the binary you
want.

`ffmpeg_source` tells you which binary wins: `SPLICE_FFMPEG`, then a copy beside
the `splice` binary, then the splice cache, then `PATH`. `doctor` reports the
path and which rule chose it.

To move the tool itself forward: `splice update --check` reports whether the npm
registry has a newer release, and `splice update` installs it with npm (it needs
`npm` on `PATH`, and it never downgrades a build that is ahead of the registry).

## The loop

```
doctor → inspect → edit → render --preview → frames → look → render
```

1. `splice inspect` before editing: cheap, returns `duration`, clips, `warnings[]`.
2. One command per change (`trim`, `speed`, `ramp`, `text`, `caption`, `layer`,
   `key`, `mask`, `grade`, …). Every mutation prints `{"ok":true,...}` plus
   `warnings[]` when something will not do what you asked.
3. `splice render --preview` while it is wrong — a proxy at a fraction of the wall
   time with its own cache. `splice render -o out.mp4` when it is right.
4. `splice frames --at 1,3.5 -o stills` when the change is visual, then **open the
   PNGs**. A render that exits 0 can still be wrong.
5. `splice plan` when you need to see the ffmpeg argv before spending time.

## The contract you code against

- **stdout is exactly one JSON object** for every command, success or failure.
  Two exceptions, both deliberate: `view` and `find` print plain text unless you
  pass `--json`.
- **`render` writes one progress object per line to stderr** by default — `start`
  and `ran`/`reused` per step — so a long render is distinguishable from a hang.
  `--no-progress` or `SPLICE_PROGRESS=0` silences it; `--progress` adds ffmpeg
  tick lines. stdout stays one object.
- **Every error carries `error.code` and `error.fix`**, the `fix` naming a command
  that clears it. Read `fix` before guessing. See `references/errors.md`.
- **Exit codes**: `0` ok, `2` project or precondition, `3` media missing or
  unreadable, `4` ffmpeg failed or incapable, `5` bad usage.
- **Never scrape stderr for results.** It carries progress lines and a short
  failure tail, nothing else.

## Things that surprise people

**Media paths are relative to the project when inside it.** `add ./media/a.mp4`
stores `media/a.mp4`; a file outside the project tree keeps an absolute path. The
whole directory can be moved and the project still resolves. Do not "fix up"
paths by hand.

**The bottom layer of the z order is the program frame.** The layer at the lowest
`z` fills the frame: its `place`, `place_keys` and `start` are reported, not
applied. `splice key --place` and `splice layer set --start/--place` are refused
on it with exit 5 — put overlays on a layer above (`splice layer add pip --z 10`),
and use `--zoom`/`--at` for a camera move. The layer at `z = 0` carries the
program audio and must fill the canvas; those are two different rules.

**`warnings[]` is the interesting field.** An over-long trim is clamped, an overlay
past the end is dropped, a layer with audio is heard only if it is the program
layer. Each succeeds with a warning. A warning ignored at edit time becomes a
mystery at render time.

**Captions and titles are one engine.** `splice text` and `splice caption` both
burn through libass, so `drawtext` is not needed. Styling is limited to what the
command exposes; there is no arbitrary ASS passthrough.

**`transcribe` creates `captions.srt`; it does not attach it** unless you pass
`--attach`. Making captions and burning them are separate decisions.

**The cache is never authoritative.** `splice cache info` reports bytes and files;
`splice cache prune` deletes entries no plan references (`--dry-run` to look,
`--all` to empty). Cutting it entirely only costs a re-render.

## Bundled resources

- `scripts/smoke.sh` — end-to-end check in a throwaway directory: init, add,
  title, render, assert one JSON object and a non-zero output. Run it once in a new
  environment, before blaming your own edit for a failure. Exits non-zero with the
  failing command's JSON when the install is not usable.
- `references/commands.md` — the command surface with the flags that matter, and
  the JSON shape each command prints.
- `references/errors.md` — every error code with its exit status, cause and `fix`,
  plus the warning codes worth acting on.

## Anti-patterns

- Hand-editing `splice.json` for something a command already does. Commands
  validate, take a history snapshot, and invalidate only the affected cache steps.
- Piping stdout through a parser that assumes one line per document: `render` is
  one object, `view` is many lines of text.
- Rendering the final profile to check a layout. `--preview` exists because finals
  are expensive.
- Treating a successful `render` as proof the edit landed. Look at a frame.
- Reaching for raw ffmpeg because a flag is missing. Check `splice <verb> --help`
  first, then `references/commands.md`; if it is genuinely absent, ffmpeg on the
  rendered file is the fallback, not on the sources.

## Validation checklist

- [ ] `splice doctor` returned `ok: true` with the encoders and filters you need
- [ ] each edit printed `ok: true`, and every `warnings[]` entry was read and either
      accepted deliberately or fixed
- [ ] `splice render` returned `ok: true` with `steps.total == steps.ran + steps.reused`
- [ ] at least one `splice frames` still was opened and shows the intended change
- [ ] the output's duration, size and fps match what the JSON reported (ffprobe, not
      vibes) — a mismatch means a warning you ignored
