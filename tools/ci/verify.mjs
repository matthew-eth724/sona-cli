#!/usr/bin/env node
// Drive a *built* splice on a real machine and assert what the unit tests cannot:
// that this OS's ffmpeg, fonts and (optionally) a transcription provider produce a
// file that actually plays.
//
//   node tools/ci/verify.mjs platform      # doctor, render, odd paths, frames
//   node tools/ci/verify.mjs transcribe    # whisper: speech in, words out
//   node tools/ci/verify.mjs all
//
// SPLICE overrides the binary; WHISPER_PROGRAM/WHISPER_MODEL point at a whisper.cpp
// build. Exit 0 means every assertion held; a failure prints the JSON it saw.
import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const SPLICE = process.env.SPLICE ?? "splice";
const root = join(here, "..", "..");
let failures = 0;

function check(name, ok, detail) {
  if (ok) {
    process.stdout.write(`  ok    ${name}\n`);
  } else {
    failures += 1;
    process.stdout.write(`  FAIL  ${name}\n`);
    if (detail !== undefined) process.stdout.write(`${String(detail).slice(0, 2000)}\n`);
  }
}

/** Run a command, return parsed stdout JSON, and fail loudly on a non-JSON result. */
function json(args, { allowFailure = false, env = {}, cwd } = {}) {
  const r = spawnSync(SPLICE, args, { encoding: "utf8", cwd, env: { ...process.env, ...env } });
  let parsed;
  try {
    parsed = JSON.parse(r.stdout);
  } catch {
    const detail = `exit ${r.status}\nstdout: ${r.stdout}\nstderr: ${r.stderr}`;
    if (allowFailure) return { ok: false, error: { code: "unparseable", message: detail } };
    throw new Error(`${args.join(" ")} did not print JSON\n${detail}`);
  }
  if (!allowFailure && !parsed.ok) {
    throw new Error(`${args.join(" ")} failed: ${JSON.stringify(parsed.error)}`);
  }
  return parsed;
}

function ffmpeg(args) {
  const r = spawnSync("ffmpeg", args, { encoding: "utf8" });
  if (r.status !== 0) throw new Error(`ffmpeg ${args.join(" ")} failed: ${r.stderr}`);
}

function ffprobeDuration(path) {
  const r = spawnSync("ffprobe", ["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", path], {
    encoding: "utf8",
  });
  return Number.parseFloat(r.stdout.trim());
}

/** Six seconds of test pattern and tone: the cheapest input that has both streams. */
function makeSource(dir) {
  const src = join(dir, "src.mp4");
  ffmpeg([
    "-hide_banner", "-loglevel", "error", "-y",
    "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=25:duration=6",
    "-f", "lavfi", "-i", "sine=frequency=440:duration=6",
    "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest", src,
  ]);
  return src;
}

