#!/usr/bin/env bash
# Prove this install can actually edit and render, in a throwaway directory.
#
# Builds six seconds of synthetic video and tone with ffmpeg, then drives the
# whole loop: init, add, title, render, frames. Every check is a JSON assertion,
# never an exit code alone, because a command can succeed and still do nothing.
#
#   ./smoke.sh              # uses `splice` from PATH
#   SPLICE=/path/to/splice ./smoke.sh
#
# Exit 0 = the install works. Non-zero = the failing command's JSON is printed.
set -euo pipefail

SPLICE="${SPLICE:-$(command -v splice || true)}"
[ -n "$SPLICE" ] || { echo "splice not on PATH; set SPLICE=/path/to/splice" >&2; exit 1; }
command -v ffmpeg >/dev/null || { echo "ffmpeg not on PATH" >&2; exit 1; }
command -v jq >/dev/null || { echo "jq not on PATH (needed to assert the JSON)" >&2; exit 1; }

# This skill documents 0.1.6 behaviour: relative media paths, render progress,
# the cache commands, and refusals for edits the compositor cannot apply. An older
# binary will fail the assertions below for a reason that has nothing to do with
# the install being broken, so say that plainly.
need="0.1.6"
have="$("$SPLICE" --version 2>/dev/null | awk '{print $2}')"
if [ -n "$have" ] && [ "$(printf '%s\n%s\n' "$need" "$have" | sort -V | head -1)" != "$need" ]; then
  printf 'splice %s is older than %s: install the current release (`npm i -g splice-cli@next`) or set SPLICE to a newer binary.\n' "$have" "$need" >&2
  exit 1
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/splice-smoke.XXXXXX")"
trap 'rm -rf "$work"' EXIT
cd "$work"

fail() { printf 'FAIL: %s\n' "$1" >&2; [ -n "${2:-}" ] && printf '%s\n' "$2" >&2; exit 1; }

# One command, one JSON object. Print it when it failed so the reason survives.
run() {
  local out; out="$("$@" 2>/dev/null)" || fail "$*" "$out"
  printf '%s' "$out"
}

# A source with both streams: ffmpeg errors on audio-only filters otherwise.
ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i "testsrc2=size=320x240:rate=25:duration=6" \
  -f lavfi -i "sine=frequency=440:duration=6" \
  -c:v libx264 -pix_fmt yuv420p -c:a aac -shortest src.mp4

run "$SPLICE" init --size 320x240 --fps 25 >/dev/null
run "$SPLICE" add ./src.mp4 >/dev/null

# Relative storage is the 0.1.4 behaviour: an inside-the-tree file is stored
# relatively, which is what makes the project movable.
source="$(jq -r '.tracks.layers[0].clips[0].source' splice.json)"
case "$source" in
  /*) fail "source stored as an absolute path: $source" ;;
esac

run "$SPLICE" text "SMOKE" --from 0 --to 2 --pos top-left >/dev/null
run "$SPLICE" render -o out.mp4 --no-progress | jq -e '.ok and .steps.total >= 1' >/dev/null \
  || fail "render did not report ok with steps"

# One JSON object on stdout, and a file that actually has a duration.
dur="$(ffprobe -v error -show_entries format=duration -of csv=p=0 out.mp4)"
awk -v d="$dur" 'BEGIN { exit !(d > 1) }' || fail "rendered file has duration '$dur'"

run "$SPLICE" frames --at 1 -o stills | jq -e '.ok and (.frames | length) == 1' >/dev/null \
  || fail "frames did not produce one still"
[ -s stills/*.png ] || fail "frames wrote an empty PNG"

# The contract: a refusal must be JSON with a fix, not a silent success.
refusal="$("$SPLICE" key main --t 0 --place 0.2,0.5 2>/dev/null || true)"
printf '%s' "$refusal" | jq -e '.ok == false and .error.code == "usage" and (.error.fix | length > 0)' >/dev/null \
  || fail "an edit the compositor cannot apply was not refused with a JSON fix" "$refusal"

printf 'PASS: %s — init, add, text, render (%.2fs), frames, refusal\n' \
  "$("$SPLICE" --version)" "$dur"
