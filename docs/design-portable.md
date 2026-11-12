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

## Measured, before any packaging

Built ffmpeg 7.1.1 from source with `--disable-everything`, the decoders, encoders,
muxers and filters this tool's argv actually names, `--enable-libx264`,
`--enable-libfreetype`, and `--extra-ldflags=-static`. Built, stripped by the
build script, and run — not estimated.

| artifact | raw | gzipped |
| --- | --- | --- |
| `ffmpeg` (slim, static) | **12 MB** | 4.7 MB |
| `ffprobe` (same config) | **12 MB** | 4.6 MB |
| both, per platform | **24 MB** | ~9.3 MB |
| the design's estimate | 10 MB | — |

The estimate was 2.4x optimistic. Static glibc alone is a 5.5 MB archive and x264
is 2.7 MB; the system `ffmpeg` is 360 KB only because it borrows shared libraries
at runtime.

Two findings that matter more than the total:

1. **libass cannot link statically from Debian's packages.** Its
   `pkg-config --static` chain wants harfbuzz, glib, graphite2, sysprof and pcre2,
   none of which ship `.a` files. A static build means building libass from source
   with harfbuzz off, alongside fribidi and freetype, which do have archives. So
   the subtitle support this design treated as free is the one dependency needing
   real work, and it adds another 1–2 MB.
2. **`ffprobe` costs as much as `ffmpeg` inside a static bundle**, because both
   carry their own copy of libc. Bundling probe doubles the price for a job that is
   a few hundred lines of JSON parsing.

So the choice is three shapes, not one:

| shape | download | what the user still installs |
| --- | --- | --- |
| bundle ffmpeg + ffprobe everywhere | ~9.3 MB | nothing |
| bundle ffmpeg, probe with it | ~4.7 MB | nothing; stderr parsing becomes ours |
| bundle on Windows only | ~9.3 MB there, 1.1 MB elsewhere | nothing on Windows, `brew`/`apt` elsewhere |

## The rebuild, proven

The claim "Debian's libass cannot link statically" was about Debian's build, not
about libass. Built from source, here:

| component | configure | result |
| --- | --- | --- |
| HarfBuzz 15.1.0 | glib/gobject/icu/graphite2/cairo/subset/tests/docs disabled, freetype enabled, static | `libharfbuzz.a` 3.6 MB |
| libass 0.17.5 | fontconfig + fribidi + that HarfBuzz, static | `libass.a` 491 KB |
| libunibreak 6.1 | required by libass, no static package in Debian | 230 KB |
| ffmpeg 7.1.1 | `--disable-everything` plus those, `-static` | 11.8 MB stripped, `ass` and `subtitles` present |

The subtitle stack costs about two megabytes on top of the twelve, and
"impossible" was a packaging choice rather than a property of the library.

**Shaping was verified two ways, not by trusting identical output.** A subtitled
frame differs from the same frame without one by 22 dB, with 1884 ink pixels — so
text was drawn, and `PSNR = inf` against the system build cannot have been two
blank frames agreeing. Then a zero-width non-joiner control: `لا` versus
`ل<ZWNJ>ا`, and the Devanagari conjunct `क्ष` versus its broken form, differ by 26.9
and 27.9 dB, while identical Latin text scores `inf`. ZWNJ changes the picture only
if a shaper is reached — and every script matched the system build bit for bit.

## What the rebuild still does not give

**`drawtext` is absent from this build.** ffmpeg 7.1's `drawtext` needs
`libfreetype` *and* `libharfbuzz`, and this configure dropped the filter even with
both present. `splice text` uses `drawtext`, so a bundled binary built this way
could do every job except the text titles.

The clean answer is not another configure flag: **compile text overlays to ASS** and
run them through libass, which is already linked and already carries both
dependencies. Captions and overlays would share one text engine instead of two,
`drawtext` leaves the dependency list, and the bundled filter set becomes complete.
That is a contained change to the overlay lowering, and it should land before the
bundle rather than after.

