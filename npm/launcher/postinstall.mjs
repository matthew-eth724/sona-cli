// npm postinstall: install the bundled agent skill where a harness will see it.
// Deliberately swallows every error — an install must never fail over a skill.
import { install } from "./install-skill.mjs";

if (process.env.SPLICE_SKILL === "none" || process.env.SPLICE_SKILL === "0") {
  process.stdout.write("splice: agent skill install skipped (SPLICE_SKILL)\n");
} else {
  try {
    const dirs = process.env.SPLICE_SKILL_DIR ? [process.env.SPLICE_SKILL_DIR] : [];
    install({ dirs });
  } catch (error) {
    process.stdout.write(`splice: agent skill install skipped (${error.message})\n`);
  }
}
process.exit(0);
