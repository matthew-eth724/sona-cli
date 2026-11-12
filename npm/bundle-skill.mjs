#!/usr/bin/env node
// Copy the repo's canonical skill into the launcher package so `npm publish`
// ships it. One source of truth: .claude/skills/splice. The copy is gitignored.
import { cpSync, existsSync, rmSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const from = join(root, ".claude/skills/splice");
const to = join(root, "npm/launcher/skills/splice");

if (!existsSync(join(from, "SKILL.md"))) {
  console.error(`bundle-skill: no SKILL.md at ${from}`);
  process.exit(1);
}
rmSync(to, { recursive: true, force: true });
cpSync(from, to, { recursive: true });
console.log(`bundled ${from} -> ${to}`);
