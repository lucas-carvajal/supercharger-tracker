#!/usr/bin/env bash

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

RUN_ID="$(require_run_id "${1:-}")"
FILE="$(state_file "$RUN_ID")"
DIR="$(state_dir "$RUN_ID")"

if [[ ! -f "$FILE" ]]; then
  echo "verify-tesla-superchargers: no state for $RUN_ID, nothing to clean"
  exit 0
fi

load_state "$RUN_ID"

if pid_alive "$VERIFY_PID"; then
  kill -TERM "$VERIFY_PID" 2>/dev/null || true
  for _ in $(seq 1 20); do
    pid_alive "$VERIFY_PID" || break
    sleep 0.1
  done
  if pid_alive "$VERIFY_PID"; then
    kill -KILL "$VERIFY_PID" 2>/dev/null || true
  fi
fi

if pid_alive "$VERIFY_PID"; then
  die "pid $VERIFY_PID is still alive after TERM and KILL"
fi

drop_database "$VERIFY_ADMIN_DATABASE_URL" "$VERIFY_DATABASE_NAME"

rm -rf "$DIR"
echo "verify-tesla-superchargers: cleaned run $RUN_ID"
echo "evidence remains at $(evidence_dir "$RUN_ID")"
