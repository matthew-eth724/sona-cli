#!/usr/bin/env node
// `splice-skill` — install the bundled agent skill yourself, for installs made
// with --ignore-scripts, or to put it somewhere other than a known harness dir.
//
//   splice-skill                 # every harness directory this machine has
//   splice-skill --dir ~/skills  # plus a directory you name
//   splice-skill --quiet         # exit code only; 1 when a copy failed
import { install } from "../install-skill.mjs";

const argv = process.argv.slice(2);
const dirs = [];
let quiet = false;
for (let i = 0; i < argv.length; i += 1) {
  switch (argv[i]) {
    case "--dir":
      dirs.push(argv[++i]);
      break;
    case "--quiet":
      quiet = true;
      break;
    case "-h":
    case "--help":
      process.stdout.write(
        "usage: splice-skill [--dir <path>]... [--quiet]\n\nInstalls the bundled `splice` agent skill into ~/.claude/skills,\n~/.agents/skills and ~/.pi/agent/skills when those exist, plus any --dir.\n"
      );
      process.exit(0);
      break;
    default:
      process.stderr.write(`splice-skill: unknown argument ${argv[i]}\n`);
      process.exit(2);
  }
}

const { installed, failed } = install({ dirs, quiet });
if (!quiet) {
  process.stdout.write(`splice-skill: ${installed.length} installed, ${failed.length} failed\n`);
}
process.exit(failed.length === 0 ? 0 : 1);
