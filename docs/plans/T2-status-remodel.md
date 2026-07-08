# T2 — Re-model status around `project_status`

**PR 2 of 3 · Risk: high · Depends on: T1 · Frontend coordination: REQUIRED (ships with FE)**
Full design + rationale: [`planned-charger-details.md`](planned-charger-details.md) (D0, D8)

## Goal

Make `project_status` **the** status, in a single ALL-CAPS vocabulary stored as **TEXT** (no Postgres
enum), derived on scan. This replaces the `customer_facing`-derived `IN_DEVELOPMENT` /
`UNDER_CONSTRUCTION` with the finer `PRELIMINARY` / `DESIGN` / `CONSTRUCTION`.

**This is a breaking API change** (status values change) — must deploy together with the frontend PR.

## Status vocabulary (canonical = ALL CAPS)

`PRELIMINARY`, `DESIGN`, `CONSTRUCTION`, `OPENED`, `UNKNOWN`, `REMOVED` — exactly what's stored in
`status` and returned by the API. `OPENED` stays owned by the existing graduation/open-detection flow;
coming-soon rows are only ever `PRELIMINARY`/`DESIGN`/`CONSTRUCTION`/`UNKNOWN`.

(Two casings coexist by design: derived `status` = ALL CAPS; raw Tesla `project_status` = title case,
stored in `raw_project_status` from T1 and validated by `KNOWN_PROJECT_STATUS`.)

## Derivation on scan

```
project_status →
  "Preliminary"   => PRELIMINARY
  "Design"        => DESIGN
  "Construction"  => CONSTRUCTION
  "Open"          => fall back to customer_facing   // don't pre-open; open-detection owns OPENED
  missing/unknown => fall back to customer_facing    // + warn-on-unknown (T1 infra)

customer_facing fallback →
  "In Development"     => PRELIMINARY
  "Under Construction" => CONSTRUCTION
  else                 => UNKNOWN
```

## D8 — status regression policy

`derive_status` produces a **candidate**; `resolve_status_transition(existing, candidate, source)`
applies these before `compute_sync` compares old vs new (domain logic, **not** SQL CASE). `source`
must tell whether the candidate came from `project_status` or the `customer_facing` fallback.

| Situation | Action |
|---|---|
| Forward move (`PRELIMINARY→DESIGN`, `DESIGN→CONSTRUCTION`, …) | **Take it** |
| Derivation yields `UNKNOWN` | **Keep existing** — unknown ≠ backward |
| `project_status` missing; fallback would regress a finer stored status (e.g. stored `DESIGN`, fallback `PRELIMINARY`) | **Keep existing** — coarse label covers both; not a regression signal |
| Fallback upgrades (stored `DESIGN`, `customer_facing="Under Construction"`) | **Take `CONSTRUCTION`** — coarse fallback may move forward |
| Explicit `project_status` backward (`CONSTRUCTION→DESIGN`, …) | **Record it + `warn!`** — may be real; don't silently ratchet |

One-liner: *take every forward transition; record explicit backward `project_status` (+ warn); never
regress on `UNKNOWN` or on `customer_facing` fallback alone when stored status is already finer.*

## Scope / tasks

### Domain
- **`src/domain/coming_soon.rs`** — rework `SiteStatus`:
  - Variants → `Preliminary, Design, Construction, Unknown, Removed, Opened`
    (`rename_all = "SCREAMING_SNAKE_CASE"` keeps DB/API values ALL CAPS, e.g. `PRELIMINARY`).
  - **Drop the `#[sqlx(type_name = "site_status", …)]` derive** — column is now TEXT. Map via
    `Display` / `FromStr` (encode/decode as `String`).
  - Add `derive_status(project_status, customer_facing) -> SiteStatus` and
    `resolve_status_transition(existing, candidate, source) -> SiteStatus`.
  - Update `from_location` / `with_details` to set `status` via `derive_status`.
- **`src/domain/sync.rs`** — apply `resolve_status_transition` in `compute_sync` before comparing
  old vs new status. Update the `charger(...)` test helper for the new variants.

