# Plan: Local Scrape → JSON Export → Prod Import

## Context

Scrapes run locally behind a VPN to evade Akamai Bot Manager. A separate production
server hosts the HTTP API but cannot run scrapes. This plan adds a pipeline to transfer
scrape results from local to prod via JSON files: local generates a diff export, uploads
it (via file or HTTP), and prod applies it atomically.

---

## Key Design Decisions

### scrape_runs: retries update the parent row

Retries complete a scrape session, not start a new one. `retry-failed` will:
1. Query `SELECT id FROM scrape_runs ORDER BY scraped_at DESC LIMIT 1` → `parent_run_id`
2. Pass `parent_run_id` to `save_chargers()` — all `status_changes` attributed to the parent
3. `UPDATE scrape_runs SET retry_count = retry_count + 1, last_retry_at = NOW(), details_failures = $x, open_status_failures = $y WHERE id = $parent_run_id`

### Graduation recorded in status_changes

Currently graduating a charger silently moves it between tables with no `status_changes`
entry — causing a workaround LEFT JOIN in `list_recent_changes`. Fix: add `OPENED` to the
`site_status` enum and insert a `status_changes` row with `new_status = 'OPENED'` before
the delete. Export-diff then finds graduations via the same `status_changes` query as
everything else.

### Export only when scrape is complete

`export-diff` errors if the current run still has unresolved failures:
```
Error: scrape incomplete — 3 chargers have missing details, 2 have pending open-status
checks. Run `retry-failed` first (or use --force to export anyway).
```
Because exports are only generated from clean scrapes, there are no partially-failed
chargers to communicate — `details_fetch_failed_ids` and `open_status_failed_ids` are
not needed in the export format.

### Ordering enforced on import

Prod checks `incoming.run_id == MAX(source_run_id) + 1` — rejects gaps, stale, and
out-of-order imports. The snapshot's `source_run_id` seeds this chain, so no special
NULL exemption is needed. `--force` bypasses for recovery.

### Unchanged chargers not in diff

`last_scraped_at` is bulk-updated on prod for all active chargers using `scraped_at`
from the export — no need to list unchanged chargers in the file.

---

## Migration

```sql
ALTER TABLE scrape_runs
  ADD COLUMN retry_count   INT     NOT NULL DEFAULT 0,
  ADD COLUMN last_retry_at TIMESTAMPTZ,
  ADD COLUMN exported      BOOLEAN NOT NULL DEFAULT FALSE,  -- set TRUE after export-diff
  ADD COLUMN source_run_id BIGINT;                          -- prod: local run_id of import source

-- Drop FK on status_changes.supercharger_id so charger deletion (graduation) doesn't
-- cascade or fail. The init migration has this FK, but CLAUDE.md describes the intended
-- state as no FK ("so history survives charger deletion"). Required for OPENED graduation:
-- we insert the OPENED status_change then delete the charger in the same transaction.
ALTER TABLE status_changes DROP CONSTRAINT status_changes_supercharger_id_fkey;

ALTER TYPE site_status ADD VALUE 'OPENED';
```

---

## JSON Formats

### Diff export

```json
{
  "type": "diff",
  "run_id": 42,
  "scraped_at": "2026-04-05T13:55:00Z",
  "country": "US",
  "changed_chargers": [
    {
      "id": "11255", "title": "Austin, Texas", "city": "Austin", "region": "Texas",
      "latitude": 30.267, "longitude": -97.743,
      "status": "IN_DEVELOPMENT", "raw_status_value": "In Development",
      "charger_category": "COMING_SOON"
    }
  ],
  "status_changes": [
    { "supercharger_id": "11255", "old_status": null, "new_status": "IN_DEVELOPMENT" }
  ],
  "opened_chargers": [
    {
      "id": "99999", "title": "Denver, Colorado", "city": "Denver", "region": "Colorado",
      "latitude": 39.739, "longitude": -104.984,
      "opening_date": "2026-03-15", "num_stalls": 12, "open_to_non_tesla": true
    }
  ],
  "removed_ids": ["77777"]
}
```

| Field | Purpose |
|---|---|
| `run_id` | Local `scrape_runs.id`; ordering + dedup key on prod; basis for filename |
| `scraped_at` | Prod bulk-updates `last_scraped_at` for all active chargers |
| `changed_chargers` | Full charger records for upsert — those with `status_changes` entries (excl. OPENED) |
| `status_changes` | All `status_changes` for this run incl. `OPENED` and `REMOVED` entries |
| `opened_chargers` | Full data for graduated chargers (num_stalls, opening_date, etc.) |
| `removed_ids` | Charger IDs where `new_status = 'REMOVED'` in `status_changes` |

### Snapshot export

```json
{
  "type": "snapshot",
  "source_run_id": 42,
  "coming_soon_superchargers": [ { "...all fields..." } ],
  "opened_superchargers": [ { "...all fields..." } ],
  "status_changes": [
    { "supercharger_id": "11255", "scrape_run_id": 7, "old_status": null, "new_status": "IN_DEVELOPMENT", "changed_at": "2026-03-01T10:00:00Z" }
  ]
}
```

`source_run_id` is the latest local `scrape_runs.id` at snapshot time. On import, prod
creates a seed `scrape_runs` row with `source_run_id = 42`, anchoring the ordering chain.
The first diff must then have `run_id = 43` — no special-case needed.

`scrape_runs` is excluded from the snapshot — both local and prod have their own
auto-increment IDs which would conflict on insert. The `scrape_run_id` values in
`status_changes` reference local IDs that won't exist on prod, but since the FK is
dropped in the migration this is fine — they serve as audit data only.

