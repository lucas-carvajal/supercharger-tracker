# Plan: Local Scrape → JSON Export → Prod Import

## Context

Scrapes run locally behind a VPN to evade Akamai Bot Manager. A separate production
server hosts the HTTP API but cannot run scrapes. This plan adds a pipeline to transfer
scrape results from local to prod via JSON files: local generates a diff export, uploads
it (via file or HTTP), and prod applies it atomically.

---

## Decisions

### scrape_runs schema: retries update the parent row

Retries are completions of a scrape session, not independent runs. Rather than inserting
a new `scrape_runs` row per retry, `retry-failed` will:
1. Query the latest `scrape_runs.id` → `parent_run_id`
2. Use `parent_run_id` in `save_chargers()` so all `status_changes` are attributed to the parent
3. `UPDATE scrape_runs SET retry_count = retry_count + 1, last_retry_at = NOW(), details_failures = $x, open_status_failures = $y WHERE id = $parent_run_id`

New columns: `retry_count INT NOT NULL DEFAULT 0`, `last_retry_at TIMESTAMPTZ`.

### Graduation added to status_changes

Currently, graduating a charger (confirmed opened) silently moves it from
`coming_soon_superchargers` to `opened_superchargers` with no entry in `status_changes`.
This gap makes export-diff reconstruction harder and causes the history endpoint to use
a workaround LEFT JOIN.

Fix: add `OPENED` to the `site_status` Postgres enum. In `save_chargers()`, before
deleting a charger, insert a `status_changes` row with `new_status = 'OPENED'`. This
makes the export-diff a pure `status_changes` query with a secondary join for extra
opened-charger data. The `list_recent_changes` LEFT JOIN workaround can also be
simplified.

### Two export modes