### Migration (enum → TEXT, remap live data + history)
`migrations/<ts>_status_to_text.sql`:
```sql
ALTER TABLE coming_soon_superchargers ALTER COLUMN status DROP DEFAULT;
ALTER TABLE coming_soon_superchargers
    ALTER COLUMN status TYPE TEXT USING (
        CASE status::text
            WHEN 'IN_DEVELOPMENT'     THEN 'PRELIMINARY'
            WHEN 'UNDER_CONSTRUCTION' THEN 'CONSTRUCTION'
            ELSE status::text END);          -- UNKNOWN / REMOVED / OPENED unchanged
ALTER TABLE coming_soon_superchargers ALTER COLUMN status SET DEFAULT 'UNKNOWN';

ALTER TABLE status_changes
    ALTER COLUMN old_status TYPE TEXT USING (
        CASE old_status::text WHEN 'IN_DEVELOPMENT' THEN 'PRELIMINARY'
             WHEN 'UNDER_CONSTRUCTION' THEN 'CONSTRUCTION' ELSE old_status::text END),
    ALTER COLUMN new_status TYPE TEXT USING (
        CASE new_status::text WHEN 'IN_DEVELOPMENT' THEN 'PRELIMINARY'
             WHEN 'UNDER_CONSTRUCTION' THEN 'CONSTRUCTION' ELSE new_status::text END);

DROP TYPE site_status;
```
Notes: `old_status` nullable — `NULL::text` passes through as `NULL`. The `status` index rebuilds
automatically. `charger_category` stays an enum (untouched). All transactional. **History is rewritten
uniformly** `IN_DEVELOPMENT → PRELIMINARY` (lossy — accepted; the Preliminary/Design split never
existed historically).

### API
This is where **all** the API/contract changes land (T1 left the API untouched), so the frontend has a
single coordinated change to absorb.

- **Status re-model:**
  - **`src/repository/supercharger.rs`** — remove all `::site_status` casts and `status::text` shims
    that exist only because of the enum; status is plain TEXT now. The `status` value is **passed
    through directly — no remapping**.
  - Status **filter** param values and the stat buckets in `get_db_stats` /
    `count_coming_soon_by_status` → `PRELIMINARY` / `DESIGN` / `CONSTRUCTION`.
  - **`src/repository/models.rs`** `DbStats` — rename `in_development` / `under_construction` fields to
    `preliminary` / `design` / `construction` (and update `src/application/status.rs` display).
- **Expose the T1 additive fields** (deferred from T1): add `num_charger_stalls`,
  `charging_accessibility`, `street_address`, `county`, `postal_code`, `country_code`,
  `raw_project_status` to `ApiSupercharger` (`src/repository/models.rs`) and to the `SELECT` lists in
  `list_coming_soon` (both branches) + `get_coming_soon`; map them in the row→model closures.
- **`docs/API.md`** — document the new `status` values (ALL CAPS, replacing the old ones) **and** the
  new response fields (note `num_charger_stalls: 0` = unknown).

### Import / export
- `ExportChangedCharger.status` is `SiteStatus` — the renamed enum serialises to the new values.
  Local↔prod deploy together, so no cross-version mismatch; note that **export files produced by old
  code won't deserialize** (transient, acceptable).

## Acceptance criteria
- `status` is TEXT; DB + API return ALL-CAPS values (`PRELIMINARY` etc.).
- `derive_status` + D8 `resolve_status_transition` behave per the tables above.
- Migration remaps existing rows + history; `DROP TYPE site_status` succeeds.
- First scrape after deploy produces the expected one-time `PRELIMINARY → DESIGN` burst for Design-stage
  chargers (accepted — D0 sub-decision (a)).
- No `::site_status` references remain; `cargo build` + `clippy` clean.

## Tests
- `derive_status`: each `project_status` value + fallback paths + unknown (warns).
- `resolve_status_transition`: one case per D8 row (forward, UNKNOWN-keep, fallback-keep, fallback-upgrade, explicit-backward-warn).
- Migration remap: seed old values → assert new values in both tables (incl. NULL `old_status`).

## Deploy
Ship with the frontend PR. After deploy: migrations auto-run → **one local scrape** (populates derived
status + triggers the burst) → `export-snapshot` → `apply_snapshot` on prod → resume diffs.

## Out of scope
- The additive detail fields (**T1**, prerequisite).
- `installed_full_power` (**T3**).
- Suppressing the day-one burst (decided against — (a) accept).