Import modes: upsert (default) or `--replace` (TRUNCATE + INSERT for full reset).

---

## Files

### New
| File | Purpose |
|---|---|
| `src/export.rs` | `ScrapeExport` enum, `DiffExport`, `SnapshotExport`, `ExportChangedCharger`, `ExportOpenedCharger`, `ExportStatusChange`, `ExportSnapshotStatusChange` types |
| `src/application/export_diff.rs` | `run_export_diff(repos, file, force)` |
| `src/application/export_snapshot.rs` | `run_export_snapshot(repos, file)` |
| `src/application/import.rs` | `run_import(repos, path, force)` |
| `src/api/import.rs` | `POST /scrapes/import` HTTP handler |
| `migrations/20260405000000_export_pipeline.sql` | All schema changes above |

### Modified
| File | Change |
|---|---|
| `src/main.rs` | Add `ExportDiff { file, force }`, `ExportSnapshot { file }`, `Import { file, force }` subcommands; `mod export;` |
| `src/application/mod.rs` | `pub mod export_diff, export_snapshot, import` |
| `src/application/retry.rs` | Query latest run_id, UPDATE parent row instead of INSERT new row |
| `src/repository/supercharger.rs` | Add `get_changed_chargers_for_run`, `get_all_chargers`, `get_all_opened`, `get_all_status_changes`, `save_chargers_from_diff`; insert `OPENED` status_change on graduation; remove LEFT JOIN workaround from `list_recent_changes` |
| `src/repository/scrape_run.rs` | Add `get_last_run_id`, `update_retry`, `mark_exported`, `get_last_exported_run`, `find_by_source_run_id`, `record_import_run` |
| `src/api/mod.rs` | Add `POST /scrapes/import` route |
| `src/domain/coming_soon.rs` | Add `OPENED` variant to `SiteStatus` enum |

---

## Implementation Detail

### `export-diff` logic

```
1. Load current run: SELECT id, scraped_at, country, details_failures, open_status_failures
   FROM scrape_runs ORDER BY scraped_at DESC LIMIT 1
   → if details_failures > 0 OR open_status_failures > 0: error (unless --force)

2. Load last exported run: SELECT id FROM scrape_runs WHERE exported = TRUE
   ORDER BY id DESC LIMIT 1 → last_id (0 if none)

3. SELECT * FROM status_changes WHERE scrape_run_id > last_id
   → status_changes; extract removed_ids (new_status = 'REMOVED')

4. SELECT cs.* FROM coming_soon_superchargers
   WHERE id IN (SELECT DISTINCT supercharger_id FROM status_changes
                WHERE scrape_run_id > last_id AND new_status != 'OPENED')
   → changed_chargers

5. SELECT os.* FROM opened_superchargers
   WHERE id IN (SELECT supercharger_id FROM status_changes
                WHERE scrape_run_id > last_id AND new_status = 'OPENED')
   → opened_chargers

6. Write scrape_export_{run_id}.json (atomic tmp→rename)
7. UPDATE scrape_runs SET exported = TRUE WHERE id = $run_id
```

### `save_chargers_from_diff` (prod import transaction)

```
1. Dedup: reject if source_run_id already exists in scrape_runs
   Ordering: if prior imports exist, reject unless run_id == MAX(source_run_id) + 1
2. UPSERT changed_chargers into coming_soon_superchargers
3. INSERT status_changes (with prod run_id)
4. For each opened_charger:
     INSERT INTO opened_superchargers ... ON CONFLICT DO NOTHING
     DELETE FROM coming_soon_superchargers WHERE id = $id
5. UPDATE coming_soon SET status = 'REMOVED' WHERE id = ANY($removed_ids)
6. UPDATE coming_soon SET last_scraped_at = $scraped_at WHERE status != 'REMOVED'
```

### HTTP import (`POST /scrapes/import`)

- Auth: `X-Import-Token` header vs `IMPORT_TOKEN` env var; unset → 503, mismatch → 401
- `?force=true` bypasses ordering check
- Responses:
  - `200 { "status": "applied", "run_id": 42, "changed": 3, "opened": 1, "removed": 0 }`
  - `200 { "status": "duplicate" }`
  - `409 { "status": "out_of_order", "expected": 43, "got": 41 }`
  - `400` version mismatch, `401` bad token

---

## Typical Workflow

```bash
# Initial prod setup
cargo run -- export-snapshot --file snapshot.json
cargo run -- import --file snapshot.json          # on prod

# Ongoing
cargo run -- scrape
cargo run -- retry-failed                         # repeat until clean
cargo run -- export-diff                          # errors if still incomplete; writes scrape_export_42.json

cargo run -- import --file scrape_export_42.json  # on prod
# OR
curl -X POST https://prod/scrapes/import \
  -H "X-Import-Token: $SECRET" -H "Content-Type: application/json" \
  -d @scrape_export_42.json
```

---

## Verification

1. `export-diff` with pending failures → clear error message
2. `export-diff --force` with pending failures → proceeds, writes file
3. `export-snapshot --file snap.json` → valid JSON; `import --file snap.json` on empty DB → chargers upserted
4. Clean scrape + `export-diff` → `scrape_export_{id}.json` written, `exported = TRUE` set on run
5. `import scrape_export_N.json` → changes applied, `last_scraped_at` bulk-updated, graduation in `status_changes`
6. Re-import same file → "already imported", no DB changes
7. Import with `run_id` != MAX(source_run_id) + 1 → rejected as out-of-order with expected value
8. HTTP wrong token → 401; correct → 200; `?force=true` bypasses ordering
9. `cargo test --verbose` passes
