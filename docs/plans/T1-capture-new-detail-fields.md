# T1 — Capture new planned-charger detail fields (additive)

**PR 1 of 3 · Risk: low · Depends on: nothing · Frontend coordination: none**
Full design + rationale: [`planned-charger-details.md`](planned-charger-details.md)

## Goal

Capture and persist the extra fields Tesla already returns for coming-soon chargers — **without
touching the status model**. The existing `IN_DEVELOPMENT` / `UNDER_CONSTRUCTION` status keeps working
unchanged; the status re-model is T2. Zero extra HTTP cost (same call, richer `functionTypes`).

New data captured: `raw_project_status`, `num_charger_stalls`, `charging_accessibility`, and a
structured address (`street_address`, `county`, `postal_code`, `country_code`).

## Scope / tasks

### Fetch
- **`src/scraper/loaders.rs`** — in `eval_detail_batch` (~line 644) change the query string:
  `functionTypes=coming_soon_supercharger` → `coming_soon_supercharger,supercharger`.
  (Requesting `supercharger` alone on a planned site returns nothing — both are required.)
- **`src/scraper/raw.rs`** — add `RawSuperchargerFunction`, `RawFunction` (address only —
  `functions[0].status` is deferred, see [D6], do **not** deserialize it), `RawAddress`
  (`address_1`, `county`, `postal_code`, `country`). Extend `ComingSoonDetails` with the new fields.
- **`detail_from_value`** — merge `supercharger_function` + `functions.first()` (address only).
  **Precedence:** stalls / accessibility / project_status come strictly from `supercharger_function`;
  only the address comes from `functions[0]`. If `supercharger_function` is absent → no usable details
  (as today).

### Migration (additive only — no enum change)
`migrations/<ts>_planned_charger_detail_fields.sql`:
```sql
ALTER TABLE coming_soon_superchargers
    ADD COLUMN raw_project_status     TEXT,
    ADD COLUMN num_charger_stalls     INTEGER NOT NULL DEFAULT 0,  -- 0 = unknown
    ADD COLUMN charging_accessibility TEXT,
    ADD COLUMN street_address         TEXT,
    ADD COLUMN county                 TEXT,
    ADD COLUMN postal_code            TEXT,
    ADD COLUMN country_code           TEXT;
```

### Domain
- **`src/domain/coming_soon.rs`** — add the fields to `ComingSoonSupercharger`; populate in
  `from_location` and `with_details`. `num_charger_stalls`: parse to `i32`, **missing/unparseable → 0**
  (`0 = unknown`, see [num_charger_stalls semantics]). `raw_project_status` = raw Tesla string (title
  case, e.g. `"Design"`).

### Warn-on-unknown infra
- **`UnknownEnumTracker`** (`src/scraper/loaders.rs`) modelled on `failure_reasons: HashMap` — dedupe:
  `warn!` the first time each distinct `(field, value)` is seen, count all occurrences. Own one in
  `fetch_batch_details_from_page`, thread `&mut` into the per-batch parse, move it onto `LoadResult`.
- `KNOWN_PROJECT_STATUS = ["Preliminary","Design","Construction","Open"]` and
  `KNOWN_CHARGING_ACCESSIBILITY = ["Tesla Only","All Vehicles (Production)","NACS Partner Enabled (Production)"]`.
  Validate the **raw** values (title case). Raw string is always stored regardless — warning is
  observability only.
- **`src/application/scrape.rs`** — call `tracker.log_summary()` after the "DB update complete" log.

### Persistence
- **`src/repository/supercharger.rs` `save_chargers`** — write the new columns in **both** the upsert
  INSERT (~169–218) and the unchanged UPDATE (~233–257).
- **Rule A guard** (don't overwrite a known value with an "unknown" blank):
  `col = CASE WHEN EXCLUDED.col IS NULL THEN existing.col ELSE EXCLUDED.col END`
  (`= 0` instead of `IS NULL` for `num_charger_stalls`). Compose with the existing
  preserve-on-failure guard. Mirror the same CASE in `save_chargers_from_diff` and `apply_snapshot`.
- Update the row→struct closures in `get_failed_detail_chargers` / `get_failed_open_status_chargers`.

### API — **unchanged in T1**
T1 deliberately does **not** touch the public API. `ApiSupercharger` and the `list_coming_soon` /
`get_coming_soon` SELECTs stay as-is, so API responses are byte-for-byte unchanged and no frontend work
is triggered. The new columns are captured, persisted, and shipped to prod (below), so the data is ready
and backfilled — exposing it via the API happens in **T2**, together with the breaking status change and
the frontend deploy, so there's a single API-contract change.

> Internal reads that build the full `ComingSoonSupercharger` (`get_failed_detail_chargers` /
> `get_failed_open_status_chargers`) still need the new columns in their `SELECT` to populate the struct
> — that's internal, not the public API.

### Import / export
- **`src/export.rs`** — add the fields to `ExportChangedCharger`
  (`#[serde(default, skip_serializing_if = "Option::is_none")]`; `#[serde(default)]` for the
  non-optional `num_charger_stalls`).
- `get_changed_chargers_for_run` SELECT + `row_to_export_changed`; `save_chargers_from_diff` INSERT;
  `apply_snapshot` INSERT; `export-snapshot` read path.

## Acceptance criteria
- After a scrape, the new columns are populated for both changed and **unchanged** rows (backfill works
  on the first run post-deploy).
- `num_charger_stalls` is `0` when Tesla reports `"0"` or omits it; a real count is never overwritten
  by `0` (Rule A).
- An unrecognised `project_status` / `charging_accessibility` value logs a deduped warning + end-of-run
  summary, but is still stored verbatim and does not break ingestion.
- Existing status behaviour (`IN_DEVELOPMENT` / `UNDER_CONSTRUCTION`) is **unchanged**.
- **API responses are unchanged** — no new fields exposed (that's T2).
- Snapshot + diff round-trip carries the new fields (so prod is backfilled before T2 exposes them).

## Tests
- Stall parse: `"8"`→8, `"0"`→0, missing→0.
- Rule A: known value not overwritten by blank/0.
- Warn-on-unknown: unknown value warns once, stored verbatim.
- Repo round-trip: save with new fields, read via `get_coming_soon`.

## Out of scope
- **Exposing the new fields via the public API** — deferred to **T2** so all API/contract changes ship
  together with the frontend.
- Status re-model / `project_status`-derived status (**T2**).
- `installed_full_power` at graduation (**T3**).
- `real_estate_status` (deferred, D6).
- Deriving `city`/`region` from the address (D4 — keep title parsing).