## Resolution order, built and proven

`ffmpeg` and `ffprobe` now resolve in the order that makes a bundled copy win
without touching `PATH`:

1. `SPLICE_FFMPEG` — a directory holding both, or a path to `ffmpeg` (its sibling
   `ffprobe` is preferred)
2. a sibling of the running `splice` binary
3. `PATH`

`doctor` reports the chosen path and which rule chose it, so "why is it using that
binary" has an answer.

Verified end to end on this machine: with a copy of ffmpeg and ffprobe in the same
directory as `splice`, `doctor` reports `source: beside the binary`, both paths
point into that directory, and a real render — with a title — goes through it.
Pointing `SPLICE_FFMPEG=/usr/bin` reports `source: SPLICE_FFMPEG` and switches.

## Titles and captions share one engine

`splice text` used `drawtext`; captions used libass. Two text engines meant two
escaping rules, two font lookups, two positioning systems, and a bundled ffmpeg
that had to satisfy `drawtext`'s freetype *and* harfbuzz dependency — which the
slim build could not.

Titles now compile to an ASS document, written into the cache as a content-addressed
sidecar, and burn through the same `ass` filter captions use. `paths::text_escape`
and the whole drawtext chain are gone, and `Capabilities::missing()` no longer
requires drawtext — it would otherwise reject the very binary this tool ships.

Verified: the decorate filter is `ass=filename='…ass'` with no `drawtext` in it, a
title changes the pixels by 15.6 dB, and a title anchored `top-left` puts 18,004 ink
pixels in the top-left quadrant and zero in the bottom-right.

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

## The recipe, working

A slim static ffmpeg now builds, links and runs, and the whole tool works on it
with no ffmpeg anywhere on `PATH`:

| | |
| --- | --- |
| `ffmpeg` | 14 MB (5.4 MB gzipped) |
| `ffprobe` | 13 MB (5.4 MB gzipped) |
| has | libx264, libass + harfbuzz, hevc/h264 decoders, zoompan, colorkey, lut3d |
| has not | drawtext — which nothing needs any more |

Two configure findings that cost real time and would block any future attempt:

1. **H.264 alone does not link.** `OBJS-$(CONFIG_H264_SEI)` adds `h2645_sei.o`,
   which calls `ff_aom_uninit_film_grain_params`, but only the *HEVC* line adds
   `aom_film_grain.o`. An H.264-only build therefore references a symbol it never
   links. Enabling `hevc` fixes it, and we want HEVC anyway.
2. **`ass`/`subtitles` need ffmpeg's subtitle *decoders*.** libass renders the
   document, but the filter still opens the file through libavformat: without
   `ass`/`subrip`/`webvtt` decoders it fails with "Decoder not found". Enable the
   decoders, not just the library.

Verified end to end with `PATH` containing only `fc-match`: `doctor` reports
`source: beside the binary` and the sibling `ffprobe`, a project with a title and
Arabic/Hindi captions renders, and `frames` writes a still. Against the same cut
without overlays the title band differs by 14.1 dB and the caption band by 21.2 dB.

One bug this exposed in our own code: `doctor` still probed a literal `"ffprobe"`
instead of the resolved one, so a bundled probe was invisible to it — the exact
class of miss the resolution order exists to prevent.

## Next implementation step## Next implementation step

Prove the two numbers the design rests on, before writing packaging code:

1. Build `vendor/ffmpeg/build.sh` with the `-filters`/`-encoders` list derived from
   `src/graph.rs`, strip it, and record the real size.
2. Link libass + fontconfig statically into one binary and run a `subtitles`
   burn-in with `FONTCONFIG_FILE` pointing at a bundled `fonts.conf`, on Linux.
3. Only then change `src/ffmpeg.rs` resolution order and delete `fc-match`.

If step 1 comes back at 8–10 MB, the rest is packaging. If it comes back at 30 MB,
the design changes before any code does.
