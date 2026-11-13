#!/usr/bin/env node
// Drive a *built* splice on a real machine and assert what the unit tests cannot:
// that this OS's ffmpeg, fonts and (optionally) a transcription provider produce a
// file that actually plays.
//
//   node tools/ci/verify.mjs platform      # doctor, render, odd paths, frames
//   node tools/ci/verify.mjs transcribe    # whisper: speech in, words out
//   node tools/ci/verify.mjs provider      # a live hosted call (GEMINI_API_KEY)
//   node tools/ci/verify.mjs all
//
// SPLICE overrides the binary; WHISPER_PROGRAM/WHISPER_MODEL point at a whisper.cpp
// build. VERIFY_REQUIRE_LIBASS=1 makes an ffmpeg_incapable doctor a hard FAIL
// instead of the degraded path. Exit 0 means every assertion held; a failure prints
// the JSON it saw.
import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
// A relative SPLICE breaks the moment the script enters a temp directory, which is
// most of what it does — CI passes `target/release/splice`. Make it absolute once.
const rawSplice = process.env.SPLICE ?? "splice";
const SPLICE = rawSplice.includes("/") || rawSplice.includes("\\") ? resolve(rawSplice) : rawSplice;
const root = join(here, "..", "..");
let failures = 0;

function check(name, ok, detail) {
  if (ok) {
    process.stdout.write(`  ok    ${name}\n`);
  } else {
    failures += 1;
    process.stdout.write(`  FAIL  ${name}\n`);
    if (detail !== undefined) {
      const text = typeof detail === "string" ? detail : JSON.stringify(detail, null, 2);
      process.stdout.write(`${String(text).slice(0, 4000)}\n`);
    }
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

  // An incapable ffmpeg is a legitimate state (a distro bottle without libass), so
  // it gets its own contract instead of failing the four capability assertions.
  const source = doc.ffmpeg_source ?? doc.report?.ffmpeg_source;
  if (doc.ok !== true && doc.error?.code === "ffmpeg_incapable") {
    if (process.env.VERIFY_REQUIRE_LIBASS === "1") {
      check("doctor sees a libass-capable ffmpeg (VERIFY_REQUIRE_LIBASS=1)", false,
        "VERIFY_REQUIRE_LIBASS=1 was set but this ffmpeg cannot burn subtitles: " +
        "fetch the pinned libass-capable ffmpeg (eugeneware/ffmpeg-static b6.1.1), " +
        "put it first on PATH ahead of the system bottle, and if a font burn still " +
        "fails, pass an explicit font to splice text/caption.");
      return;
    }
    degradedChecks(doc, source);
    return;
  }

  check("doctor exits with JSON and ok:true", doc.ok === true, doc);
  check("doctor sees libx264", doc.encoders?.libx264 === true, JSON.stringify(doc.encoders));
  check("doctor sees libass", doc.filters?.libass === true, JSON.stringify(doc.filters));
  check("doctor resolved a font", typeof doc.font === "string" && doc.font.length > 0, doc.font);
  check("doctor names where ffmpeg came from", typeof source === "string", source);

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

/** With an ffmpeg that lacks libass: the contract is a clean refusal, not a stack. */
function degradedChecks(doc, source) {
  process.stdout.write("  ffmpeg lacks libass: verifying the refusal path instead\n");
  const r = spawnSync(SPLICE, ["doctor"], { encoding: "utf8" });
  check("doctor exits 4 when ffmpeg cannot burn subtitles", r.status === 4, `exit ${r.status}`);
  check("the error names ffmpeg_incapable", doc.error?.code === "ffmpeg_incapable", JSON.stringify(doc.error));
  check("the error carries a fix", typeof doc.error?.fix === "string" && doc.error.fix.length > 0, doc.error?.fix);
  check("the report still shows what it did find", typeof doc.report?.ffmpeg === "string", JSON.stringify(doc.report?.ffmpeg));
  check("it still says where ffmpeg came from", typeof source === "string", source);

  const dir = mkdtempSync(join(tmpdir(), "splice-degraded-"));
  try {
    const src = makeSource(dir);
    const work = join(dir, "project");
    mkdirSync(work);
    execFileSync("cp", [src, join(work, "clip.mp4")]);
    json(["init", "--size", "320x240", "--fps", "25"], { cwd: work });
    json(["add", "./clip.mp4"], { cwd: work });
    check("editing works without libass", json(["list"], { cwd: work }).clips?.length === 1);
    // A plain render is fine without libass; it is the caption burn that needs it.
    json(["render", "-o", "plain.mp4"], { cwd: work });
    check("a render without captions still works", existsSync(join(work, "plain.mp4")));
    writeFileSync(join(work, "subs.srt"), "1\n00:00:00,500 --> 00:00:02,000\nhello\n");
    json(["caption", "subs.srt"], { cwd: work });
    const render = json(["render", "-o", "out.mp4"], { cwd: work, allowFailure: true });
    // No pre-flight capability check exists on the render path, so this fails as
    // ffmpeg_failed naming the missing filter. What matters is that it fails
    // cleanly, in JSON, with a fix — and leaves no partial output to trust.
    check(
      "a render that needs libass fails cleanly instead of writing a broken file",
      render.ok === false && render.error?.code === "ffmpeg_failed" && render.error?.fix,
      JSON.stringify(render.error)
    );
    check("it did not leave an output file behind", !existsSync(join(work, "out.mp4")));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  process.stdout.write("  skipped: text, captions, frames, moved-project render (need libass)\n");
}


/**
 * A short clip that carries speech, on any of the three OSes: SPEECH_WAV if given,
 * else macOS `say`, Windows System.Speech, or espeak-ng. The speech rides on a black
 * frame because `add` rejects audio-only media — which is what an agent would do with
 * a podcast too.
 */
function makeSpeechClip(dir) {
  const wav = join(dir, "speech.wav");
  const aiff = join(dir, "speech.aiff");
  const text = "the quick brown fox jumps over the lazy dog";

  if (process.env.SPEECH_WAV) {
    ffmpeg(["-hide_banner", "-loglevel", "error", "-y", "-i", process.env.SPEECH_WAV, "-ar", "16000", "-ac", "1", wav]);
  } else if (process.platform === "darwin" && spawnSync("say", ["-o", aiff, text]).status === 0) {
    ffmpeg(["-hide_banner", "-loglevel", "error", "-y", "-i", aiff, "-ar", "16000", "-ac", "1", wav]);
  } else if (process.platform === "win32") {
    const ps = [
      "Add-Type -AssemblyName System.Speech;",
      "$s = New-Object System.Speech.Synthesis.SpeechSynthesizer;",
      `$s.SetOutputToWaveFile('${wav}');`,
      `$s.Speak('${text}');`,
      "$s.Dispose();",
    ].join(" ");
    const r = spawnSync("powershell", ["-NoProfile", "-Command", ps], { encoding: "utf8" });
    if (r.status !== 0) throw new Error(`System.Speech failed: ${r.stderr}`);
  } else if (spawnSync("espeak-ng", ["-w", wav, text]).status === 0) {
    // espeak writes 22050 Hz mono; ffmpeg normalises it below.
  } else {
    throw new Error("no speech source: set SPEECH_WAV, or install espeak-ng");
  }

  const clip = join(dir, "speech.mp4");
  ffmpeg([
    "-hide_banner", "-loglevel", "error", "-y",
    "-f", "lavfi", "-i", "color=c=black:size=320x240:rate=25",
    "-i", wav, "-shortest", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", clip,
  ]);
  return clip;
}

/** A hosted provider: the one thing a unit test cannot do is cross the network. */
function providerChecks(provider = process.env.PROVIDER ?? "gemini") {
  process.stdout.write(`provider checks (${provider}), a live call\n`);
  const envName = provider === "deepgram" ? "DEEPGRAM_API_KEY" : "GEMINI_API_KEY";
  const key = process.env[envName];
  if (!key) {
    check(`${envName} is set`, false, "no key in the environment");
    return;
  }

  const dir = mkdtempSync(join(tmpdir(), "splice-provider-"));
  const config = join(dir, "config.json");
  try {
    const env = { SPLICE_CONFIG: config };
    // --from-env reads the variable itself: the key never appears in argv or in the
    // project file, which is the contract this checks as a side effect.
    json(["config", "set", `${provider}.api_key`, "--from-env", envName], { env });

    const doc = json(["doctor"], { allowFailure: true });
    // `doctor` reports providers as an object keyed by name, not an array.
    const seen = (doc.providers ?? doc.report?.providers ?? {})[provider];
    check("doctor sees the key", seen?.key_set === true, JSON.stringify(seen));

    const clip = makeSpeechClip(dir);
    const work = join(dir, "project");
    mkdirSync(work);
    execFileSync("cp", [clip, join(work, "speech.mp4")]);
    json(["init", "--size", "320x240", "--fps", "25"], { cwd: work, env });
    json(["add", "./speech.mp4"], { cwd: work, env });

    const result = json(["transcribe", "--provider", provider, "--attach"], { cwd: work, env, allowFailure: true });
    check("the live call succeeded", result.ok === true, JSON.stringify(result.error));
    check("it returned words", typeof result.words === "number" && result.words > 0, result.words);
    check("the provider is the one asked for", result.provider === provider, result.provider);

    const project = JSON.parse(readFileSync(join(work, "splice.json"), "utf8"));
    const caption = project.tracks?.captions?.source;
    check("captions attached and stored relatively", caption === "captions.srt", caption);
    check("transcript stored relatively", !String(project.tracks?.index?.transcript ?? "").startsWith("/"), project.tracks?.index?.transcript);

    const cap = json(["doctor"], { allowFailure: true });
    if (cap.ok === true && cap.filters?.libass === true) {
      const render = json(["render", "-o", "out.mp4"], { cwd: work, env });
      check("a render with the burned captions succeeds", render.ok === true, JSON.stringify(render.error));
    }
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
    const clip = makeSpeechClip(dir);
    const work = join(dir, "project");
    mkdirSync(work);
    execFileSync("cp", [clip, join(work, "speech.mp4")]);
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

    // Burning the captions needs libass, which is the platform check's business,
    // not this one's: transcribe's job is words in, attached captions out. Assert the
    // render only where libass exists, and say so where it does not.
    const cap = json(["doctor"], { allowFailure: true });
    if (cap.ok === true && cap.filters?.libass === true) {
      const render = json(["render", "-o", "out.mp4"], { cwd: work, env });
      check("a render with the burned captions succeeds", render.ok === true, JSON.stringify(render.error));
    } else {
      process.stdout.write("  skipped: burning the captions (this ffmpeg has no libass)\n");
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

const mode = process.argv[2] ?? "platform";
if (mode === "platform" || mode === "all") platformChecks();
if (mode === "transcribe" || mode === "all") transcribeChecks();
if (mode === "provider") providerChecks();

if (failures > 0) {
  process.stdout.write(`\n${failures} check(s) failed\n`);
  process.exit(1);
}
process.stdout.write("\nall checks passed\n");
