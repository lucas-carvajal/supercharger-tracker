---
name: verify-tesla-superchargers
description: Drive tesla-superchargers the way an operator does. Isolated Postgres, the host HTTP API, and CLI subcommands. Use when proving host, status, import, or export behavior, or before calling a CLI or API change done.
---

# Verify tesla-superchargers

This skill drives the real `tesla-superchargers` binary. The primary surface is the `host` HTTP API. The CLI (`status`, `export-diff`, `export-snapshot`) is the other operator surface. There is no web UI in this repo.

Do not run `scrape` or `retry-failed` during verification. Those commands launch Chrome and call Tesla's Find Us API. Seed data from `fixtures/` instead.

Read `features/README.md` before you drive. Use the matching feature file as the recipe.

## Launch

Build once, then start one isolated `host` process per run.

Prerequisites:

- Rust stable 1.85 or newer (edition 2024)
- `pkg-config` and OpenSSL headers (`libssl-dev` on Ubuntu)
- Postgres 13+ with `CREATE DATABASE` for the current OS user, or `VERIFY_PG_ADMIN_URL`
- `psql` and `curl` on `PATH`

From the repo root:

```bash
.cursor/skills/verify-tesla-superchargers/scripts/launch.sh demo1
```

`launch.sh` creates database `tesla_superchargers_verify_demo1`, binds a free port on `127.0.0.1`, starts `target/debug/tesla-superchargers host`, and imports `fixtures/snapshot.json`. Ready means `GET /health` returns `{"status":"ok"}` and the seed response is `{"status":"snapshot_applied",...}`.

Empty database, no fixture:

```bash
.cursor/skills/verify-tesla-superchargers/scripts/launch.sh --empty demo1
```

State lives at `/tmp/tesla-superchargers-verify/<run_id>/state.json`. Two runs can share a machine when they use different `run_id` values. Never drive a `host` this run did not start.

Tear down with `scripts/cleanup.sh <run_id>`.

## Doctor

Run this first whenever anything looks off.

```bash
.cursor/skills/verify-tesla-superchargers/scripts/doctor.sh demo1
```

Doctor is read-only. It checks that the recorded pid is alive and is `tesla-superchargers`, that `GET /health` is `{"status":"ok"}`, that the recorded port is listening, and that `/superchargers/soon/stats` matches the launch mode (2 active chargers when seeded, 0 when `--empty`).

Refuse to drive when doctor fails. Relaunch or clean up that run instead of talking to a shared or leftover instance.

## Drive

Use the helpers. Do not invent a second `DATABASE_URL` or port.

```bash
.cursor/skills/verify-tesla-superchargers/scripts/http.sh demo1 GET /superchargers/soon
.cursor/skills/verify-tesla-superchargers/scripts/http.sh demo1 GET /admin/import/current-version --admin
.cursor/skills/verify-tesla-superchargers/scripts/http.sh demo1 POST /admin/import/scrapes --admin --file .cursor/skills/verify-tesla-superchargers/fixtures/diff.json
.cursor/skills/verify-tesla-superchargers/scripts/cli.sh demo1 -- status
.cursor/skills/verify-tesla-superchargers/scripts/cli.sh demo1 -- export-snapshot --file /tmp/out.json
```

Stable handles:

- Routes from `docs/API.md` and `src/api/mod.rs`
- Charger id `11255` (Highbridge, United Kingdom, `DESIGN`) and `12001` (Austin, TX, `CONSTRUCTION`) from `fixtures/snapshot.json`
- Opened charger id `99999` exists only in `opened_superchargers`. `GET /superchargers/soon/99999` is 404
- Admin header `X-Admin-Internal-Secret`, value from `state.json` (`verify-internal-secret` unless `VERIFY_IMPORT_SECRET` was set)
- CLI lines from `src/application/status.rs` (`Last run #1 (full)`, `Active chargers: 2`)

Treat every command in a feature file as literal.

## Evidence

Write proof under `.cursor/skills/verify-tesla-superchargers/evidence/<run_id>/`. Cleanup must not delete that directory.

Proof standards:

- Exercise the real binary path. Do not write rows with `psql` to fake a user action.
- Capture the command and the resulting body, stdout, or file. A final screenshot is not this app.
- For mutations, read the value back through a second user path (list, detail, `status`, or a fresh export).
- `scrape` and `retry-failed` are out of scope. If a change only exists on those paths, say so and stop. Do not open Chrome against Tesla to paper over the gap.

HTTP proof is the request, status code, and JSON body. CLI proof is the command, exit code, stdout, and stderr. Export proof is the written JSON plus a second read (`jq` or a later import).

## Cleanup

```bash
.cursor/skills/verify-tesla-superchargers/scripts/cleanup.sh demo1
```

Cleanup sends `TERM` then `KILL` to the pid in `state.json` only. It drops that run's database. It deletes `/tmp/tesla-superchargers-verify/<run_id>/`. It leaves `evidence/<run_id>/` in place.

Never `pkill tesla-superchargers` or drop a database you did not create.

After a failed launch or drive, run cleanup for that `run_id` before you retry.

## Helpers

Scripts live in `.cursor/skills/verify-tesla-superchargers/scripts/` and are executable.

| Command | Role |
|---|---|
| `scripts/launch.sh [--empty] [run_id]` | Build if needed, create the database, start `host`, optionally seed |
| `scripts/doctor.sh <run_id>` | Read-only health of that run |
| `scripts/http.sh <run_id> GET or POST <path> [--admin] [--file json] [--out dest]` | curl against that run |
| `scripts/cli.sh <run_id> -- <subcommand...>` | CLI with that run's `DATABASE_URL` |
| `scripts/cleanup.sh <run_id>` | Kill that pid, drop that database, keep evidence |

Optional env:

- `VERIFY_PG_ADMIN_URL` when the current user cannot `psql -d postgres`
- `VERIFY_DATABASE_URL_TEMPLATE` with `{name}` if the app cannot use the unix socket URL
- `VERIFY_IMPORT_SECRET` to override the admin secret
- `VERIFY_BINARY` to point at a non-default binary
- `VERIFY_STATE_ROOT` to move `/tmp/tesla-superchargers-verify`
