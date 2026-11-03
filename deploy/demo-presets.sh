#!/usr/bin/env bash
# Writes a copy of the built-in presets pointing at the mock server, for demos and UI tests:
#   deploy/demo-presets.sh http://127.0.0.1:8794 > /tmp/presets.json && VR_PRESETS_FILE=/tmp/presets.json viberooms
set -euo pipefail
MOCK=${1:-http://127.0.0.1:8794}
python3 - "$MOCK" "$(dirname "$0")/../server/src/providers/presets.json" <<'PY'
import json, re, sys
mock, src = sys.argv[1], sys.argv[2]
presets = json.load(open(src))
fix = lambda u: re.sub(r"^https://", mock.rstrip("/") + "/fx/", u)
for p in presets:
    s = p["spec"]
    if s.get("url"): s["url"] = fix(s["url"])
    for r in s.get("requests", []): r["url"] = fix(r["url"])
print(json.dumps(presets, ensure_ascii=False, indent=1))
PY
