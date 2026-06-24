# T3 — Capture `installed_full_power` at graduation

**PR 3 of 3 · Risk: low · Depends on: nothing (orthogonal) · Frontend coordination: optional**
Full design + rationale: [`planned-charger-details.md`](planned-charger-details.md) (D11)

## Goal

Capture each Supercharger's installed power (kW) — but **only when it opens**, not on coming-soon rows.
`installed_full_power` is always `"0"` for planned sites (confirmed across the sample) and only carries
a real value (e.g. `"250"`) once a site is live. So we read it from the **open-check** response at
graduation and store it on `opened_superchargers`.

> Independent of T1/T2 — can ship anytime, or fold into T1 if you prefer fewer PRs.

## Scope / tasks

### Migration
`migrations/<ts>_opened_installed_power.sql`:
```sql
ALTER TABLE opened_superchargers
    ADD COLUMN installed_full_power_kw INTEGER;  -- NULL if missing / unparseable / "0"
```
(Pre-existing opened rows stay NULL — we don't re-fetch graduated chargers. Expected.)

### Fetch / domain
- **`src/scraper/raw.rs`** — add `installed_full_power: Option<String>` to
  `OpenCheckSuperchargerFunction`.
- **`src/scraper/loaders.rs`** `fetch_open_status_for_ids` — parse it to `i32`; treat missing,
  unparseable, and `"0"` as `None`. Populate the new field on `OpenResult`.
- **`src/domain/sync.rs`** — add `installed_full_power_kw: Option<i32>` to `OpenResult`.

### Persistence + import/export
- **`src/repository/supercharger.rs`** — graduation `INSERT INTO opened_superchargers` in
  `save_chargers`: bind the new column. Mirror in `save_chargers_from_diff` and `apply_snapshot`
  opened-charger inserts. Add to `get_opened_chargers_for_run` SELECT + `row_to_export_opened`.
- **`src/export.rs`** — add
  `#[serde(default, skip_serializing_if = "Option::is_none")] pub installed_full_power_kw: Option<i32>`
  to `ExportOpenedCharger`.
- **`export-snapshot`** opened read path — select the column.
- **`docs/API.md`** — document the field if opened chargers are exposed via the API.

## Acceptance criteria
- A charger confirmed open at graduation stores its kW (`"250"` → `250`).
- `"0"` / missing / unparseable → `NULL`.
- Snapshot + diff round-trip carries the field.

## Tests
- Open-check parse: `"250"` → `Some(250)`; `"0"` / missing → `None`.

## Out of scope
- Anything on `coming_soon_superchargers` (power is always 0 there).
- Backfilling power for already-opened rows.
