# Portable install — every OS, one command

Status: **designed, not built.** Everyone already types one command today; this
design is about what happens behind it.

## Problem

`npm install -g splice-cli` is one command, but the promise behind it has three
holes. The user must install ffmpeg, ffprobe and a font system themselves, or
`splice` does nothing. The maintainer carries five artifacts and five runners,
and the set is arbitrary — no armv7, no RISC-V, nothing beyond glibc and Alpine.
And the same project renders differently on two machines because `drawtext` picks
whichever font the host has.

The ask — *runnable on all OSes, without any OS-specific anything, one install* —
has an honest version and a dishonest one. The dishonest one is a single artifact
that runs everywhere; that exists (WASM) and it costs 5–10× the render time,
which is the one thing this tool cannot spend. The honest one: **one command, no
prerequisites, no platform thinking by the user**, with the platform detail
absorbed by the package and by three runners.

## Usage

```console
$ npm install -g splice-cli
$ splice doctor
{"ffmpeg":"7.1-splice.3 (bundled)","filters":{"libass":true,"drawtext":true},
 "font":"NotoSans-Regular.ttf (bundled)","ok":true}

$ splice init --size 1080x1920 --fps 30
$ splice add podcast.mp4 && splice render --preview
```

Nothing else to install. No `brew install ffmpeg`, no fontconfig, no platform
detection, no "if you're on Windows do X". `doctor` names where the runtime came
from, so a user who *does* have a system ffmpeg can see they are not using it.

On a platform with no build — FreeBSD, RISC-V, 32-bit x86 — the launcher attempts
a compatible candidate and probes it, and if that fails says exactly that:

```console
$ splice render
splice: no build for freebsd-x64. Tried splice-cli-linux-x64 (probe failed).
  fix: npm i -g splice-cli --splice-ffmpeg=system   (needs ffmpeg on PATH)
       or cargo install splice-cli
```

## Shape

### The package, after

```
splice-cli/                     launcher, platform-independent, ~1.9 MB gzipped
  bin/splice.js  targets.json   targets.json generates the shim, stage.mjs, workflow
  fonts/  NotoSans-{Regular,Bold,Italic}.ttf  DejaVuSans.ttf  fonts.conf
splice-cli-linux-x64/           ~7.0 MB gzipped
  bin/splice  bin/ffmpeg  bin/ffprobe        all static (musl on linux)
splice-cli-linux-arm64/         same shape
splice-cli-linux-armv7/         same shape (musleabihf)
splice-cli-darwin-universal/    one lipo'd binary + ffmpeg, cpu: ["x64","arm64"]
splice-cli-windows-x64/           bin/*.exe
```

Two moves do most of the work.

**ffmpeg and ffprobe become siblings of `splice`.** Not an `ffmpeg-static`-style
resolver, not a postinstall download: the same tarball, one integrity hash, one
`os`/`cpu` gate. Resolution is `stat`, in order: `--ffmpeg`/`SPLICE_FFMPEG`, then
`current_exe().parent()/ffmpeg`, then `PATH`. `PATH` last keeps `cargo run` and
the 63 tests working, and it means `SPLICE_BINARY` and direct invocation of the
binary still behave. Downloading an unsigned binary after install would be worse
than bytes npm has already hashed.

**Fonts move into the launcher and `fc-match` disappears.** The two font paths
were always different problems:

| filter | needs | answer |
| --- | --- | --- |
| `drawtext` | a font *file* | `--font <face>` → a bundled face; default `NotoSans-Regular.ttf` |
| `subtitles`/`ass` | a font *provider* | libass built with fontconfig + a bundled `fonts.conf` via `FONTCONFIG_FILE` |

That deletes the fourth external process and one whole class of "works on my
machine": the same project burns in the same face everywhere, so ASS output
becomes reproducible. It also makes Windows uniform — libass never reaches for
CoreText or DirectWrite, because `fonts.conf` names exactly one directory.

### FFmpeg is not `ffmpeg-static`

`--disable-everything` plus an explicit `--enable-*` list derived from the argv
`src/graph.rs` can actually emit, with a CI test that greps the built binary's
`-filters`/`-encoders`/`-decoders` against that list and fails on a gap. Budget
~8.5 MB ffmpeg + 1.8 MB ffprobe stripped. **That number is the one to measure
before anything else** — if it comes out at 30 MB, the whole trade changes.

The build is pinned in `vendor/ffmpeg/build.sh` (ffmpeg 7.1, x264 and libass at
pinned revisions) and stamped with a build id, `7.1-splice.3`. The **build id
enters the step cache key**, so upgrading ffmpeg invalidates cached steps exactly
like upgrading splices does. Today two people with different ffmpeg builds share
cache hits and one of them gets a render from an encoder they did not choose.

### Three runners, four artifacts

