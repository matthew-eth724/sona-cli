#!/usr/bin/env node
// Resolve the platform package for this machine and hand over to the real
// binary. Nothing is downloaded at install time: the platform packages are
// optionalDependencies, so npm's own resolution and integrity check does the
// work and `npm ci --ignore-scripts` still produces a working install.
"use strict";

const { spawnSync } = require("node:child_process");
const path = require("node:path");

// Linux ships a static (musl) binary, so one artifact covers glibc and musl
// distros and there is no libc field to fight with.
const TARGETS = {
  "linux-x64": "splice-cli-linux-x64",
  "linux-arm64": "splice-cli-linux-arm64",
  "darwin-x64": "splice-cli-darwin-x64",
  "darwin-arm64": "splice-cli-darwin-arm64",
  "win32-x64": "splice-cli-windows-x64",
};

function target() {
  const key = `${process.platform}-${process.arch}`;
  const pkg = TARGETS[key];
  if (!pkg) {
    fail(
      `splice does not ship a binary for ${key}`,
      "supported: " + Object.keys(TARGETS).join(", ")
    );
  }
  return pkg;
}

function fail(message, hint) {
  process.stderr.write(`splice: ${message}\n`);
  if (hint) process.stderr.write(`  ${hint}\n`);
  process.exit(1);
}

let pkgPath;
try {
  pkgPath = require.resolve(`${target()}/package.json`);
} catch {
  fail(
    `the platform package ${target()} is not installed`,
    "reinstall without --no-optional, or set SPLICE_BINARY to a binary you built yourself"
  );
}

const exe = process.platform === "win32" ? "splice.exe" : "splice";
const binary =
  process.env.SPLICE_BINARY || path.join(path.dirname(pkgPath), "bin", exe);

const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
  if (result.error.code === "ENOENT") {
    fail(`cannot execute ${binary}`, "set SPLICE_BINARY to an absolute path");
  }
  fail(result.error.message);
}
if (result.signal) {
  process.kill(process.pid, result.signal);
}
process.exit(result.status === null ? 1 : result.status);
