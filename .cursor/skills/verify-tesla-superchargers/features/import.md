# Import scrapes

An operator copies scrape output from one instance to another by posting a snapshot or a sequential diff to `POST /admin/import/scrapes`. The same secret protects `GET /admin/import/current-version`.

## Sub-features

- `import-auth-missing` rejects a missing or wrong `X-Admin-Internal-Secret`.
- `import-version` reports `current_version` and `next_expected_version`.
- `import-snapshot` replaces all four tables.
- `import-diff` applies the next `run_id`.
- `import-duplicate` is a no-op for a `run_id` that already exists.
- `import-out-of-order` is 409 when `run_id` is not `MAX(id) + 1`.

## How to get to it (user POV)

- `POST /admin/import/scrapes` with header `X-Admin-Internal-Secret` and a `ScrapeExport` body from `export-diff` or `export-snapshot`.
- `GET /admin/import/current-version` with the same header.
- Next.js calls these routes for the operator. This repo only exposes the HTTP API.

## Driving it with verify-tesla-superchargers

Preconditions:

- Seeded launch for `<run_id>` unless a case says `--empty`.
- `scripts/doctor.sh <run_id>` is green.

- **Wrong secret.** Run `curl -sS -o evidence/<run_id>/import/unauthorized.json -w "%{http_code}" -X POST "$VERIFY_BASE_URL/admin/import/scrapes" -H "X-Admin-Internal-Secret: wrong" -H "Content-Type: application/json" --data-binary @.cursor/skills/verify-tesla-superchargers/fixtures/diff.json`. HTTP `401`.
- **Current version.** Run `scripts/http.sh <run_id> GET /admin/import/current-version --admin --out evidence/<run_id>/import/current-version.json`. HTTP `200`, `current_version` is `1`, `next_expected_version` is `2`.
- **Apply diff.** Run `scripts/http.sh <run_id> POST /admin/import/scrapes --admin --file .cursor/skills/verify-tesla-superchargers/fixtures/diff.json --out evidence/<run_id>/import/diff.json`. HTTP `200` and `status` is `applied` with `run_id` `2`.
- **Confirm the new site.** Run `scripts/http.sh <run_id> GET /superchargers/soon/13000`. `title` is `Berlin, Germany` and `status` is `PRELIMINARY`.
- **Duplicate.** Post `fixtures/diff.json` again. HTTP `200` and `status` is `duplicate`.
- **Out of order.** On a fresh seeded run, post a body whose `run_id` is `5`. HTTP `409` and `status` is `out_of_order` with `expected` `2` and `got` `5`.
- **Snapshot on empty.** Launch `--empty`, then post `fixtures/snapshot.json`. HTTP `200` and `status` is `snapshot_applied`. `GET /superchargers/soon` then has `total` `2`.
- **Proof.** Keep `diff.json` and the `GET /superchargers/soon/13000` body. The second view must show `13000` after `applied`.

## Gotchas

- Launch already applies the snapshot. Posting the snapshot again still returns `snapshot_applied` and truncates the four tables.
- A diff on an empty database expects `run_id` `1`. Real local run ids are much larger. Apply a snapshot first, then diffs.
- `?force=true` bypasses the ordering check. Only use it when the recipe is testing gap recovery.
- If `RUST_INTERNAL_IMPORT_SECRET` is unset, both admin routes return `503`. `launch.sh` always sets the secret.
- `charger_category` in fixture JSON is the serde name `ComingSoon`, not the SQL enum label `COMING_SOON`.
