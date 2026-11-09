#!/usr/bin/env bash
# Pixel diff of one timestamp, cropped. Usage: psnr-ab.sh <a> <b> <seconds> <crop>
# Prints "average:<dB>". ~40+ means the crop is identical; <20 means it changed.
set -euo pipefail
a="${1:?usage: psnr-ab.sh <a> <b> <seconds> <crop>}"
b="${2:?}"; t="${3:?}"; crop="${4:?}"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
i=0
for f in "$a" "$b"; do
  ffmpeg -hide_banner -loglevel error -y -ss "$t" -i "$f" -frames:v 1 -vf "crop=$crop" "$tmp/$i.png"
  i=$((i+1))
done
ffmpeg -hide_banner -i "$tmp/0.png" -i "$tmp/1.png" -filter_complex psnr -f null - 2>&1 \
  | grep -o 'average:[0-9.]*' | head -1