| runner | produces | how |
| --- | --- | --- |
| `ubuntu-24.04` | linux x64 / arm64 / **armv7** | `cargo-zigbuild`; `zig cc` as cross-prefix for ffmpeg, x264, libass |
| `macos-14` | darwin-universal | both target slices, then `lipo -create`; no Intel runner needed |
| `windows-2022` | win32-x64 | native MSVC; cross-building ffmpeg for MSVC is not worth it |

Five runners become three, six artifacts become four, and armv7 stops being a
hole. The ffmpeg build is cached as a released artifact keyed on
`sha256(vendor/ffmpeg/**)`, because a 40-minute cold ffmpeg build on every push is
how a matrix becomes resented.

## Costs

| per machine, cold | today | this design |
| --- | --- | --- |
| launcher + fonts | 1.3 MB | 1.9 MB |
| platform package | 2.7 MB | ~7.0 MB |
| user separately installs ffmpeg | 25–80 MB, or nothing and nothing works | 0 |
| **on disk** | ~4 MB **+ their ffmpeg** | **~16 MB** |

Anyone who would have installed ffmpeg to use this tool — everyone, since without
it there is no tool — comes out ahead. Someone with a complete ffmpeg already
pays ~6.5 MB they don't need; npm cannot ask at resolve time, so `--ffmpeg
system` skips *using* the bundled one, not downloading it.

**What the user pays that is real:** a static build has no hardware encoders.
`videotoolbox`, `nvenc`, `qsv` and `vaapi` are all `dlopen`ed, which static
linking forbids. For a tool whose loop is proxy renders and short clips this is
acceptable, and it is why `--ffmpeg system` exists as a documented escape hatch
for anyone encoding long masters.

**What the user pays that is not real:** "one more dependency". The bundled
ffmpeg is smaller than the one `apt` would give them, because it is built for the
five filters this tool emits.

## Alternatives considered

| alternative | why it lost |
| --- | --- |
| **WASM end to end** — Rust→`wasm32-wasip1` + `@ffmpeg/ffmpeg`, `exec(argv)` over MEMFS | The only design that is genuinely one artifact on every OS. `Invocation { args }` maps onto `exec(argv)` cleanly and the cache is whole-file, so the fit is better than it looks. But: 64.7 MB core, 4 GB memory ceiling, single-threaded, and 35–70 s for a render that takes 7.1 s. The product's entire value is a cheap iteration loop; a 5–10× tax on every preview deletes it. Also `ffprobe` and `fc-match` are separate processes that each need a wasm answer, and `kill()` on timeout has no meaning. |
| **Node/TypeScript rewrite + `ffmpeg-static`** | Same per-platform artifacts as native, plus it throws away the typed document model and 63 tests to gain nothing the bundle does not already give. `ffmpeg-static` is a wrapper and its binaries are tens of MB. |
| **Per-platform bins inside one "fat" npm package** | One package name, but npm downloads every platform's bytes to every machine, and `optionalDependencies` already exists to avoid exactly that. |
| **Download the binary at `postinstall`** | Breaks behind proxies, ignores `--ignore-scripts`, needs its own integrity story. The current design exists to avoid this. |
| **Ship no ffmpeg, keep the install instructions** | Today's state, and the state the ask rejects: a tool that cannot do anything until the user reads a README and installs 80 MB. |

## Open questions

1. **Measure the slim ffmpeg before building it.** 8.5 MB is an estimate from a
   `--disable-everything` budget, not a number anyone here has seen. If it lands
   near 30 MB the honest answer may be to bundle ffmpeg only for the platforms
   where `apt`/`brew` is painful (Windows), and keep the install instruction
   elsewhere.
2. **Can libass be statically linked with fontconfig?** It pulls expat and
   freetype; the `FONTCONFIG_FILE` trick assumes a static libfontconfig, which
   needs proving on macOS and Windows.
3. **CJK, Arabic and Indic.** 2.4 MB of Latin faces is not "all OSes" for anyone
   writing Chinese captions. An optional `splice-cli-fonts-cjk` keeps the default
   small; is an optional font package acceptable, or does that smell like the
   per-platform packages we are trying to delete?
4. **Windows and long paths / drive escaping** are covered by `paths.rs` tests
   but have never run on Windows. Bundling does not change that, and the first
   release should say so rather than imply it is tested.
5. **Do we keep `--ffmpeg system`?** It is the only escape hatch for hardware
   encoders and for the platforms with no build, and it costs one `stat` chain.
   It also creates a second way to run ffmpeg; the design keeps it because the
   alternative is a tool that cannot encode a long master on a Mac.

## Next implementation step

Prove the two numbers the design rests on, before writing packaging code:

1. Build `vendor/ffmpeg/build.sh` with the `-filters`/`-encoders` list derived from
   `src/graph.rs`, strip it, and record the real size.
2. Link libass + fontconfig statically into one binary and run a `subtitles`
   burn-in with `FONTCONFIG_FILE` pointing at a bundled `fonts.conf`, on Linux.
3. Only then change `src/ffmpeg.rs` resolution order and delete `fc-match`.

If step 1 comes back at 8–10 MB, the rest is packaging. If it comes back at 30 MB,
the design changes before any code does.