**Diff export** (`export-diff`, on-demand): computed from `status_changes` after all
retries are done. Auto-named `scrape_export_{run_id}.json`. No ordering enforcement —
dedup only (prod checks `source_run_id` hasn't been imported before). `--force` skips
dedup for re-import.

**Snapshot export** (`export-snapshot --file`, manual): full current state dump. Used
for initial prod setup or full recovery. Never auto-generated.

### Unchanged chargers not in diff

Prod bulk-updates `last_scraped_at` for all active non-failed chargers using the
`scraped_at` field from the export — no unchanged charger data needed in the file.

### Failure flag lists represent current state

Both `open_status_failed_ids` and `details_fetch_failed_ids` in the export represent
the **current** set of failing chargers. On import, prod replaces the flag entirely:
```sql
UPDATE coming_soon SET details_fetch_failed    = (id = ANY($details_fetch_failed_ids))  WHERE status != 'REMOVED';
UPDATE coming_soon SET open_status_check_failed = (id = ANY($open_status_failed_ids))   WHERE status != 'REMOVED';
```
This correctly clears flags for chargers that were resolved by a retry without a status
change (which would otherwise never appear in `changed_chargers`).

---

## Migration

```sql
ALTER TABLE scrape_runs
  ADD COLUMN retry_count   INT     NOT NULL DEFAULT 0,
  ADD COLUMN last_retry_at TIMESTAMPTZ,
  ADD COLUMN exported      BOOLEAN NOT NULL DEFAULT FALSE,  -- local: was this run exported?
  ADD COLUMN source_run_id BIGINT;                          -- prod: local run_id this was imported from

ALTER TYPE site_status ADD VALUE 'OPENED';
```

---

## JSON Formats

### Diff export

```json
{
  "type": "diff",
  "version": 1,
  "run_id": 42,
  "scraped_at": "2026-04-05T13:55:00Z",
  "country": "US",
  "changed_chargers": [
    {
      "id": "11255", "title": "Austin, Texas", "city": "Austin", "region": "Texas",
      "latitude": 30.267, "longitude": -97.743,
      "status": "IN_DEVELOPMENT", "raw_status_value": "In Development",
      "charger_category": "COMING_SOON",
      "details_fetch_failed": false, "open_status_check_failed": false
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
  "open_status_failed_ids": ["88888"],
  "details_fetch_failed_ids": ["77776"],
  "removed_ids": ["77777"]
}
```

| Field | Purpose |
|---|---|
| `run_id` | Local `scrape_runs.id`; dedup key on prod (`source_run_id`); basis for filename |
| `scraped_at` | Prod bulk-updates `last_scraped_at` for all active non-failed chargers |
| `changed_chargers` | Full charger records for upsert — chargers with entries in `status_changes` (excl. OPENED) |
| `status_changes` | All `status_changes` rows for this run incl. `OPENED` and `REMOVED` |
| `opened_chargers` | Extra data for graduated chargers (num_stalls, opening_date, etc.) |
| `open_status_failed_ids` | Current complete set — prod replaces flag wholesale |
| `details_fetch_failed_ids` | Current complete set — prod replaces flag wholesale |
| `removed_ids` | Derived from `status_changes` where `new_status = 'REMOVED'` |

### Snapshot export

```json
{
  "type": "snapshot",
  "version": 1,
  "exported_at": "2026-04-05T09:00:00Z",
  "coming_soon_superchargers": [ { "...all fields..." } ],
  "opened_superchargers": [ { "...all fields..." } ]
}
```

Import modes: upsert (default, safe) or `--replace` (TRUNCATE + INSERT for full reset).

---

## Files

### New
| File | Purpose |
|---|---|
| `src/export.rs` | `ScrapeExport` enum, `DiffExport`, `SnapshotExport`, `ExportChangedCharger`, `ExportOpenedCharger`, `ExportStatusChange` types |
| `src/application/export_diff.rs` | `run_export_diff(repos, file)` — queries DB, builds diff, writes atomically |
| `src/application/export_snapshot.rs` | `run_export_snapshot(repos, file)` — full state dump |
| `src/application/import.rs` | `run_import(repos, path, force)` — handles both types |
| `src/api/import.rs` | `POST /scrapes/import` HTTP handler |
| `migrations/20260405000000_export_pipeline.sql` | All schema changes above |

### Modified
| File | Change |
|---|---|
| `src/main.rs` | Add `ExportDiff`, `ExportSnapshot`, `Import` subcommands; `mod export;` |
| `src/application/mod.rs` | `pub mod export_diff, export_snapshot, import` |
| `src/application/retry.rs` | Query latest run_id, UPDATE parent row instead of INSERT new row |
| `src/repository/supercharger.rs` | Add `get_changed_chargers_for_run`, `get_all_chargers`, `get_all_opened`, `save_chargers_from_diff`; insert OPENED status_change on graduation; simplify `list_recent_changes` LEFT JOIN |
| `src/repository/scrape_run.rs` | Add `get_last_run_id`, `update_retry`, `mark_exported`, `get_last_exported_run`, `find_by_source_run_id`, `record_import_run` |
| `src/api/mod.rs` | Add `POST /scrapes/import` route |
| `src/domain/coming_soon.rs` | Add `OPENED` variant to `SiteStatus` enum |

---

## Implementation Detail

### `export-diff` build logic

```
1. SELECT id, scraped_at, country FROM scrape_runs WHERE exported = TRUE
   ORDER BY id DESC LIMIT 1 → last_exported_run (or none if first export)
2. SELECT * FROM status_changes
   WHERE scrape_run_id > last_exported_run.id   (or all, if first export)
   → status_changes; extract removed_ids (new_status = 'REMOVED')
3. SELECT cs.* FROM coming_soon_superchargers cs
   WHERE cs.id IN (SELECT DISTINCT supercharger_id FROM status_changes
                   WHERE scrape_run_id > last_exported_run.id
                     AND new_status != 'OPENED')
   → changed_chargers
4. SELECT os.* FROM opened_superchargers os
   WHERE os.id IN (SELECT supercharger_id FROM status_changes
                   WHERE scrape_run_id > last_exported_run.id
                     AND new_status = 'OPENED')
   → opened_chargers
5. SELECT id FROM coming_soon_superchargers WHERE open_status_check_failed = TRUE
   → open_status_failed_ids
6. SELECT id FROM coming_soon_superchargers WHERE details_fetch_failed = TRUE
   → details_fetch_failed_ids
7. Write scrape_export_{run_id}.json (atomic tmp→rename)
8. UPDATE scrape_runs SET exported = TRUE WHERE id = $current_run_id
```

### `save_chargers_from_diff` (prod import transaction)

```
1. UPSERT changed_chargers into coming_soon_superchargers
2. INSERT status_changes (with prod run_id)
3. For each opened_charger:
     INSERT INTO opened_superchargers ... ON CONFLICT DO NOTHING
     DELETE FROM coming_soon_superchargers WHERE id = $id
4. UPDATE coming_soon SET status = 'REMOVED' WHERE id = ANY($removed_ids)
5. UPDATE coming_soon SET details_fetch_failed    = (id = ANY($details_fetch_failed_ids))   WHERE status != 'REMOVED'
6. UPDATE coming_soon SET open_status_check_failed = (id = ANY($open_status_failed_ids))    WHERE status != 'REMOVED'
7. UPDATE coming_soon SET last_scraped_at = $scraped_at
   WHERE status != 'REMOVED'
```

### HTTP import handler (`POST /scrapes/import`)

- Auth: `X-Import-Token` header vs `IMPORT_TOKEN` env var; unset → 503, mismatch → 401
- `?force=true` bypasses dedup check
- Responses:
  - `200 { "status": "applied", "scrape_run_id": 42, "changed": 3, "opened": 1, "removed": 0 }`
  - `200 { "status": "duplicate", "scrape_run_id": 39 }`
  - `400` version mismatch, `401` bad token

---

## Typical Workflow

```bash
# Initial prod setup
cargo run -- export-snapshot --file snapshot.json
cargo run -- import --file snapshot.json   # on prod

# Ongoing
cargo run -- scrape
cargo run -- retry-failed                  # repeat as needed
cargo run -- export-diff                   # writes scrape_export_42.json

cargo run -- import --file scrape_export_42.json        # CLI on prod
# OR
curl -X POST https://prod/scrapes/import \
  -H "X-Import-Token: $SECRET" -H "Content-Type: application/json" \
  -d @scrape_export_42.json
```

---

## Verification

1. `cargo run -- export-snapshot --file /tmp/snap.json` → valid JSON, both tables present
2. `cargo run -- import --file /tmp/snap.json` on empty DB → chargers upserted
3. `cargo run -- scrape && cargo run -- export-diff` → `scrape_export_{id}.json` written, `exported = TRUE` on run
4. `cargo run -- import --file scrape_export_N.json` → changes applied, flags replaced wholesale, `last_scraped_at` bulk-updated
5. Re-import without `--force` → "already imported" message, no DB changes
6. Graduate a charger locally → `status_changes` has `new_status = 'OPENED'` row
7. HTTP `POST /scrapes/import` wrong token → 401; correct → 200
8. `cargo test --verbose` passes
