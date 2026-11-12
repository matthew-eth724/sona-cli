// Install the bundled `splice` agent skill into the skill directories this
// machine actually has. Run by npm's postinstall, and by `splice-skill` for
// anyone who installed with --ignore-scripts.
//
// Rules, in order of importance:
//   1. Never fail an install. A missing or read-only home is worth a line of
//      output, not a failed `npm i`.
//   2. Only write where an agent harness already looks, so a project that has
//      never used one does not gain an empty directory tree in its home.
//   3. Report where it wrote, so a person can see the side effect and undo it.
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const SKILL = join(here, "skills", "splice");
const NAME = "splice";

// The directories agent harnesses load skills from. Only the ones whose parent
// already exists get a copy: `~/.claude/skills` means Claude Code is in use.
const DEFAULT_TARGETS = [
  [".claude/skills", "Claude Code"],
  [".agents/skills", "harnesses that read .agents"],
  [".pi/agent/skills", "Pi"],
];

export function readVersion(pkgPath) {
  try {
    return JSON.parse(readFileSync(pkgPath, "utf8")).version ?? "0.0.0";
  } catch {
    return "0.0.0";
  }
}

/** True when `b` is a byte-for-byte copy of `a`. */
function sameTree(a, b) {
  if (!existsSync(b)) return false;
  const left = statSync(a);
  const right = statSync(b);
  if (left.isDirectory() !== right.isDirectory()) return false;
  if (!left.isDirectory()) return readFileSync(a).equals(readFileSync(b));
  const names = readdirSync(a).sort();
  if (names.join("\0") !== readdirSync(b).sort().join("\0")) return false;
  return names.every((name) => sameTree(join(a, name), join(b, name)));
}

/** Copy `from` over `to` only when the contents differ. */
function syncTree(from, to) {
  if (sameTree(from, to)) return false;
  rmSync(to, { recursive: true, force: true });
  cpSync(from, to, { recursive: true });
  return true;
}

/**
 * @param {{ home?: string, dirs?: string[], quiet?: boolean }} options
 * @returns {{ installed: string[], skipped: string[], failed: string[] }}
 */
export function install({ home = homedir(), dirs = [], quiet = false } = {}) {
  const installed = [];
  const skipped = [];
  const failed = [];

  if (!existsSync(SKILL)) {
    failed.push(`skill payload missing at ${SKILL}`);
    return { installed, skipped, failed };
  }

  const targets = [];
  for (const [rel, label] of DEFAULT_TARGETS) {
    const root = join(home, rel);
    if (existsSync(root)) targets.push([root, label]);
    else skipped.push(rel);
  }
  for (const dir of dirs) {
    targets.push([resolve(dir), "requested with --dir"]);
  }

  for (const [root, label] of targets) {
    const dest = join(root, NAME);
    try {
      mkdirSync(root, { recursive: true });
      const changed = syncTree(SKILL, dest);
      installed.push(dest);
      if (!quiet) {
        const verb = changed ? "installed" : "up to date";
        process.stdout.write(`splice: agent skill ${verb} at ${dest} (${label})\n`);
      }
    } catch (error) {
      failed.push(`${dest}: ${error.message}`);
      if (!quiet) {
        process.stdout.write(
          `splice: could not install the agent skill at ${dest} (${error.message}); the CLI works without it\n`
        );
      }
    }
  }

  if (!quiet && installed.length === 0 && failed.length === 0) {
    process.stdout.write(
      "splice: no agent skill directory found (~/.claude/skills, ~/.agents/skills, ~/.pi/agent/skills); run `splice-skill --dir <dir>` to choose one\n"
    );
  }
  return { installed, skipped, failed };
}
