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
makes the export-diff purely a `status_changes` query with a secondary join for
extra opened-charger data. The `list_recent_changes` LEFT JOIN workaround can also
be simplified.

### Two export modes

**Diff export** (`export-diff`, on-demand): computed from `status_changes` + DB after
all retries are done. Auto-named `scrape_export_{run_id}.json`. Sequential ordering
enforced via `export_number` integer — prod checks `incoming == MAX(export_number) + 1`.
`--force` bypasses for gap recovery.

**Snapshot export** (`export-snapshot --file`, manual): full current state dump. Used
for initial prod setup or full recovery. Never auto-generated.

### Unchanged chargers not in diff

Prod bulk-updates `last_scraped_at` for all active non-failed chargers using the
`scraped_at` field from the export — no unchanged charger data needed in the file.

---

## Migration

```sql
-- scrape_runs: retry tracking + export anchor + ordering counter
ALTER TABLE scrape_runs
  ADD COLUMN retry_count        INT         NOT NULL DEFAULT 0,
  ADD COLUMN last_retry_at      TIMESTAMPTZ,
  ADD COLUMN export_source_time TIMESTAMPTZ,  -- timestamp anchor for diff cutoff + dedup key
  ADD COLUMN export_number      INT;          -- sequential counter for ordering check

CREATE UNIQUE INDEX ON scrape_runs (export_source_time)
  WHERE export_source_time IS NOT NULL;
CREATE UNIQUE INDEX ON scrape_runs (export_number)
  WHERE export_number IS NOT NULL;

-- site_status: add OPENED for graduation events
ALTER TYPE site_status ADD VALUE 'OPENED';
```

---

## JSON Formats

### Diff export

```json
{
  "type": "diff",
  "version": 1,
  "export_number": 42,
  "exported_at": "2026-04-05T14:23:00Z",
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
  "removed_ids": ["77777"]
}
```

| Field | Purpose |
|---|---|
| `export_number` | Sequential counter; prod rejects if `incoming != MAX(export_number) + 1` |
| `exported_at` | Dedup key (unique index on prod); timestamp anchor for next diff's `status_changes` cutoff on local |
| `scraped_at` | Prod bulk-updates `last_scraped_at` for all active non-failed chargers |
| `changed_chargers` | Full data for chargers in `status_changes` since last export (excl. OPENED) |
| `status_changes` | All `status_changes` rows since last export incl. `OPENED` and `REMOVED` |
| `opened_chargers` | Extra data for graduated chargers (num_stalls, opening_date, etc.) |
| `open_status_failed_ids` | Chargers with `open_status_check_failed = TRUE`; prod sets flag |
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
After snapshot import, a seed `scrape_runs` row is created with `export_source_time =
exported_at` to anchor subsequent diff imports.

---

## Ordering Enforcement (diff imports only)

1. `SELECT COALESCE(MAX(export_number), 0) FROM scrape_runs WHERE export_number IS NOT NULL` → `last_number`
2. If `incoming.export_number != last_number + 1` → reject: "expected export_number {n}, got {m}. Use --force to override."
3. `--force` bypasses for gap recovery (warns user)
4. Snapshot imports are exempt — they reset the anchor; the next diff starts at `last_number + 1`

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
| `src/repository/supercharger.rs` | Add `get_changed_chargers_since`, `get_all_chargers`, `get_all_opened`, `save_chargers_from_diff`; insert OPENED status_change on graduation; simplify `list_recent_changes` LEFT JOIN |
| `src/repository/scrape_run.rs` | Add `get_last_run_id`, `update_retry`, `get_last_export_anchor`, `find_by_export_source_time`, `record_import_run` |
| `src/api/mod.rs` | Add `POST /scrapes/import` route |
| `src/domain/coming_soon.rs` | Add `OPENED` variant to `SiteStatus` enum |

---

## Implementation Detail

### `export-diff` build logic

```
1. parallel: get_last_export_anchor() → previous_export_at (timestamp cutoff)
             get_last_run_stats()     → run_id, scraped_at, country
             COALESCE(MAX(export_number), 0) + 1 → next export_number
2. SELECT * FROM status_changes WHERE changed_at > previous_export_at
   → status_changes list; extract removed_ids (new_status = 'REMOVED')
3. SELECT cs.* FROM coming_soon_superchargers cs
   WHERE cs.id IN (SELECT DISTINCT supercharger_id FROM status_changes
                   WHERE changed_at > previous_export_at
                     AND new_status != 'OPENED')
   → changed_chargers
4. SELECT os.* FROM opened_superchargers os
   WHERE os.id IN (SELECT supercharger_id FROM status_changes
                   WHERE changed_at > previous_export_at
                     AND new_status = 'OPENED')
   → opened_chargers
5. SELECT id FROM coming_soon_superchargers WHERE open_status_check_failed = TRUE
   → open_status_failed_ids
6. Write scrape_export_{run_id}.json (atomic tmp→rename)
7. UPDATE scrape_runs SET export_source_time = exported_at, export_number = N WHERE id = run_id
```

### `save_chargers_from_diff` (prod import transaction)

```
1. UPSERT changed_chargers into coming_soon_superchargers
2. INSERT status_changes (with run_id)
3. For each opened_charger:
     INSERT INTO opened_superchargers ... ON CONFLICT DO NOTHING
     DELETE FROM coming_soon_superchargers WHERE id = $id
4. UPDATE coming_soon SET status = 'REMOVED' WHERE id = ANY($removed_ids)
5. UPDATE coming_soon SET open_status_check_failed = TRUE
   WHERE id = ANY($open_status_failed_ids)
6. UPDATE coming_soon SET last_scraped_at = $scraped_at
   WHERE status != 'REMOVED' AND id != ALL($open_status_failed_ids)
```

### HTTP import handler (`POST /scrapes/import`)

- Auth: `X-Import-Token` header vs `IMPORT_TOKEN` env var; unset → 503, mismatch → 401
- `?force=true` bypasses ordering check
- Responses:
  - `200 { "status": "applied", "scrape_run_id": 42, "changed": 3, "opened": 1, "removed": 0 }`
  - `200 { "status": "duplicate", "scrape_run_id": 39 }`
  - `409 { "status": "out_of_order", "expected": 43, "got": 45 }`
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

# Gap recovery
cargo run -- import --file scrape_export_44.json --force
```

---

## Verification

1. `cargo run -- export-snapshot --file /tmp/snap.json` → valid JSON, both tables present
2. `cargo run -- import --file /tmp/snap.json` on empty DB → chargers upserted, seed run created
3. `cargo run -- scrape && cargo run -- export-diff` → `export_number = 1`, file written
4. `cargo run -- import --file scrape_export_1.json` → changes applied, `last_scraped_at` bulk-updated
5. Re-import without `--force` → "already imported" message, no DB changes
6. Import `export_number = 3` when last is 1 → rejected with expected 2, got 3
7. Graduate a charger locally → `status_changes` has `new_status = 'OPENED'` row
8. HTTP `POST /scrapes/import` wrong token → 401; correct → 200; `?force=true` bypasses ordering
9. `cargo test --verbose` passes
