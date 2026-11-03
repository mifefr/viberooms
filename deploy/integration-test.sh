#!/usr/bin/env bash
# End-to-end test against the mock providers: sign-in, token rotation, collection, recommendation.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
TMP=$(mktemp -d)
MOCK_PORT=18794
PORT=18799
cleanup() { kill "${MOCK_PID:-}" "${VR_PID:-}" 2>/dev/null || true; rm -rf "$TMP"; }
trap cleanup EXIT

python3 "$ROOT/deploy/mock_server.py" $MOCK_PORT >"$TMP/mock.log" 2>&1 & MOCK_PID=$!
VR_DATA_DIR="$TMP/data" VR_PORT=$PORT VR_POLL_MS=5000 VR_LANG=en \
VR_CLAUDE_API_BASE=http://127.0.0.1:$MOCK_PORT VR_CLAUDE_TOKEN_URL=http://127.0.0.1:$MOCK_PORT/v1/oauth/token \
VR_CODEX_AUTH_BASE=http://127.0.0.1:$MOCK_PORT VR_CODEX_USAGE_URL=http://127.0.0.1:$MOCK_PORT/backend-api/wham/usage \
  "$ROOT/target/release/viberooms" >"$TMP/server.log" 2>&1 & VR_PID=$!
for _ in $(seq 1 50); do curl -fs "localhost:$PORT/api/health" >/dev/null && break; sleep 0.2; done

post() { curl -fs -X POST -H "x-viberooms: 1" -H "content-type: application/json" "localhost:$PORT$1" -d "$2"; }
login() { # provider code
  local r flow state paste
  r=$(post /api/login/start "{\"provider\":\"$1\",\"label\":\"$2\"}")
  flow=$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["flowId"])' "$r")
  state=$(python3 -c 'import json,sys,urllib.parse as u; print(u.parse_qs(u.urlparse(json.loads(sys.argv[1])["url"]).query)["state"][0])' "$r")
  if [ "$1" = codex ]; then paste="http://localhost:1455/auth/callback?code=$2&state=$state"; else paste="$2#$state"; fi
  post /api/login/finish "{\"flowId\":\"$flow\",\"code\":\"$paste\"}" >/dev/null
}
login claude perso; login claude msds; login claude third; login codex gpt-plus; login codex gpt-pro
# Let several polls happen: tokens last 90 s in the mock, so every poll refreshes (rotation).
sleep 12
curl -fs "localhost:$PORT/api/state" > "$TMP/state.json"
python3 - "$TMP/state.json" <<'PY'
import json, sys
s = json.load(open(sys.argv[1]))
acc = {a["label"]: a for a in s["accounts"]}
assert len(acc) == 5, acc.keys()
for a in acc.values():
    assert a["status"] == "ok", (a["label"], a["status"], a["lastError"])
    assert a["windows"], a["label"]
assert s["recommended"]["claude"] == acc["third"]["id"], s["recommended"]
assert s["recommended"]["codex"] == acc["gpt-pro"]["id"], s["recommended"]
assert acc["msds"]["extra"]["limit"] == 50.0
assert acc["gpt-pro"]["plan"] == "Pro"
print("integration OK:", {k: round(v["headroom"]) for k, v in acc.items()})
PY
if grep -E "sign-in required|refresh refused" "$TMP/server.log"; then echo "unexpected re-auth" >&2; exit 1; fi
