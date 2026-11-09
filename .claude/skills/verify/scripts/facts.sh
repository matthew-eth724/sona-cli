#!/usr/bin/env bash
# ffprobe facts as one JSON object. Usage: facts.sh <file>
set -euo pipefail
f="${1:?usage: facts.sh <file>}"
[ -f "$f" ] || { echo "{\"ok\":false,\"error\":\"missing $f\"}"; exit 3; }
bytes=$(stat -c %s "$f" 2>/dev/null || stat -f %z "$f")
ffprobe -v error -print_format json -show_format -show_streams "$f" \
| python3 -c '
import json,sys
d=json.load(sys.stdin); f=d["format"]; v=next((s for s in d["streams"] if s["codec_type"]=="video"),{})
a=next((s for s in d["streams"] if s["codec_type"]=="audio"),{})
print(json.dumps({"ok":True,"file":sys.argv[1],"bytes":int(sys.argv[2]),
 "duration":round(float(f.get("duration",0)),3),
 "video":v.get("codec_name"),"width":v.get("width"),"height":v.get("height"),
 "fps":v.get("avg_frame_rate"),"audio":a.get("codec_name"),
 "sample_rate":a.get("sample_rate"),"channels":a.get("channels")}))
' "$f" "$bytes"
