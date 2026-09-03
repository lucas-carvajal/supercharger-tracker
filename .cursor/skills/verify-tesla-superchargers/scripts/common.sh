# shellcheck shell=bash

set -euo pipefail

SKILL_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="$(cd "$SKILL_DIR/../../.." && pwd)"
SCRIPTS_DIR="$SKILL_DIR/scripts"
FIXTURES_DIR="$SKILL_DIR/fixtures"
EVIDENCE_ROOT="$SKILL_DIR/evidence"
STATE_ROOT="${VERIFY_STATE_ROOT:-/tmp/tesla-superchargers-verify}"
BINARY="${VERIFY_BINARY:-$REPO_ROOT/target/debug/tesla-superchargers}"
DEFAULT_SECRET="${VERIFY_IMPORT_SECRET:-verify-internal-secret}"

die() {
  echo "verify-tesla-superchargers: $*" >&2
  exit 1
}

require_run_id() {
  local run_id="${1:-}"
  [[ -n "$run_id" ]] || die "usage: $0 <run_id>"
  [[ "$run_id" =~ ^[A-Za-z0-9_-]+$ ]] || die "run_id must be letters, digits, underscore, or hyphen"
  echo "$run_id"
}

state_dir() {
  echo "$STATE_ROOT/$1"
}

state_file() {
  echo "$STATE_ROOT/$1/state.json"
}

evidence_dir() {
  echo "$EVIDENCE_ROOT/$1"
}

load_state() {
  local run_id="$1"
  local file
  file="$(state_file "$run_id")"
  [[ -f "$file" ]] || die "no state for run $run_id at $file. Run scripts/launch.sh $run_id first."
  eval "$(python3 - "$file" <<'PY'
import json, shlex, sys
state = json.load(open(sys.argv[1]))
keys = (
    "run_id",
    "pid",
    "port",
    "database_url",
    "database_name",
    "secret",
    "binary",
    "seeded",
    "base_url",
    "admin_database_url",
)
for key in keys:
    if key not in state:
        raise SystemExit(f"state.json missing {key}")
    value = state[key]
    if isinstance(value, bool):
        value = "1" if value else "0"
    print(f"export VERIFY_{key.upper()}={shlex.quote(str(value))}")
PY
)"
}

write_state() {
  local dest="$1"
  python3 - "$dest" <<'PY'
import json, os, sys
dest = sys.argv[1]
payload = {
    "run_id": os.environ["VERIFY_RUN_ID"],
    "pid": int(os.environ["VERIFY_PID"]),
    "port": int(os.environ["VERIFY_PORT"]),
    "database_url": os.environ["VERIFY_DATABASE_URL"],
    "database_name": os.environ["VERIFY_DATABASE_NAME"],
    "secret": os.environ["VERIFY_SECRET"],
    "binary": os.environ["VERIFY_BINARY"],
    "seeded": os.environ.get("VERIFY_SEEDED", "1") == "1",
    "base_url": os.environ["VERIFY_BASE_URL"],
    "admin_database_url": os.environ["VERIFY_ADMIN_DATABASE_URL"],
}
with open(dest, "w", encoding="utf-8") as fh:
    json.dump(payload, fh, indent=2)
    fh.write("\n")
PY
}

pid_alive() {
  local pid="$1"
  [[ -n "$pid" && "$pid" != "0" ]] || return 1
  kill -0 "$pid" 2>/dev/null
}

cmdline_of() {
  local pid="$1"
  if [[ -r "/proc/$pid/cmdline" ]]; then
    tr '\0' ' ' <"/proc/$pid/cmdline"
    echo
  else
    ps -p "$pid" -o args=
  fi
}

pick_free_port() {
  python3 - <<'PY'
import socket
sock = socket.socket()
sock.bind(("127.0.0.1", 0))
print(sock.getsockname()[1])
sock.close()
PY
}

pg_admin_url() {
  if [[ -n "${VERIFY_PG_ADMIN_URL:-}" ]]; then
    echo "$VERIFY_PG_ADMIN_URL"
    return
  fi
  if psql -d postgres -c "SELECT 1" >/dev/null 2>&1; then
    echo "postgres:///?host=/var/run/postgresql&dbname=postgres"
    return
  fi
  if sudo -n -u postgres psql -d postgres -c "SELECT 1" >/dev/null 2>&1; then
    echo "postgres:///?host=/var/run/postgresql&dbname=postgres"
    return
  fi
  die "cannot reach Postgres. Start a local cluster and grant CREATE DATABASE, or set VERIFY_PG_ADMIN_URL."
}

psql_url() {
  local url="$1"
  shift
  psql "$url" "$@"
}

ensure_database() {
  local admin_url="$1"
  local name="$2"
  psql_url "$admin_url" -v ON_ERROR_STOP=1 -c "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '$name' AND pid <> pg_backend_pid();" >/dev/null
  psql_url "$admin_url" -v ON_ERROR_STOP=1 -c "DROP DATABASE IF EXISTS \"$name\";" >/dev/null
  psql_url "$admin_url" -v ON_ERROR_STOP=1 -c "CREATE DATABASE \"$name\";" >/dev/null
}

drop_database() {
  local admin_url="$1"
  local name="$2"
  psql_url "$admin_url" -v ON_ERROR_STOP=1 -c "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '$name' AND pid <> pg_backend_pid();" >/dev/null || true
  psql_url "$admin_url" -v ON_ERROR_STOP=1 -c "DROP DATABASE IF EXISTS \"$name\";" >/dev/null
}

app_database_url() {
  local name="$1"
  if [[ -n "${VERIFY_DATABASE_URL_TEMPLATE:-}" ]]; then
    echo "${VERIFY_DATABASE_URL_TEMPLATE//\{name\}/$name}"
    return
  fi
  # localhost uses TCP+scram on a stock Ubuntu cluster. The unix socket uses peer auth.
  echo "postgres:///?host=/var/run/postgresql&dbname=$name"
}
