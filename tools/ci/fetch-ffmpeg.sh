#!/usr/bin/env bash
#
# fetch-ffmpeg.sh — install a pinned static macOS ffmpeg + ffprobe pair.
#
# Usage: fetch-ffmpeg.sh <arm64|x64> <destination-dir>
#
# Downloads the release .gz assets from eugeneware/ffmpeg-static (tag b6.1.1),
# gunzips them, checks the sha256 of each uncompressed binary against the pin
# map below, chmod +x and moves them into <destination-dir>. If
# <dest>/ffmpeg and <dest>/ffprobe already exist and match the pins, nothing
# is downloaded and the script prints "up to date" and exits 0. On any hash
# mismatch nothing is installed and the script exits 1.
#
# The binaries are just bytes on this side: no brew, otool, or any other
# macOS-only tool is used, so the same script runs on Linux and macOS CI.
#
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: fetch-ffmpeg.sh <arm64|x64> <destination-dir>" >&2
  exit 2
fi

arch=$1
dest=$2

case "$arch" in
  arm64|x64) ;;
  *)
    echo "usage: fetch-ffmpeg.sh <arm64|x64> <destination-dir>" >&2
    echo "fetch-ffmpeg.sh: unknown arch '$arch' (want arm64 or x64)" >&2
    exit 2
    ;;
esac

# --- URL base: one edit for a version bump -------------------------------
readonly VERSION="b6.1.1"
readonly BASE_URL="https://github.com/eugeneware/ffmpeg-static/releases/download/${VERSION}"

# --- Pin map: sha256 of the gunzipped (uncompressed) binary --------------
PIN_FFMPEG_ARM64="a90e3db6a3fd35f6074b013f948b1aa45b31c6375489d39e572bea3f18336584"
PIN_FFMPEG_X64="ebdddc936f61e14049a2d4b549a412b8a40deeff6540e58a9f2a2da9e6b18894"
PIN_FFPROBE_ARM64="bb2db6f5d8cef919da12fbf592119a987202a8c060a886f3cab091f9cab90b64"
PIN_FFPROBE_X64="fa3add0ce901f7241abe0dfc0155d958fc834aca3f8ce61f87cc712ae669c1e0"

case "$arch" in
  arm64)
    URL_FFMPEG="${BASE_URL}/ffmpeg-darwin-arm64.gz"
    URL_FFPROBE="${BASE_URL}/ffprobe-darwin-arm64.gz"
    PIN_FFMPEG="${PIN_FFMPEG_ARM64}"
    PIN_FFPROBE="${PIN_FFPROBE_ARM64}"
    ;;
  x64)
    URL_FFMPEG="${BASE_URL}/ffmpeg-darwin-x64.gz"
    URL_FFPROBE="${BASE_URL}/ffprobe-darwin-x64.gz"
    PIN_FFMPEG="${PIN_FFMPEG_X64}"
    PIN_FFPROBE="${PIN_FFPROBE_X64}"
    ;;
esac

sha256_of() {
  local file=$1
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$file" | awk '{print $1}'
  else
    echo "fetch-ffmpeg.sh: neither sha256sum nor shasum is available" >&2
    exit 1
  fi
}

hash_matches() {
  local file=$1 pin=$2
  [ "$(sha256_of "$file")" = "$pin" ]
}

mkdir -p "$dest"

# Warm cache: both binaries present and pinned -> nothing to do.
if [ -f "$dest/ffmpeg" ] && [ -f "$dest/ffprobe" ] \
   && hash_matches "$dest/ffmpeg" "$PIN_FFMPEG" \
   && hash_matches "$dest/ffprobe" "$PIN_FFPROBE"; then
  echo "up to date: $dest"
  exit 0
fi

# Stage everything, verify everything, then move into place: a failure
# leaves any previous install untouched.
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

fetch() {
  local tool=$1 url=$2 pin=$3
  local gz="$stage/${tool}.gz"
  local bin="$stage/${tool}"
  local actual

  curl -fsSL "$url" -o "$gz"
  gunzip -c "$gz" > "$bin"
  actual=$(sha256_of "$bin")
  if [ "$actual" != "$pin" ]; then
    echo "fetch-ffmpeg.sh: sha256 mismatch for ${tool} (${arch}):" >&2
    echo "  expected: ${pin}" >&2
    echo "  actual:   ${actual}" >&2
    exit 1
  fi
  chmod +x "$bin"
  echo "fetched ${tool} (${arch}) sha256 ${actual:0:8}…"
}

fetch ffmpeg "$URL_FFMPEG" "$PIN_FFMPEG"
fetch ffprobe "$URL_FFPROBE" "$PIN_FFPROBE"

mv -f "$stage/ffmpeg" "$dest/ffmpeg"
mv -f "$stage/ffprobe" "$dest/ffprobe"
echo "installed ffmpeg ffprobe (${arch}) -> $dest"
