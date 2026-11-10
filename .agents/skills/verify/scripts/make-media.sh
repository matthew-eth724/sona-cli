#!/usr/bin/env bash
# Deterministic fixtures for driving `sona`. Usage: make-media.sh <target-dir>
set -euo pipefail
DIR="${1:?usage: make-media.sh <target-dir>}"
mkdir -p "$DIR"
cd "$DIR"

ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i "testsrc2=size=640x360:rate=30:duration=6" \
  -f lavfi -i "sine=frequency=440:duration=6" \
  -c:v libx264 -pix_fmt yuv420p -c:a aac -shortest clip-a.mp4

ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i "smptebars=size=1280x720:rate=25:duration=5" \
  -f lavfi -i "sine=frequency=660:duration=5" \
  -c:v libx264 -pix_fmt yuv420p -c:a aac -shortest clip-b.mp4

ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i "sine=frequency=200:duration=8" \
  -c:a libmp3lame -b:a 128k music.mp3

cat > subs.srt <<'SRT'
1
00:00:00,500 --> 00:00:03,000
Ship faster.

2
00:00:04,000 --> 00:00:07,000
Second caption.
SRT

echo "fixtures ready in $DIR"