function platformChecks() {
  process.stdout.write(`platform checks with ${SPLICE}\n`);
  const version = execFileSync(SPLICE, ["--version"], { encoding: "utf8" }).trim();
  process.stdout.write(`  ${version}\n`);

  // doctor is the contract every other command leans on: if these are false, a
  // render fails later with a filter error that looks like the edit's fault.
  const doc = json(["doctor"], { allowFailure: true });
  check("doctor exits with JSON and ok:true", doc.ok === true, doc.error);
  check("doctor sees libx264", doc.encoders?.libx264 === true, JSON.stringify(doc.encoders));
  check("doctor sees libass", doc.filters?.libass === true, JSON.stringify(doc.filters));
  check("doctor resolved a font", typeof doc.font === "string" && doc.font.length > 0, doc.font);
  check("doctor names where ffmpeg came from", typeof doc.ffmpeg_source === "string", doc.ffmpeg_source);

  const dir = mkdtempSync(join(tmpdir(), "splice-verify-"));
  try {
    const src = makeSource(dir);
    const work = join(dir, "project");
    mkdirSync(work);
    // A space and an apostrophe in the name: the quoting that broke on Windows.
    const awkward = join(work, "it's a clip.mp4");
    execFileSync("cp", [src, awkward]);

    json(["init", "--size", "320x240", "--fps", "25"], { cwd: work });
    json(["add", "./it's a clip.mp4"], { cwd: work });

    const stored = JSON.parse(readFileSync(join(work, "splice.json"), "utf8"))
      .tracks.layers[0].clips[0].source;
    check("an inside-the-tree source is stored relatively", stored === "it's a clip.mp4", stored);

    // Text and captions both burn through libass, so this proves the font stack
    // and the ASS path on this machine, not just that ffmpeg is installed.
    json(["text", "VERIFY", "--from", "0", "--to", "2", "--pos", "top-left"], { cwd: work });
    writeFileSync(
      join(work, "subs.srt"),
      "1\n00:00:01,000 --> 00:00:03,000\none\ttwo\n\n2\n00:00:03,000 --> 00:00:05,000\nthree\n"
    );
    json(["caption", "subs.srt"], { cwd: work });

    const render = json(["render", "-o", "out.mp4"], { cwd: work });
    check("render reports ok", render.ok === true);
    check(
      "every step ran or was reused",
      render.steps && render.steps.total === render.steps.ran + render.steps.reused,
      JSON.stringify(render.steps)
    );

    const duration = ffprobeDuration(join(work, "out.mp4"));
    check("output is a real file of the reported length", Number.isFinite(duration) && duration > 5, duration);

    const frames = json(["frames", "--at", "1", "-o", "stills"], { cwd: work });
    check("frames wrote a still", frames.ok === true && Array.isArray(frames.frames) && frames.frames.length === 1, JSON.stringify(frames));

    // The project must survive being moved: the whole point of relative storage.
    const moved = `${work}-moved`;
    execFileSync("mv", [work, moved]);
    const listed = json(["list"], { cwd: moved });
    check("the moved project still resolves its media", listed.clips?.[0]?.missing === false, JSON.stringify(listed.clips?.[0]));

    // A refusal must be JSON with a fix, never a silent success.
    const refused = json(["key", "main", "--t", "0", "--place", "0.2,0.5"], { cwd: moved, allowFailure: true });
    check(
      "an unappliable edit is refused with a fix",
      refused.ok === false && refused.error?.code === "usage" && refused.error?.fix,
      JSON.stringify(refused.error)
    );

    const pruned = json(["cache", "info"], { cwd: moved });
    check("cache info reports the cache", pruned.ok === true && pruned.files >= 1, JSON.stringify(pruned));
    rmSync(moved, { recursive: true, force: true });
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

/** Whisper, because it needs no key: speech in, words out, paths stored relatively. */
function transcribeChecks() {
  process.stdout.write("transcribe checks\n");
  const program = process.env.WHISPER_PROGRAM;
  const model = process.env.WHISPER_MODEL;
  if (!program || !existsSync(program)) {
    check("WHISPER_PROGRAM points at a whisper binary", false, program ?? "unset");
    return;
  }
  if (!model || !existsSync(model)) {
    check("WHISPER_MODEL points at a ggml model", false, model ?? "unset");
    return;
  }

  const dir = mkdtempSync(join(tmpdir(), "splice-asr-"));
  const config = join(dir, "config.json");
  try {
    const env = { SPLICE_CONFIG: config };
    json(["config", "set", "whisper.program", program], { env });
    json(["config", "set", "whisper.model", model], { env });

    // Speech without committing a recording: macOS `say` and Linux espeak can both
    // make it, and whisper does not care which.
    const wav = join(dir, "speech.wav");
    const aiff = join(dir, "speech.aiff");
    // SPEECH_WAV is the escape hatch: a machine with neither `say` nor espeak-ng
    // can still prove the transcription path with a recording of its own.
    const said = process.env.SPEECH_WAV
      ? { status: 0 }
      : spawnSync("say", ["-o", aiff, "the quick brown fox jumps over the lazy dog"], { encoding: "utf8" });
    if (process.env.SPEECH_WAV) {
      ffmpeg(["-hide_banner", "-loglevel", "error", "-y", "-i", process.env.SPEECH_WAV, "-ar", "16000", "-ac", "1", wav]);
    } else if (said.status === 0) {
      ffmpeg(["-hide_banner", "-loglevel", "error", "-y", "-i", aiff, "-ar", "16000", "-ac", "1", wav]);
    } else {
      const espeak = spawnSync("espeak-ng", ["-w", wav, "the quick brown fox jumps over the lazy dog"], { encoding: "utf8" });
      if (espeak.status !== 0) {
        check("a speech source ('say', espeak-ng, or SPEECH_WAV)", false, espeak.stderr);
        return;
      }
      ffmpeg(["-hide_banner", "-loglevel", "error", "-y", "-i", wav, "-ar", "16000", "-ac", "1", wav]);
    }
    check("speech input exists", existsSync(wav) && ffprobeDuration(wav) > 0.5);

    // `add` rejects audio-only media — a clip must have a picture — so carry the
    // speech on a still frame. This is also what an agent would do with a podcast.
    const work = join(dir, "project");
    mkdirSync(work);
    ffmpeg([
      "-hide_banner", "-loglevel", "error", "-y",
      "-f", "lavfi", "-i", "color=c=black:size=320x240:rate=25",
      "-i", wav, "-shortest", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac",
      join(work, "speech.mp4"),
    ]);
    json(["init", "--size", "320x240", "--fps", "25"], { cwd: work, env });
    json(["add", "./speech.mp4"], { cwd: work, env });

    const result = json(["transcribe", "--provider", "whisper", "--attach"], { cwd: work, env, allowFailure: true });
    check("transcribe reports ok", result.ok === true, JSON.stringify(result.error));
    check("it found words", typeof result.words === "number" && result.words > 0, result.words);

    const doc = JSON.parse(readFileSync(join(work, "splice.json"), "utf8"));
    const caption = doc.tracks?.captions?.source;
    check("captions are attached", typeof caption === "string", caption);
    check("the caption path is stored relatively", caption === "captions.srt", caption);
    check("transcript path is stored relatively", !String(doc.tracks?.index?.transcript ?? "").startsWith("/"), doc.tracks?.index?.transcript);

    // Captions that cannot render are worse than none: render them.
    const render = json(["render", "-o", "out.mp4"], { cwd: work, env });
    check("a render with the burned captions succeeds", render.ok === true, JSON.stringify(render.error));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

const mode = process.argv[2] ?? "platform";
if (mode === "platform" || mode === "all") platformChecks();
if (mode === "transcribe" || mode === "all") transcribeChecks();

if (failures > 0) {
  process.stdout.write(`\n${failures} check(s) failed\n`);
  process.exit(1);
}
process.stdout.write("\nall checks passed\n");
