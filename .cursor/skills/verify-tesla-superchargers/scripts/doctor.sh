#!/usr/bin/env bash

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

RUN_ID="$(require_run_id "${1:-}")"
load_state "$RUN_ID"
DIR="$(state_dir "$RUN_ID")"

fail=0
say() { echo "$*"; }
bad() { echo "FAIL: $*" >&2; fail=1; }

if ! pid_alive "$VERIFY_PID"; then
  bad "pid $VERIFY_PID is not running"
else
  cmd="$(cmdline_of "$VERIFY_PID")"
  say "pid $VERIFY_PID is running: $cmd"
  if [[ "$cmd" != *"$VERIFY_BINARY"* && "$cmd" != *"tesla-superchargers"* ]]; then
    bad "pid $VERIFY_PID is not tesla-superchargers ($cmd)"
  fi
fi

if [[ ! -x "$VERIFY_BINARY" ]]; then
  bad "binary missing: $VERIFY_BINARY"
else
  say "binary $VERIFY_BINARY"
fi

health_file="$DIR/health.json"
health="$(curl -sS -o "$health_file" -w "%{http_code}" "$VERIFY_BASE_URL/health" || true)"
if [[ "$health" != "200" ]]; then
  bad "GET $VERIFY_BASE_URL/health returned HTTP $health"
else
  python3 -c 'import json,sys; body=json.load(open(sys.argv[1])); assert body=={"status":"ok"}, body; print("GET /health", body)' "$health_file" \
    || bad "GET /health body is not {\"status\":\"ok\"}"
fi

if command -v ss >/dev/null; then
  if ss -lntp | grep -q ":$VERIFY_PORT "; then
    say "port $VERIFY_PORT is listening"
  else
    bad "port $VERIFY_PORT is not listening"
  fi
fi

if [[ "$VERIFY_SEEDED" == "1" ]]; then
  stats="$(curl -fsS "$VERIFY_BASE_URL/superchargers/soon/stats" || true)"
  echo "$stats" | python3 -c 'import json,sys
body=json.load(sys.stdin)
assert body.get("total_active")==2, body
assert body.get("by_status",{}).get("DESIGN")==1, body
assert body.get("by_status",{}).get("CONSTRUCTION")==1, body
print("seeded stats", body)' || bad "seeded stats do not match fixtures/snapshot.json"
else
  stats="$(curl -fsS "$VERIFY_BASE_URL/superchargers/soon/stats" || true)"
  echo "$stats" | python3 -c 'import json,sys
body=json.load(sys.stdin)
assert body.get("total_active")==0, body
print("empty stats", body)' || bad "empty instance still has chargers"
fi

if [[ "$fail" != "0" ]]; then
  die "doctor failed for run $RUN_ID"
fi
echo "doctor ok for run $RUN_ID at $VERIFY_BASE_URL"
