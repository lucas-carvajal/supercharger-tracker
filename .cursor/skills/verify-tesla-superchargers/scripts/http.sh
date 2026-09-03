#!/usr/bin/env bash
# HTTP client bound to this run's host.
# usage: http.sh <run_id> GET|POST <path> [--admin] [--file json] [--out dest] [--] [curl args]

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

RUN_ID="$(require_run_id "${1:-}")"
shift
METHOD="${1:-}"
PATH_Q="${2:-}"
shift 2 || true
[[ "$METHOD" == "GET" || "$METHOD" == "POST" ]] || die "usage: http.sh <run_id> GET|POST <path> [--admin] [--file json] [--out dest]"
[[ -n "$PATH_Q" ]] || die "path is required"

load_state "$RUN_ID"

ADMIN=0
FILE=""
OUT=""
EXTRA=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --admin)
      ADMIN=1
      shift
      ;;
    --file)
      FILE="${2:-}"
      shift 2
      ;;
    --out)
      OUT="${2:-}"
      shift 2
      ;;
    --)
      shift
      EXTRA+=("$@")
      break
      ;;
    *)
      EXTRA+=("$1")
      shift
      ;;
  esac
done

URL="$VERIFY_BASE_URL$PATH_Q"
args=(-sS -X "$METHOD" "$URL")
if [[ "$ADMIN" == "1" ]]; then
  args+=(-H "X-Admin-Internal-Secret: $VERIFY_SECRET")
fi
if [[ -n "$FILE" ]]; then
  args+=(-H "Content-Type: application/json" --data-binary @"$FILE")
fi
if [[ -n "$OUT" ]]; then
  mkdir -p "$(dirname "$OUT")"
  args+=(-o "$OUT" -w "%{http_code}")
fi
args+=("${EXTRA[@]}")

curl "${args[@]}"
if [[ -n "$OUT" ]]; then
  echo
fi
