#!/usr/bin/env bash
# Start an isolated host process and optional snapshot seed.
# usage: launch.sh [--empty] [run_id]

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

SEED=1
RUN_ID=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --empty)
      SEED=0
      shift
      ;;
    -*)
      die "unknown flag $1"
      ;;
    *)
      RUN_ID="$1"
      shift
      ;;
  esac
done

if [[ -z "$RUN_ID" ]]; then
  RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)-$$"
fi
RUN_ID="$(require_run_id "$RUN_ID")"

if [[ ! -x "$BINARY" ]]; then
  command -v cargo >/dev/null || die "cargo is required to build $BINARY"
  command -v pkg-config >/dev/null || die "pkg-config is required. On Ubuntu install pkg-config and libssl-dev."
  pkg-config --exists openssl || die "OpenSSL headers are missing. On Ubuntu install libssl-dev."
  (cd "$REPO_ROOT" && cargo build)
  [[ -x "$BINARY" ]] || die "cargo build did not produce $BINARY"
fi

if [[ -f "$(state_file "$RUN_ID")" ]]; then
  die "run $RUN_ID already exists at $(state_dir "$RUN_ID"). Run scripts/cleanup.sh $RUN_ID first."
fi

command -v psql >/dev/null || die "psql is required on PATH"
command -v curl >/dev/null || die "curl is required on PATH"

ADMIN_URL="$(pg_admin_url)"
DB_NAME="tesla_superchargers_verify_${RUN_ID//-/_}"
DB_URL="$(app_database_url "$DB_NAME")"
PORT="$(pick_free_port)"
SECRET="$DEFAULT_SECRET"
DIR="$(state_dir "$RUN_ID")"
mkdir -p "$DIR" "$(evidence_dir "$RUN_ID")"

ensure_database "$ADMIN_URL" "$DB_NAME"

HOST_LOG="$DIR/host.log"
export DATABASE_URL="$DB_URL"
export RUST_INTERNAL_IMPORT_SECRET="$SECRET"
export PORT="$PORT"

# Bind is 0.0.0.0 inside the binary. Clients use 127.0.0.1.
"$BINARY" host --port "$PORT" >"$HOST_LOG" 2>&1 &
PID="$!"
echo "$PID" >"$DIR/host.pid"

ready=0
for _ in $(seq 1 40); do
  if ! pid_alive "$PID"; then
    drop_database "$ADMIN_URL" "$DB_NAME" || true
    rm -rf "$DIR"
    die "host process $PID exited before /health. See $HOST_LOG"
  fi
  if curl -fsS "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 0.25
done

if [[ "$ready" != "1" ]]; then
  kill -TERM "$PID" 2>/dev/null || true
  sleep 0.5
  kill -KILL "$PID" 2>/dev/null || true
  drop_database "$ADMIN_URL" "$DB_NAME" || true
  rm -rf "$DIR"
  die "host did not become ready on port $PORT. See $HOST_LOG"
fi

if [[ "$SEED" == "1" ]]; then
  seed_code="$(curl -sS -o "$DIR/seed.json" -w "%{http_code}" -X POST "http://127.0.0.1:$PORT/admin/import/scrapes" \
    -H "X-Admin-Internal-Secret: $SECRET" \
    -H "Content-Type: application/json" \
    --data-binary @"$FIXTURES_DIR/snapshot.json")"
  if [[ "$seed_code" != "200" ]] || ! python3 -c 'import json,sys; body=json.load(open(sys.argv[1])); assert body.get("status")=="snapshot_applied", body' "$DIR/seed.json"; then
    kill -TERM "$PID" 2>/dev/null || true
    sleep 0.5
    kill -KILL "$PID" 2>/dev/null || true
    drop_database "$ADMIN_URL" "$DB_NAME" || true
    die "snapshot seed failed HTTP $seed_code. Body in $DIR/seed.json"
  fi
fi

export VERIFY_RUN_ID="$RUN_ID"
export VERIFY_PID="$PID"
export VERIFY_PORT="$PORT"
export VERIFY_DATABASE_URL="$DB_URL"
export VERIFY_DATABASE_NAME="$DB_NAME"
export VERIFY_SECRET="$SECRET"
export VERIFY_BINARY="$BINARY"
export VERIFY_SEEDED="$SEED"
export VERIFY_BASE_URL="http://127.0.0.1:$PORT"
export VERIFY_ADMIN_DATABASE_URL="$ADMIN_URL"
write_state "$(state_file "$RUN_ID")"

cat "$(state_file "$RUN_ID")"
echo "verify-tesla-superchargers: launched run $RUN_ID"
