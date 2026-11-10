#!/usr/bin/env node
// Build a platform package from an already-built binary:
//   node npm/stage.mjs <target-key> <path-to-binary>
// e.g. node npm/stage.mjs linux-x64 target/release/splice
//
// Produces npm/out/<pkg-name>/ ready for `npm publish`. The launcher package is
// copied from npm/launcher/ with the same version.
import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync, chmodSync } from "node:fs";
import { join } from "node:path";

const TARGETS = {
  // Linux binaries must be built for *-unknown-linux-musl: static-pie means one
  // artifact runs on glibc and musl distros both, with no libc floor.
  "linux-x64": { pkg: "splice-cli-linux-x64", os: ["linux"], cpu: ["x64"], exe: "splice" },
  "linux-arm64": { pkg: "splice-cli-linux-arm64", os: ["linux"], cpu: ["arm64"], exe: "splice" },
  "darwin-x64": { pkg: "splice-cli-darwin-x64", os: ["darwin"], cpu: ["x64"], exe: "splice" },
  "darwin-arm64": { pkg: "splice-cli-darwin-arm64", os: ["darwin"], cpu: ["arm64"], exe: "splice" },
  "win32-x64": { pkg: "splice-cli-win32-x64", os: ["win32"], cpu: ["x64"], exe: "splice.exe" },
};

const [key, binary] = process.argv.slice(2);
if (key && key.startsWith("linux") && !binary.includes("musl")) {
  console.warn(
    "warning: linux binaries should come from target/<triple>-musl/release/splice (static-pie)"
  );
}
if (!key || !binary || !TARGETS[key]) {
  console.error(
    `usage: node npm/stage.mjs <${Object.keys(TARGETS).join("|")}> <binary>`
  );
  process.exit(1);
}

const t = TARGETS[key];
const root = new URL("..", import.meta.url).pathname;
const version = JSON.parse(readFileSync(join(root, "npm/launcher/package.json"), "utf8")).version;
const out = join(root, "npm/out", t.pkg);

rmSync(out, { recursive: true, force: true });
mkdirSync(join(out, "bin"), { recursive: true });

cpSync(binary, join(out, "bin", t.exe));
chmodSync(join(out, "bin", t.exe), 0o755);

writeFileSync(
  join(out, "package.json"),
  JSON.stringify(
    {
      name: t.pkg,
      version,
      description: `splice binary for ${t.os[0]} ${t.cpu[0]}`,
      license: "MIT",
      repository: { type: "git", url: "git+https://github.com/matthew-eth724/splice-cli.git" },
      os: t.os,
      cpu: t.cpu,
      // npm refuses to install a package for the wrong platform, which is what
      // we want; no install script, no download.
      files: ["bin"],
    },
    null,
    2
  ) + "\n"
);

writeFileSync(
  join(out, "README.md"),
  `# ${t.pkg}\n\nThe ${t.os[0]}/${t.cpu[0]} binary for \`@matthew-eth724/splice-cli\`. Installed automatically; you do not depend on this directly.\n`
);

console.log(`staged ${out}`);
