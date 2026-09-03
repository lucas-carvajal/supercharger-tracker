#!/usr/bin/env bash

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

RUN_ID="$(require_run_id "${1:-}")"
shift
if [[ "${1:-}" == "--" ]]; then
  shift
fi
[[ $# -gt 0 ]] || die "usage: cli.sh <run_id> [--] <subcommand...>"

load_state "$RUN_ID"
export DATABASE_URL="$VERIFY_DATABASE_URL"
export RUST_INTERNAL_IMPORT_SECRET="$VERIFY_SECRET"
exec "$VERIFY_BINARY" "$@"
