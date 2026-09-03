# Export

`export-snapshot` writes the full database. `export-diff` writes the latest scrape run. Both files are the bodies that `POST /admin/import/scrapes` accepts.

## Sub-features

- `export-snapshot` writes a `type: snapshot` file with all four tables.
- `export-diff` writes a `type: diff` file for the latest run.
- `export-diff-force` is required when the latest run still has detail or open-status failures.
- `export-diff-empty` fails when no scrape runs exist.

## How to get to it (user POV)

- Run `tesla-superchargers export-snapshot --file <path>`.
- Run `tesla-superchargers export-diff` or `tesla-superchargers export-diff --file <path>`.
- Add `--force` when `status` still reports failures and you still want the diff.

## Driving it with verify-tesla-superchargers

Preconditions:

- Seeded launch for `<run_id>` unless a case says `--empty`.
- `scripts/doctor.sh <run_id>` is green.

- **Snapshot.** Run `scripts/cli.sh <run_id> -- export-snapshot --file evidence/<run_id>/export/snapshot.json`. Exit code `0`. Stdout mentions `1 scrape_runs`, `2 coming-soon`, `1 opened`. The file has `"type": "snapshot"` and charger ids `11255`, `12001`, and opened id `99999`.
- **Diff.** Run `scripts/cli.sh <run_id> -- export-diff --file evidence/<run_id>/export/diff.json`. Exit code `0`. The file has `"type": "diff"` and `"run_id": 1`. `changed_chargers` include `11255` and `12001`. `opened_chargers` include `99999`.
- **Empty diff.** Launch `--empty`, then run `scripts/cli.sh <run_id> -- export-diff --file evidence/<run_id>/export/empty-diff.json`. Non-zero exit. Stderr or stdout mentions `No scrape runs found`.
- **Round trip.** Export a snapshot, launch a second `--empty` run, and post the file to that run's `/admin/import/scrapes`. `GET /superchargers/soon` on the second run has `total` `2`.
- **Proof.** Keep `evidence/<run_id>/export/snapshot.json` and `diff.json`. Both must parse as JSON and include `11255`.

## Gotchas

- Default `export-diff` path is `scrape_export_{id}.json` in the current working directory. Always pass `--file` under `evidence/<run_id>/`.
- `export-diff` refuses a run with `details_failures` or `open_status_failures` unless you pass `--force`. The fixture run has zeros, so `--force` is not required here.
- Snapshot export is a read. It does not change API data.
- Atomic write uses a `.json.tmp` neighbor, then rename. A leftover tmp file means the command died mid-write.
