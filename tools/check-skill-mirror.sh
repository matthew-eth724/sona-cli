#!/usr/bin/env bash
# .agents/skills/verify is a mirror of .claude/skills/verify for non-Claude
# harnesses. The only intended difference is the self-referential script path
# in each copy. This fails when the two trees drift apart.
set -euo pipefail
cd "$(dirname "$0")/.."

norm() {
  find "$1" -type f | sort | while read -r f; do
    rel="${f#"$1"/}"
    printf '=== %s\n' "$rel"
    sed -E 's#\.(claude|agents)/skills/#<tree>/skills/#g' "$f"
  done
}

if diff -q <(norm .claude) <(norm .agents) >/dev/null; then
  echo "ok: .agents mirrors .claude (modulo self-paths)"
else
  echo "DRIFT: .agents and .claude disagree" >&2
  diff <(norm .claude) <(norm .agents) >&2 || true
  echo "fix: rm -rf .agents && cp -a .claude .agents, then rewrite .claude/skills -> .agents/skills in every .agents/*.md" >&2
  exit 1
fi
