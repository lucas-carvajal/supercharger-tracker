# CLI status

`status` prints the latest scrape run and a count of active coming-soon chargers by pipeline stage.

## Sub-features

- `status-empty` reports no runs and zero active chargers.
- `status-seeded` reports run `#1` of type `full` and the fixture counts.
- `status-after-import` reflects a new diff after `POST /admin/import/scrapes`.

## How to get to it (user POV)

- Run `tesla-superchargers status` against a database.

## Driving it with verify-tesla-superchargers

Preconditions:

- For `status-empty`, launch with `--empty`. For the other cases, use the default seeded launch.
- `scripts/doctor.sh <run_id>` is green.

- **Empty database.** Launch empty, then print status. Run `scripts/launch.sh --empty <run_id>` and `scripts/cli.sh <run_id> -- status`. Exit code `0`. Stdout contains `No runs recorded yet.` and `Active chargers: 0`.
- **Seeded database.** Launch with the snapshot, then print status. Run `scripts/cli.sh <run_id> -- status`. Exit code `0`. Stdout contains `Last run #1 (full)` and `Active chargers: 2`, plus `Preliminary: 0`, `Design: 1`, and `Construction: 1`.
- **After a later import.** Apply `fixtures/diff.json`, then print status again. Run `scripts/http.sh <run_id> POST /admin/import/scrapes --admin --file .cursor/skills/verify-tesla-superchargers/fixtures/diff.json` and `scripts/cli.sh <run_id> -- status`. Stdout contains `Last run #2` and `Active chargers: 3`.
- **Proof.** Write stdout to `evidence/<run_id>/status/stdout.txt`. The file must include the run line and the active count from the case you drove.

## Gotchas

- `status` reads `DATABASE_URL`. `scripts/cli.sh` sets it from `state.json`. A bare `cargo run -- status` uses `.env` and can point at the wrong database.
- Unknown chargers print only when the count is greater than zero. Do not require an `Unknown` line on the fixture.
- Detail-fetch failure lines also print only when the count is greater than zero.
